/* The SDK reserves 128 KiB at 0x20020000 for bootloader execution, but the
 * flash table loads at most 64 KiB. Use the unused second half for a band.
 * NOLOAD: no flash bytes, no startup copy/zero, always drawn before DMA. */
MEMORY { RECOVERY_DISPLAY (rw) : ORIGIN = 0x20030000, LENGTH = 0x10000 }
SECTIONS
{
    .recovery_scratch 0x20030000 (NOLOAD) : ALIGN(4)
    {
        *(.recovery_scratch)
    } > RECOVERY_DISPLAY
}
INSERT AFTER .bss;
ASSERT(LOADADDR(.data) + SIZEOF(.data) <= 0x20030000,
       "bootloader image overlaps display scratch / exceeds 64 KiB slot")
ASSERT(SIZEOF(.recovery_scratch) <= 0x10000,
       "bootloader display scratch exceeds reserved SRAM")
