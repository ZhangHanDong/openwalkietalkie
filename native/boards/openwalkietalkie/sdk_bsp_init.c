/* Reuse the SDK clock, flash, PSRAM and radio-core startup implementation.
 * Compile through this wrapper so each core gets its own build-directory object.
 * Keep the basename ending in bsp_init: the SDK linker places *bsp_init.o in
 * SRAM. Clock/divider helpers must remain executable while XIP NOR's clock
 * source and divider are being changed by sifli_pm_run(). */
#include "bsp_init.c"
