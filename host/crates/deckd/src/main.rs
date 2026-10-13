// The usage strings macOS asks a process for before it may touch
// CoreBluetooth (the deck's voice link, `deckd::ble`), the microphone or
// speech recognition. A bare `deckd` started by launchd
// (tools/install-host.sh) has no bundle to carry an Info.plist, so the
// binary carries one in `__TEXT,__info_plist`; without it the first
// CoreBluetooth call ends the process instead of asking.
//
// It is embedded unconditionally rather than behind a feature, because the
// dev install builds with none (tools/build-host.sh), and that is harmless
// to the bundled build: the plist's CFBundleIdentifier is the
// `ai.cohdl.openwalkietalkie` that tools/package-macos.sh passes to
// `codesign -i` for both Mach-Os, so the bundled deckd signs as exactly what
// it signed as before, and inside the bundle the app's own Info.plist is
// the one macOS reads. See the comment in `Info.plist` beside Cargo.toml.
#[cfg(target_os = "macos")]
embed_plist::embed_info_plist!("../Info.plist");

fn main() {
    std::process::exit(deckd::cli::main());
}
