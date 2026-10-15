/* Durg kernel linker script.
 *
 * The kernel lives in the higher half of the address space, at the address the
 * Limine boot protocol expects for kernels linked at -2 GiB.
 */

OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)

ENTRY(_start)

/* Segments, with explicit permissions so no section is writable and executable. */
PHDRS
{
    text   PT_LOAD FLAGS((1 << 0) | (1 << 2)); /* execute + read */
    rodata PT_LOAD FLAGS(1 << 2);              /* read */
    data   PT_LOAD FLAGS((1 << 1) | (1 << 2)); /* write + read */
}

SECTIONS
{
    . = 0xffffffff80000000;

    /* Section boundaries for the kernel's own page tables, which map each
     * section with its own permissions (code R+X, read-only data R, data and bss
     * RW without execute). All are page aligned. */
    __kernel_start = .;

    .text : {
        __text_start = .;
        *(.text .text.*)
    } :text
    __text_end = ALIGN(CONSTANT(MAXPAGESIZE));

    /* Page-align each segment so permissions can be applied per page later. */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .rodata : {
        __rodata_start = .;
        *(.rodata .rodata.*)
    } :rodata
    __rodata_end = ALIGN(CONSTANT(MAXPAGESIZE));

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .data : {
        __data_start = .;
        *(.data .data.*)

        /* Limine requests and the base-revision tag. The bootloader scans the
         * kernel image for these, so they must be kept even if unreferenced. */
        KEEP(*(.limine_requests_start))
        KEEP(*(.limine_requests))
        KEEP(*(.limine_requests_end))
    } :data

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data
    __data_end = ALIGN(CONSTANT(MAXPAGESIZE));
    __kernel_end = __data_end;

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
