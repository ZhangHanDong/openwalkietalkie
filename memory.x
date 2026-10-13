/* LilyGo T-Display SF32 (SF32LB52x, 16MB NOR + 8MB PSRAM).
 *
 * FLASH is the *application* partition, not the whole chip. These numbers are
 * read back from the flash table living at 0x12000000 on the actual board:
 *
 *   sftool -c SF32LB52 -p <port> read_flash ftab.bin@0x12000000:256
 *
 *   magic "SECF"     0x12000000
 *   bootloader       0x12010000  128K
 *   application      0x12020000    7M   <-- this is us
 *   LCPU image       0x10010000   64K
 *
 * We only ever write the application partition, which is why the stock
 * bootloader and flash table survive a flash and the board stays recoverable.
 * If you re-partition, re-read the ftab and update the LENGTH here to match --
 * the linker cannot detect an overrun into the next partition on its own.
 */
MEMORY
{
  /* The last 64 KiB is an append-only pairing-key journal. Firmware flashes
   * only ELF load segments, so this reserved area survives app updates. */
  FLASH : ORIGIN = 0x12020000, LENGTH = 7M - 64K

  /* The SDK reserves the last 1K of HPSYS RAM for the inter-core mailbox IPC
   * ring buffers (CH2 0x2007FC00..FDFF, CH1 0x2007FE00..FFFF). Leaving it in
   * the linker's pool lets the stack walk into them and corrupt LCPU IPC in
   * ways that look random. */
  RAM   : ORIGIN = 0x20000000, LENGTH = 512K - 1K

  PSRAM : ORIGIN = 0x60000000, LENGTH = 8M
}
