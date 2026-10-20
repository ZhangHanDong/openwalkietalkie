fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let out_dir = std::path::Path::new(&out);
    std::fs::copy("memory.x", out_dir.join("memory.x")).unwrap();
    std::fs::copy("psram.x", out_dir.join("psram.x")).unwrap();
    // A temporary bootloader updater must fit in protocol-1 recovery's lower
    // bank so it can install protocol 2 before any upper-bank assets are used.
    // hw-b includes hw-a for shared peripherals, but only Rev A needs to
    // avoid NOR reads coupling into the amplifier input during playback.
    let psram = std::env::var_os("CARGO_FEATURE_HW_A").is_some()
        && std::env::var_os("CARGO_FEATURE_HW_B").is_none();
    let bootstrap = std::env::var_os("CARGO_FEATURE_BOOTLOADER_UPDATE").is_some();
    let assets = match (psram, bootstrap) {
        (true, true) => "flash-bootstrap-psram.x",
        (true, false) => "flash-assets-psram.x",
        (false, true) => "flash-bootstrap.x",
        (false, false) => "flash-assets.x",
    };
    std::fs::copy(assets, out_dir.join(assets)).unwrap();
    if psram {
        std::fs::copy("link-psram.x", out_dir.join("link-psram.x")).unwrap();
    }
    println!("cargo:rustc-link-search={out}");
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=psram.x");
    println!("cargo:rerun-if-changed=flash-assets.x");
    println!("cargo:rerun-if-changed=flash-bootstrap.x");
    println!("cargo:rerun-if-changed=flash-assets-psram.x");
    println!("cargo:rerun-if-changed=flash-bootstrap-psram.x");
    println!("cargo:rerun-if-changed=link-psram.x");

    // Section addresses are not 0x10000-aligned; without --nmagic the linker
    // pads them out to page boundaries and the image stops fitting.
    println!("cargo:rustc-link-arg=--nmagic");
    let linker = if psram { "link-psram.x" } else { "link.x" };
    println!("cargo:rustc-link-arg=-T{linker}");
    println!("cargo:rustc-link-arg=-T{assets}");
    println!("cargo:rustc-link-arg=-Tpsram.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
