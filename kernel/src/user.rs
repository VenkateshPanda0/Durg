//! Loading and starting user programs (before the ELF loader, M9).
//!
//! A user program here is a flat blob of position-independent machine code. The
//! loader builds a new address space, copies the code into fresh zeroed frames
//! mapped **read and execute, not write** at [`CODE_BASE`], maps a stack of
//! fresh zeroed frames **read and write, not execute** just below the top of
//! user space, and starts a thread that enters ring 3 at the first byte.
//!
//! Every page is mapped user-accessible, and every frame is zeroed before use, so
//! a task cannot read what a previous owner of its memory left behind. When the
//! task ends, the scheduler destroys the space and every frame goes back to the
//! pool.
//!
//! The programs themselves are tiny assembly routines embedded below. They exist
//! to prove the ring-3 round trip and, in the self-tests, to misbehave on purpose.

use core::arch::global_asm;

use durg_paging::{flags, PAGE_SIZE};

use crate::mm::frame;
use crate::mm::paging::{self, AddressSpace, MapError};
use crate::sched::{self, JoinHandle, SpawnError, UserEntry};

/// Where the program's first byte is mapped.
pub const CODE_BASE: u64 = 0x0000_0000_0040_0000;
/// One past the last byte of the stack. The stack grows down from here.
pub const STACK_TOP: u64 = durg_syscall::USER_SPACE_END;
/// Pages of user stack.
pub const STACK_PAGES: u64 = 4;
/// Largest program the loader accepts.
pub const MAX_CODE_BYTES: usize = 16 * PAGE_SIZE as usize;

/// Why a program could not be started.
#[derive(Debug)]
pub enum LoadError {
    Empty,
    TooBig,
    Map(MapError),
    OutOfFrames,
    Spawn(SpawnError),
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str("program is empty"),
            Self::TooBig => f.write_str("program is too big"),
            Self::Map(error) => write!(f, "mapping failed: {error}"),
            Self::OutOfFrames => f.write_str("out of frames"),
            Self::Spawn(error) => write!(f, "cannot start the thread: {error}"),
        }
    }
}

impl From<MapError> for LoadError {
    fn from(error: MapError) -> Self {
        Self::Map(error)
    }
}

/// Builds an address space for `code` and starts a thread that runs it in ring 3.
pub fn spawn_program(name: &'static str, code: &[u8]) -> Result<JoinHandle, LoadError> {
    if code.is_empty() {
        return Err(LoadError::Empty);
    }
    if code.len() > MAX_CODE_BYTES {
        return Err(LoadError::TooBig);
    }
    let mut space = AddressSpace::new_user()?;
    match populate(&mut space, code) {
        Ok(()) => {}
        Err(error) => {
            // Frees whatever was mapped so far.
            space.destroy_user_half();
            return Err(error);
        }
    }
    let entry = UserEntry {
        rip: CODE_BASE,
        // Sixteen bytes below the top: aligned, and inside the topmost page.
        rsp: STACK_TOP - 16,
    };
    // On failure `spawn_user` frees the space itself.
    sched::spawn_user(name, space, entry).map_err(LoadError::Spawn)
}

fn populate(space: &mut AddressSpace, code: &[u8]) -> Result<(), LoadError> {
    // Code: readable and executable by the user, never writable (W^X).
    let code_flags = flags::PRESENT | flags::USER;
    for (index, piece) in code.chunks(PAGE_SIZE as usize).enumerate() {
        let physical = frame::alloc_zeroed_frame().ok_or(LoadError::OutOfFrames)?;
        if frame::write_bytes(physical, piece).is_none() {
            let _ = frame::free_frame(physical);
            return Err(LoadError::OutOfFrames);
        }
        if let Err(error) =
            space.map_page(CODE_BASE + index as u64 * PAGE_SIZE, physical, code_flags)
        {
            let _ = frame::free_frame(physical);
            return Err(error.into());
        }
    }
    // Stack: readable and writable by the user, never executable.
    let stack_flags = flags::PRESENT | flags::WRITABLE | flags::USER | paging::no_execute();
    for page in 1..=STACK_PAGES {
        let physical = frame::alloc_zeroed_frame().ok_or(LoadError::OutOfFrames)?;
        if let Err(error) = space.map_page(STACK_TOP - page * PAGE_SIZE, physical, stack_flags) {
            let _ = frame::free_frame(physical);
            return Err(error.into());
        }
    }
    Ok(())
}

// ---- embedded programs ------------------------------------------------------
//
// Position independent, entered at the first byte, calling the kernel with the
// provisional ABI: rax = number, rdi/rsi/rdx = arguments, result in rax.
//   0 = debug_print(pointer, length)   1 = yield()   2 = exit(code)

