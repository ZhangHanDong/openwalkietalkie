//! Execute in place from NOR on every board. The clock-configuration fix
//! supersedes Rev A's former copy of code and font atlases into PSRAM.
//!
//! cortex-m-rt sets VTOR to the flash vector table, initializes .data in SRAM
//! and clears .bss. Flash-writing and shutdown routines retain their explicit
//! `.data.*` placement because they must run while NOR is busy or unpowered.
//! This pre-init hook only supplies the stack limit omitted from the HAL's
//! default features; it performs no code, font or vector-table copy.

#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn __pre_init() {
    core::arch::naked_asm!("ldr r0, =_stack_end", "msr MSPLIM, r0", "bx lr");
}
