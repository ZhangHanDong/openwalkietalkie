//! cortex-m-rt's `__pre_init`: set the stack limit (what the HAL's
//! `set-msplim` feature did) and, on Rev A, run the firmware from PSRAM
//! instead of the external NOR.
//!
//! On BodyV6.13 the speaker amplifier's summing node crosses the NOR's
//! clock and data lines on an inner layer, so every execute-in-place fetch
//! from the NOR clicks in the speaker: with the radio busy, a click train at
//! the BLE connection interval that sounds like hiss. The in-package PSRAM's
//! bus never reaches the PCB, so code and constants fetched from it are
//! isolated from that coupling. link-psram.x links .text and .rodata at
//! PSRAM 0x60000000, followed by the font assets from flash-assets-psram.x.
//! Their load images stay in NOR; `__pre_init` below, which cortex-m-rt's
//! Reset calls before RAM init, copies them, checks the copy, and points
//! VTOR at a copy of the vector table in SRAM. After that the NOR is only
//! read for .data's load image (once, by Reset) and written by the bond
//! journal.
//!
//! It is assembly in the `.boot` section, which stays in the NOR: nothing
//! in .text exists yet when it runs, so it may not call any Rust (not even
//! `memcpy`). The bootloader leaves the PSRAM initialised and the caches and
//! MPU off; the copy still cleans the D-cache over it and invalidates the
//! I-cache in case they are on. A copy that does not read back after three
//! tries leaves the bootloader's recovery request ("OWTR" in RTC BKP7R) and
//! resets the core, so a failing PSRAM ends in USB recovery, not in code
//! that runs off garbage.

/// cortex-m-rt's pre-RAM-init hook (see the module comment). A naked
/// function rather than `global_asm!`, which fat LTO emits twice.
#[cfg(all(feature = "hw-a", not(feature = "hw-b")))]
#[unsafe(no_mangle)]
#[unsafe(naked)]
#[link_section = ".boot.pre_init"]
pub unsafe extern "C" fn __pre_init() {
    core::arch::naked_asm!(
        "    ldr r0, =_stack_end",
        "    msr MSPLIM, r0",
        "    push {{r4, r5, r6, lr}}",
        "    movs r6, #3",
        "2:",
        "    ldr r0, =__stext",
        "    ldr r1, =__etext",
        "    ldr r2, =__sitext",
        "    bl 9f",
        "    cbnz r3, 3f",
        "    ldr r0, =__srodata",
        "    ldr r1, =__erodata",
        "    ldr r2, =__sirodata",
        "    bl 9f",
        "    cbnz r3, 3f",
        "    ldr r0, =__sassets",
        "    ldr r1, =__eassets",
        "    ldr r2, =__siassets",
        "    bl 9f",
        "    cbnz r3, 3f",
        "    ldr r0, =__vector_table_ram",
        "    ldr r1, =__vector_table_ram_end",
        "    ldr r2, =__vector_table",
        "    bl 9f",
        "    cbnz r3, 3f",
        // Clean and invalidate the D-cache over the PSRAM copy (DCCIMVAC),
        // invalidate the I-cache (ICIALLU), then switch VTOR.
        "    ldr r0, =__stext",
        "    ldr r1, =__eassets",
        "    ldr r2, =0xE000EF70",
        "4:",
        "    str r0, [r2]",
        "    adds r0, r0, #32",
        "    cmp r0, r1",
        "    blo 4b",
        "    dsb",
        "    ldr r2, =0xE000EF50",
        "    movs r0, #0",
        "    str r0, [r2]",
        "    ldr r2, =0xE000ED08",
        "    ldr r0, =__vector_table_ram",
        "    str r0, [r2]",
        "    dsb",
        "    isb",
        "    pop {{r4, r5, r6, pc}}",
        // A copy that did not read back: try again, then give up to recovery.
        "3:",
        "    subs r6, r6, #1",
        "    bne 2b",
        "    ldr r2, =0x500CB04C",
        "    ldr r0, =0x4F575452",
        "    str r0, [r2]",
        "    dsb",
        "    ldr r2, =0xE000ED0C",
        "    ldr r0, =0x05FA0004",
        "    str r0, [r2]",
        "    dsb",
        "5:",
        "    b 5b",
        // Copy words [r2..] to [r0, r1) and read them back: r3 = 0 when every
        // word matches. Uses r0-r5.
        "9:",
        "    mov r4, r0",
        "    mov r5, r2",
        "6:",
        "    cmp r0, r1",
        "    bhs 7f",
        "    ldr r3, [r2], #4",
        "    str r3, [r0], #4",
        "    b 6b",
        "7:",
        "    dsb",
        "    movs r3, #0",
        "8:",
        "    cmp r4, r1",
        "    bhs 1f",
        "    ldr r0, [r4], #4",
        "    ldr r2, [r5], #4",
        "    cmp r0, r2",
        "    beq 8b",
        "    movs r3, #1",
        "1:",
        "    bx lr",
        "    .ltorg",
    );
}

/// Rev B and LilyGo run from NOR as linked by cortex-m-rt's link.x:
/// only the stack limit.
#[cfg(not(all(feature = "hw-a", not(feature = "hw-b"))))]
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn __pre_init() {
    core::arch::naked_asm!("ldr r0, =_stack_end", "msr MSPLIM, r0", "bx lr");
}
