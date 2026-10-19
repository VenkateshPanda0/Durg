//! # A hostile-input ELF64 parser
//!
//! An **ELF** file is how a program is packaged: a header, a table of *program
//! headers* describing which bytes of the file go where in memory and with what
//! permissions, and the bytes themselves. The loader reads one to build a task's
//! address space. Everything in the file is chosen by whoever built it, which
//! for an agent platform means **someone untrusted**, so this parser is written
//! like the system-call validator: pure, no `unsafe` code, no allocation, every
//! number checked before it is used, and anything surprising is *refused*, not
//! worked around.
//!
//! It does not touch memory. It turns a byte slice into an [`Image`]: an entry
//! point and a short list of [`Segment`]s, each already checked against a
//! [`Policy`]. The kernel's loader then maps them. Because the loader trusts only
//! what this crate returns, the properties below are the loader's:
//!
//! - **W^X:** no segment is both writable and executable.
//! - **Range:** every segment lies, page-rounded, inside the address range the
//!   policy allows, so it cannot reach the kernel half, the null page, or the
//!   stack.
//! - **No overlap:** no two segments share a page, so permissions are per page.
//! - **File bounds:** every byte range the loader will copy lies inside the file.
//! - **Entry:** the entry point is inside an executable segment's file-backed
//!   part.
//! - **No surprises:** no dynamic linking, interpreter, thread-local storage,
//!   executable stack, or unknown segment type. A loader that does not implement
//!   a feature must not silently ignore it.
//! - **Bounded:** at most [`MAX_SEGMENTS`] segments and [`Policy::max_total_bytes`]
//!   of memory.
//!
//! The format is read from the System V ABI ELF specification (the 64-bit header
//! is 64 bytes, a program header is 56) and the x86-64 psABI.

#![no_std]
#![forbid(unsafe_code)]

/// Page size the loader maps in.
pub const PAGE_SIZE: u64 = 4096;
/// Most loadable segments an image may have.
pub const MAX_SEGMENTS: usize = 8;

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
/// Most program headers read, whatever the file claims.
const MAX_PROGRAM_HEADERS: u64 = 64;

const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 0x3e;

const PT_NULL: u32 = 0;
const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
const PT_PHDR: u32 = 6;
const PT_GNU_EH_FRAME: u32 = 0x6474_e550;
const PT_GNU_STACK: u32 = 0x6474_e551;
const PT_GNU_RELRO: u32 = 0x6474_e552;
const PT_GNU_PROPERTY: u32 = 0x6474_e553;

const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

/// What a loader will accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Lowest address a segment may start at (keeps the null page unmapped).
    pub min_address: u64,
    /// One past the highest address a segment may reach, after rounding up to a
    /// page. Set below the user stack so a segment cannot cover it.
    pub max_address: u64,
    /// Most memory (`memsz`, page-rounded, summed) the whole image may occupy.
    pub max_total_bytes: u64,
}

/// Permissions of a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Perms {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

/// One loadable piece of the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// First address. Always page-aligned.
    pub vaddr: u64,
    /// Size in memory. Bytes past `filesz` are zero. Never zero.
    pub memsz: u64,
    /// Where its bytes start in the file.
    pub file_offset: u64,
    /// How many bytes come from the file. Never more than `memsz`, and the whole
    /// range lies inside the file.
    pub filesz: u64,
    pub perms: Perms,
}

impl Segment {
    /// One past the last byte in memory.
    pub fn end(&self) -> u64 {
        self.vaddr + self.memsz
    }

    /// Pages covered, rounded up.
    pub fn pages(&self) -> u64 {
        self.memsz.div_ceil(PAGE_SIZE)
    }
}

/// A parsed and checked program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Image {
    /// Where execution starts.
    pub entry: u64,
    segments: [Segment; MAX_SEGMENTS],
    count: usize,
}

impl Image {
    /// The loadable segments, in the order the file lists them.
    pub fn segments(&self) -> &[Segment] {
        &self.segments[..self.count]
    }
}

