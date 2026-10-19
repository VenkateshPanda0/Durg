/* Layout of a Durg user program.
 *
 * Fixed addresses (no relocation), one segment per permission set, each
 * starting on a page boundary so the loader can give every page exactly the
 * permissions of its segment:
 *
 *   0x400000  text    read + execute
 *             rodata  read
 *   0x600000  data    read + write   (.data then .bss)
 *
 * Two different programs deliberately put their data at the SAME address; the
 * isolation tests rely on that.
 */

ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);   /* R X */
    rodata PT_LOAD FLAGS(4);   /* R   */
    data   PT_LOAD FLAGS(6);   /* R W */
}

SECTIONS
{
    . = 0x400000;
    __image_start = .;

    .text : { *(.text .text.*) } :text

    . = ALIGN(4096);
    .rodata : { *(.rodata .rodata.*) } :rodata
    __image_ro_end = ALIGN(4096);

    /* The gap between here and the data segment is NOT mapped. */
    . = 0x600000;
    __image_data_start = .;
    .data : { *(.data .data.*) } :data
    .bss  : { *(.bss .bss.*) *(COMMON) } :data

    . = ALIGN(4096);
    __image_end = .;

    /DISCARD/ : { *(.eh_frame*) *(.comment) *(.note*) *(.gnu*) }
}
