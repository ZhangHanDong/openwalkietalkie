SECTIONS
{
  .psram_bss (NOLOAD) :
  {
    *(.psram_bss .psram_bss.*);
  } > PSRAM
}

ASSERT(ADDR(.psram_bss) + SIZEOF(.psram_bss) <= __psram_scratch_start,
       "PSRAM buffers overlap the reserved diagnostic scratch area");
