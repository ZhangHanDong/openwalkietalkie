fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let out_dir = std::path::Path::new(&out);
    std::fs::copy("memory.x", out_dir.join("memory.x")).unwrap();
    std::fs::copy("psram.x", out_dir.join("psram.x")).unwrap();
    println!("cargo:rustc-link-search={out}");
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=psram.x");

    // Section addresses are not 0x10000-aligned; without --nmagic the linker
    // pads them out to page boundaries and the image stops fitting.
    println!("cargo:rustc-link-arg=--nmagic");
    // Rev A runs from PSRAM (link-psram.x, src/xip.rs): its speaker
    // amplifier picks up every fetch from the external NOR.
    std::fs::copy("link-psram.x", out_dir.join("link-psram.x")).unwrap();
    println!("cargo:rerun-if-changed=link-psram.x");
    if std::env::var_os("CARGO_FEATURE_HW_A").is_some() {
        println!("cargo:rustc-link-arg=-Tlink-psram.x");
    } else {
        println!("cargo:rustc-link-arg=-Tlink.x");
    }
    println!("cargo:rustc-link-arg=-Tpsram.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