global_asm!(
    r#"
    .section .rodata.user_programs,"a"
    .balign 16

    // Prints a greeting, yields once, exits with code 7.
    .global durg_user_hello_start
    .global durg_user_hello_end
    durg_user_hello_start:
        mov eax, 0
        lea rdi, [rip + 3f]
        mov esi, 17
        syscall
        mov eax, 1
        syscall
        mov eax, 2
        mov edi, 7
        syscall
        ud2
    3:  .ascii "hello from ring 3"
    durg_user_hello_end:
    "#
);

extern "C" {
    static durg_user_hello_start: u8;
    static durg_user_hello_end: u8;
}

/// The bytes between two linker-visible labels.
///
/// # Safety
///
/// `start..end` must be the bounds of a blob the assembly above emitted: valid,
/// initialised, read-only memory for the life of the kernel.
unsafe fn blob(start: *const u8, end: *const u8) -> &'static [u8] {
    // SAFETY: the caller's contract. Both labels are in the same section, so the
    // difference is the blob's length.
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}

/// The `hello` program.
pub fn hello() -> &'static [u8] {
    // SAFETY: the labels bracket the blob emitted in `global_asm!` above.
    unsafe {
        blob(
            core::ptr::addr_of!(durg_user_hello_start),
            core::ptr::addr_of!(durg_user_hello_end),
        )
    }
}

/// Starts `hello`, waits for it, and reports. Run on every boot.
pub fn boot_demo() -> Result<(), &'static str> {
    let handle = spawn_program("hello", hello()).map_err(|_| "could not start the program")?;
    let status = loop {
        if let Some(status) = handle.status() {
            break status;
        }
        sched::yield_now();
    };
    match status {
        sched::ExitStatus::Exited(7) => Ok(()),
        _ => Err("the program did not exit with code 7"),
    }
}

// ---- programs that misbehave on purpose (self-tests only) -------------------

/// Declares a program: the assembly between two labels, and a function that
/// returns its bytes.
#[cfg(feature = "selftest")]
macro_rules! test_program {
    ($function:ident, $start:ident, $end:ident, $assembly:expr) => {
        global_asm!(concat!(
            ".section .rodata.user_programs,\"a\"\n",
            ".balign 16\n",
            ".global ",
            stringify!($start),
            "\n",
            ".global ",
            stringify!($end),
            "\n",
            stringify!($start),
            ":\n",
            $assembly,
            "\n",
            stringify!($end),
            ":\n",
        ));
        extern "C" {
            static $start: u8;
            static $end: u8;
        }
        pub fn $function() -> &'static [u8] {
            // SAFETY: the labels bracket the blob emitted just above.
            unsafe { blob(core::ptr::addr_of!($start), core::ptr::addr_of!($end)) }
        }
    };
}

#[cfg(feature = "selftest")]
pub mod programs {
    use super::*;

    // Exits with the value the previous system call returned. Used by the
    // programs that call the kernel with bad arguments.
    macro_rules! exit_with_rax {
        () => {
            "mov rdi, rax\n mov eax, 2\n syscall\n ud2"
        };
    }

    // -- faults: each must end only the task ---------------------------------

    // Read kernel text. The page is present but supervisor-only: a protection
    // violation (error code: present | user).
    test_program!(
        read_kernel,
        durg_user_read_kernel_start,
        durg_user_read_kernel_end,
        "mov rax, 0xffffffff80000000\n mov rax, [rax]\n ud2"
    );
    // Write kernel text.
    test_program!(
        write_kernel,
        durg_user_write_kernel_start,
        durg_user_write_kernel_end,
        "mov rax, 0xffffffff80000000\n mov byte ptr [rax], 0\n ud2"
    );
    // Jump into kernel text.
    test_program!(
        exec_kernel,
        durg_user_exec_kernel_start,
        durg_user_exec_kernel_end,
        "mov rax, 0xffffffff80000000\n jmp rax"
    );
    // Read an address in user space that nothing maps (not-present fault).
    test_program!(
        read_unmapped,
        durg_user_read_unmapped_start,
        durg_user_read_unmapped_end,
        "mov rax, 0x10000000\n mov rax, [rax]\n ud2"
    );
    // Write to its own code page (mapped read-execute).
    test_program!(
        write_own_code,
        durg_user_write_own_code_start,
        durg_user_write_own_code_end,
        "lea rax, [rip + 1f]\n 1: mov byte ptr [rax], 0\n ud2"
    );
    // Run code from its own stack (mapped read-write, no-execute).
    test_program!(
        exec_stack,
        durg_user_exec_stack_start,
        durg_user_exec_stack_end,
        "mov rax, rsp\n jmp rax"
    );
    // Privileged instructions: each is a general protection fault in ring 3.
    test_program!(
        disable_interrupts,
        durg_user_cli_start,
        durg_user_cli_end,
        "cli\n ud2"
    );
    test_program!(halt, durg_user_hlt_start, durg_user_hlt_end, "hlt\n ud2");
    test_program!(
        read_cr3,
        durg_user_cr3_start,
        durg_user_cr3_end,
        "mov rax, cr3\n ud2"
    );
    test_program!(
        port_io,
        durg_user_port_start,
        durg_user_port_end,
        "mov dx, 0x3f8\n mov al, 0x58\n out dx, al\n ud2"
    );

