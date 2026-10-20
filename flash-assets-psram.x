/* Rev A keeps the same upper-bank flash image as the other boards, but
 * __pre_init copies the font assets into PSRAM before any UI can read them. */
MEMORY
{
  FLASH_ASSETS (rx) : ORIGIN = 0x12720000, LENGTH = 0x8E0000
}
SECTIONS
{
  .flash_assets : ALIGN(32)
  {
    __sassets = .;
    *(.flash_assets .flash_assets.*);
    . = ALIGN(32);
    __eassets = .;
  } > PSRAM AT> FLASH_ASSETS
  __siassets = LOADADDR(.flash_assets);
}
INSERT AFTER .rodata;
