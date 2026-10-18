/* Shared by the Embassy and native linkers. Total application capacity is
 * 15.75 MiB: 6.875 MiB below the bond stores, 8.875 MiB above them.
 * Protocol-2 recovery preserves the intervening 128 KiB on every update. */
MEMORY
{
  FLASH_ASSETS (rx) : ORIGIN = 0x12720000, LENGTH = 0x8E0000
}
SECTIONS
{
  .flash_assets : ALIGN(4)
  {
    *(.flash_assets .flash_assets.*);
    . = ALIGN(4);
  } > FLASH_ASSETS
}
INSERT BEFORE .text;
