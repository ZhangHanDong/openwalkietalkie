/* Rev A's temporary updater also executes from PSRAM. Its entire load
 * image must fit below the bond stores for protocol-1 recovery. */
SECTIONS
{
  .flash_assets : ALIGN(32)
  {
    __sassets = .;
    *(.flash_assets .flash_assets.*);
    . = ALIGN(32);
    __eassets = .;
  } > PSRAM AT> FLASH
  __siassets = LOADADDR(.flash_assets);
}
INSERT AFTER .rodata;