/// Why a file was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Shorter than an ELF header.
    TooShort,
    /// Does not start with `\x7fELF`.
    BadMagic,
    /// Not a 64-bit file.
    NotElf64,
    /// Not little-endian.
    NotLittleEndian,
    /// An ELF version other than 1.
    BadVersion,
    /// Not a fixed-address executable (`ET_EXEC`). Shared objects and
    /// position-independent executables need relocation, which the loader does
    /// not do.
    NotExecutable,
    /// Not for x86-64.
    WrongMachine,
    /// The header or program header size is not the standard one.
    BadHeaderSize,
    /// The program header table is missing, too large, or outside the file.
    BadProgramHeaders,
    /// A segment type this loader does not implement (dynamic linking, an
    /// interpreter, thread-local storage, or anything unknown).
    UnsupportedSegment(u32),
    /// The stack segment asks for an executable stack.
    ExecutableStack,
    /// The file has no loadable segment.
    NoLoadSegments,
    /// More than [`MAX_SEGMENTS`] loadable segments.
    TooManySegments,
    /// A loadable segment of zero size.
    EmptySegment,
    /// A segment's file bytes lie outside the file.
    SegmentFileRange,
    /// A segment has more file bytes than memory.
    SegmentFileLargerThanMemory,
    /// A segment does not start on a page boundary.
    SegmentUnaligned,
    /// A segment reaches outside the range the policy allows, or wraps.
    SegmentOutOfRange,
    /// Permission bits other than read, write and execute.
    BadFlags,
    /// A segment with no permissions at all.
    NoPermissions,
    /// A segment both writable and executable.
    WritableAndExecutable,
    /// Two segments share a page.
    SegmentsOverlap,
    /// The image needs more memory than the policy allows.
    TooMuchMemory,
    /// The entry point is not in an executable segment's file-backed bytes.
    EntryNotExecutable,
}

fn bytes<const N: usize>(file: &[u8], offset: usize) -> Result<[u8; N], Error> {
    let end = offset.checked_add(N).ok_or(Error::TooShort)?;
    file.get(offset..end)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(Error::TooShort)
}

fn u16_at(file: &[u8], offset: usize) -> Result<u16, Error> {
    bytes::<2>(file, offset).map(u16::from_le_bytes)
}

fn u32_at(file: &[u8], offset: usize) -> Result<u32, Error> {
    bytes::<4>(file, offset).map(u32::from_le_bytes)
}

fn u64_at(file: &[u8], offset: usize) -> Result<u64, Error> {
    bytes::<8>(file, offset).map(u64::from_le_bytes)
}

/// Rounds `value` up to a page, or `None` on overflow.
fn page_up(value: u64) -> Option<u64> {
    value
        .checked_add(PAGE_SIZE - 1)
        .map(|v| v & !(PAGE_SIZE - 1))
}