    // -- system calls with bad arguments: refused, the task goes on ------------

    // debug_print at a kernel address.
    test_program!(
        print_kernel_pointer,
        durg_user_sys_kernel_start,
        durg_user_sys_kernel_end,
        concat!(
            "mov eax, 0\n mov rdi, 0xffff800000000000\n mov esi, 16\n syscall\n ",
            exit_with_rax!()
        )
    );
    // debug_print at an unmapped user address.
    test_program!(
        print_unmapped_pointer,
        durg_user_sys_unmapped_start,
        durg_user_sys_unmapped_end,
        concat!(
            "mov eax, 0\n mov rdi, 0x20000000\n mov esi, 8\n syscall\n ",
            exit_with_rax!()
        )
    );
    // debug_print with a length over the limit.
    test_program!(
        print_too_long,
        durg_user_sys_long_start,
        durg_user_sys_long_end,
        concat!(
            "mov eax, 0\n lea rdi, [rip + 1f]\n mov esi, 100000\n syscall\n ",
            exit_with_rax!(),
            "\n 1: .ascii \"x\""
        )
    );
    // debug_print with a wrapping range.
    test_program!(
        print_wrapping_range,
        durg_user_sys_wrap_start,
        durg_user_sys_wrap_end,
        concat!(
            "mov eax, 0\n mov rdi, 0xfffffffffffffff0\n mov esi, 64\n syscall\n ",
            exit_with_rax!()
        )
    );
    // A system call number that does not exist.
    test_program!(
        unknown_call,
        durg_user_sys_unknown_start,
        durg_user_sys_unknown_end,
        concat!("mov eax, 999\n syscall\n ", exit_with_rax!())
    );
    // Output made of control characters and an escape sequence.
    test_program!(
        print_control_characters,
        durg_user_sys_control_start,
        durg_user_sys_control_end,
        concat!(
            "mov eax, 0\n lea rdi, [rip + 1f]\n mov esi, 12\n syscall\n ",
            "mov edi, 0\n mov eax, 2\n syscall\n ud2\n ",
            "1: .byte 0x1b, 0x5b, 0x32, 0x4a, 0x0a, 0x5b, 0x69, 0x6e, 0x66, 0x6f, 0x5d, 0x0d"
        )
    );

    // -- other behaviour -------------------------------------------------------

    // Sets every callee-saved register, calls the kernel (a yield), and checks
    // they came back. Exits 0 if all six survived, 1 otherwise.
    test_program!(
        keeps_registers,
        durg_user_regs_start,
        durg_user_regs_end,
        concat!(
            "mov rbx, 0x1111\n mov rbp, 0x2222\n mov r12, 0x3333\n ",
            "mov r13, 0x4444\n mov r14, 0x5555\n mov r15, 0x6666\n ",
            "mov eax, 1\n syscall\n ",
            "cmp rbx, 0x1111\n jne 2f\n cmp rbp, 0x2222\n jne 2f\n ",
            "cmp r12, 0x3333\n jne 2f\n cmp r13, 0x4444\n jne 2f\n ",
            "cmp r14, 0x5555\n jne 2f\n cmp r15, 0x6666\n jne 2f\n ",
            "mov edi, 0\n mov eax, 2\n syscall\n ud2\n ",
            "2: mov edi, 1\n mov eax, 2\n syscall\n ud2"
        )
    );
    // Stores the interrupt descriptor table register: a kernel address, unless
    // UMIP forbids it.
    test_program!(
        read_idt,
        durg_user_idt_start,
        durg_user_idt_end,
        "sidt [rsp - 16]\n ud2"
    );
    // Loops forever in ring 3 without calling the kernel.
    test_program!(spin, durg_user_spin_start, durg_user_spin_end, "1: jmp 1b");
    // Reports what the kernel left in its registers on entry: they must all be
    // zero. Exits with the OR of rax..r15 (except rsp).
    test_program!(
        registers_at_entry,
        durg_user_entry_regs_start,
        durg_user_entry_regs_end,
        concat!(
            "or rax, rbx\n or rax, rcx\n or rax, rdx\n or rax, rsi\n or rax, rdi\n ",
            "or rax, rbp\n or rax, r8\n or rax, r9\n or rax, r10\n or rax, r11\n ",
            "or rax, r12\n or rax, r13\n or rax, r14\n or rax, r15\n ",
            exit_with_rax!()
        )
    );
}
