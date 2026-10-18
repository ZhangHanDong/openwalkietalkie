/* Temporary bootloader migration builds must fit entirely below
 * both bond stores. Normal builds use the upper-bank flash-assets.x. */
SECTIONS
{
  .flash_assets : ALIGN(4)
  {
    *(.flash_assets .flash_assets.*);
    . = ALIGN(4);
  } > FLASH
}
INSERT AFTER .rodata;
