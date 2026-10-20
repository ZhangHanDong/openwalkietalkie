/* SF32LB52x: 16 MiB NOR + 8 MiB PSRAM. NOR is mapped at 0x12000000.
 * 0x12000000..0x1201FFFF: flash table, reserved gap, bootloader (128 KiB).
 * 0x12020000..0x126FFFFF: code and ordinary constants (6.875 MiB).
 * 0x12700000..0x1270FFFF: native Bluetooth database (64 KiB).
 * 0x12710000..0x1271FFFF: Embassy pairing journal (64 KiB).
 * 0x12720000..0x12FFFFFF: application assets (8.875 MiB, flash-assets.x).
 *
 * Both bond stores keep their original addresses. USB updates of the upper
 * bank require the protocol-2 recovery bootloader; old recovery is refused
 * before erasing the application. UART updates must use ELF load segments,
 * not a flat binary whose padding covers the bond stores.
 */
MEMORY
{
  /* Keep both existing 64 KiB Bluetooth stores at 0x12700000..0x1271FFFF.
   * flash-assets.x adds the upper NOR bank without relocating either store. */
  FLASH : ORIGIN = 0x12020000, LENGTH = 7M - 128K

  /* The SDK reserves the last 1K of HPSYS RAM for the inter-core mailbox IPC
   * ring buffers (CH2 0x2007FC00..FDFF, CH1 0x2007FE00..FFFF). Leaving it in
   * the linker's pool lets the stack walk into them and corrupt LCPU IPC in
   * ways that look random. */
  RAM   : ORIGIN = 0x20000000, LENGTH = 512K - 1K

  PSRAM : ORIGIN = 0x60000000, LENGTH = 8M
}

/* The SF32 vector table ends on a 4-byte boundary, while some Thumb code
 * requires 8-byte alignment. Override cortex-m-rt's unaligned default. */
_stext = ALIGN(ADDR(.vector_table) + SIZEOF(.vector_table), 8);

/* Rev A's code, constants and font copies precede the PSRAM runtime buffers.
 * Other boards use NOR directly. Keep the last MiB free on every board for
 * audio-debug's destructive memory test. */
__psram_scratch_start = ORIGIN(PSRAM) + 7M;
__psram_scratch_end = ORIGIN(PSRAM) + LENGTH(PSRAM);