/// Parses and checks `file` against `policy`.
pub fn parse(file: &[u8], policy: &Policy) -> Result<Image, Error> {
    if file.len() < ELF_HEADER_SIZE {
        return Err(Error::TooShort);
    }
    if file[0..4] != *b"\x7fELF" {
        return Err(Error::BadMagic);
    }
    if file[4] != 2 {
        return Err(Error::NotElf64);
    }
    if file[5] != 1 {
        return Err(Error::NotLittleEndian);
    }
    if file[6] != 1 || u32_at(file, 20)? != 1 {
        return Err(Error::BadVersion);
    }
    if u16_at(file, 16)? != ET_EXEC {
        return Err(Error::NotExecutable);
    }
    if u16_at(file, 18)? != EM_X86_64 {
        return Err(Error::WrongMachine);
    }
    let entry = u64_at(file, 24)?;
    let program_header_offset = u64_at(file, 32)?;
    if usize::from(u16_at(file, 52)?) != ELF_HEADER_SIZE
        || usize::from(u16_at(file, 54)?) != PROGRAM_HEADER_SIZE
    {
        return Err(Error::BadHeaderSize);
    }
    let program_header_count = u64::from(u16_at(file, 56)?);
    if program_header_count == 0 || program_header_count > MAX_PROGRAM_HEADERS {
        return Err(Error::BadProgramHeaders);
    }
    let table_bytes = program_header_count * PROGRAM_HEADER_SIZE as u64;
    let table_end = program_header_offset
        .checked_add(table_bytes)
        .ok_or(Error::BadProgramHeaders)?;
    if table_end > file.len() as u64 {
        return Err(Error::BadProgramHeaders);
    }

    let empty = Segment {
        vaddr: 0,
        memsz: 0,
        file_offset: 0,
        filesz: 0,
        perms: Perms {
            read: false,
            write: false,
            execute: false,
        },
    };
    let mut segments = [empty; MAX_SEGMENTS];
    let mut count = 0;
    let mut total_bytes: u64 = 0;

    for index in 0..program_header_count {
        // Cannot overflow: the table was checked to lie inside the file.
        let base = (program_header_offset + index * PROGRAM_HEADER_SIZE as u64) as usize;
        let kind = u32_at(file, base)?;
        let flags = u32_at(file, base + 4)?;
        match kind {
            PT_LOAD => {}
            PT_NULL | PT_NOTE | PT_PHDR | PT_GNU_EH_FRAME | PT_GNU_RELRO | PT_GNU_PROPERTY => {
                continue
            }
            PT_GNU_STACK => {
                if flags & PF_X != 0 {
                    return Err(Error::ExecutableStack);
                }
                continue;
            }
            other => return Err(Error::UnsupportedSegment(other)),
        }

        let file_offset = u64_at(file, base + 8)?;
        let vaddr = u64_at(file, base + 16)?;
        let filesz = u64_at(file, base + 32)?;
        let memsz = u64_at(file, base + 40)?;

        if memsz == 0 {
            return Err(Error::EmptySegment);
        }
        if filesz > memsz {
            return Err(Error::SegmentFileLargerThanMemory);
        }
        let file_end = file_offset
            .checked_add(filesz)
            .ok_or(Error::SegmentFileRange)?;
        if file_end > file.len() as u64 {
            return Err(Error::SegmentFileRange);
        }
        if vaddr % PAGE_SIZE != 0 {
            return Err(Error::SegmentUnaligned);
        }
        let end = vaddr.checked_add(memsz).ok_or(Error::SegmentOutOfRange)?;
        let page_end = page_up(end).ok_or(Error::SegmentOutOfRange)?;
        if vaddr < policy.min_address || page_end > policy.max_address {
            return Err(Error::SegmentOutOfRange);
        }
        if flags & !(PF_R | PF_W | PF_X) != 0 {
            return Err(Error::BadFlags);
        }
        let perms = Perms {
            read: flags & PF_R != 0,
            write: flags & PF_W != 0,
            execute: flags & PF_X != 0,
        };
        if !(perms.read || perms.write || perms.execute) {
            return Err(Error::NoPermissions);
        }
        if perms.write && perms.execute {
            return Err(Error::WritableAndExecutable);
        }
        if count == MAX_SEGMENTS {
            return Err(Error::TooManySegments);
        }
        for earlier in &segments[..count] {
            let earlier_page_end = page_up(earlier.end()).ok_or(Error::SegmentOutOfRange)?;
            if vaddr < earlier_page_end && earlier.vaddr < page_end {
                return Err(Error::SegmentsOverlap);
            }
        }
        total_bytes = total_bytes
            .checked_add(page_end - vaddr)
            .ok_or(Error::TooMuchMemory)?;
        if total_bytes > policy.max_total_bytes {
            return Err(Error::TooMuchMemory);
        }
        segments[count] = Segment {
            vaddr,
            memsz,
            file_offset,
            filesz,
            perms,
        };
        count += 1;
    }

    if count == 0 {
        return Err(Error::NoLoadSegments);
    }
    let entry_ok = segments[..count].iter().any(|segment| {
        segment.perms.execute && entry >= segment.vaddr && entry - segment.vaddr < segment.filesz
    });
    if !entry_ok {
        return Err(Error::EntryNotExecutable);
    }
    Ok(Image {
        entry,
        segments,
        count,
    })
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests;
