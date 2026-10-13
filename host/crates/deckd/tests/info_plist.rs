//! The bare `deckd`'s embedded Info.plist (`crates/deckd/Info.plist`,
//! `src/main.rs`) is a second copy of facts the bundle owns, so this row
//! pins it to them:
//!
//! * its `CFBundleIdentifier` is the identifier `tools/package-macos.sh`
//!   signs both bundled Mach-Os with — the bundled deckd carries this
//!   section too, and must keep signing as what it signed as before;
//! * every usage string the bundle's `packaging/macos/Info.plist.in`
//!   declares is in it word for word, CoreBluetooth's included;
//! * and the built binary really carries it.

use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the crate sits three levels under the repository root")
}

fn read(relative: &str) -> String {
    let path = repo().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `<key>K</key> <string>V</string>` pairs, in order. Enough of a plist
/// reader for two hand-written files with no nesting under the keys read.
fn strings(plist: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = plist;
    while let Some(at) = rest.find("<key>") {
        rest = &rest[at + 5..];
        let Some(end) = rest.find("</key>") else {
            break;
        };
        let key = rest[..end].to_string();
        rest = &rest[end + 6..];
        let trimmed = rest.trim_start();
        if let Some(value) = trimmed.strip_prefix("<string>") {
            if let Some(end) = value.find("</string>") {
                out.push((key, value[..end].to_string()));
            }
        }
    }
    out
}

fn value<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

#[test]
fn the_embedded_plist_is_the_bundles_identity_and_usage_strings() {
    let embedded = strings(&read("host/crates/deckd/Info.plist"));
    let bundle = strings(&read("packaging/macos/Info.plist.in"));

    let script = read("tools/package-macos.sh");
    let signed_as = script
        .lines()
        .find_map(|l| l.strip_prefix("identifier="))
        .expect("package-macos.sh names the codesign identifier");
    assert_eq!(value(&embedded, "CFBundleIdentifier"), Some(signed_as));
    assert_eq!(value(&bundle, "CFBundleIdentifier"), Some(signed_as));

    let usage: Vec<&(String, String)> = bundle
        .iter()
        .filter(|(k, _)| k.ends_with("UsageDescription"))
        .collect();
    assert!(
        usage
            .iter()
            .any(|(k, _)| k == "NSBluetoothAlwaysUsageDescription"),
        "deckd uses CoreBluetooth: the bundle needs the usage string"
    );
    for (key, text) in usage {
        assert_eq!(
            value(&embedded, key),
            Some(text.as_str()),
            "{key} differs between the bundle and the bare binary"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn the_built_deckd_carries_the_plist() {
    let plist = std::fs::read(repo().join("host/crates/deckd/Info.plist")).unwrap();
    let binary = std::fs::read(env!("CARGO_BIN_EXE_deckd")).expect("the deckd binary");
    assert!(
        binary.windows(plist.len()).any(|w| w == plist.as_slice()),
        "src/main.rs embeds crates/deckd/Info.plist byte for byte"
    );
}
