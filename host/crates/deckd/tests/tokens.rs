//! Contract §5 test 14, the file-level half: `deckd::tokens` against a real
//! `tokens.toml`. The CLI half (`deckd token create|list|revoke|rotate`)
//! joins this file with the `deckd token` subcommand.
use deck_model::{Scope, Ts};
use deckd::tokens::{hash_hex, parse_expiry, Tokens};
use std::time::Duration;

#[test]
fn create_verify_revoke_rotate() {
    let dir = std::env::temp_dir().join(format!("deckd-tokens-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("tokens.toml");
    let now = Ts(1_789_760_523_000);
    // Create: the plaintext exists once, the disk holds its hash, mode 0600.
    let mut tokens = Tokens::load(&path).unwrap();
    let plain = tokens
        .create(
            "gate",
            vec![Scope::Observe, Scope::Agent, Scope::Operate],
            None,
            None,
            now,
        )
        .unwrap();
    tokens.save(&path).unwrap();
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(plain.starts_with("dk_") && plain.len() == 46);
    assert!(!on_disk.contains(&plain));
    assert!(on_disk.contains(&format!("hash = \"{}\"", hash_hex(&plain))));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // Verify from a fresh load.
    let loaded = Tokens::load(&path).unwrap();
    let t = loaded.verify(&plain, now).expect("the plaintext verifies");
    assert_eq!(t.name, "gate");
    assert_eq!(t.scopes, vec![Scope::Observe, Scope::Agent, Scope::Operate]);
    assert!(loaded.verify("dk_not_it", now).is_none());
    // Expiry: `12h` from now is valid now and invalid afterwards.
    let expires = parse_expiry("12h", now).unwrap();
    let mut tokens = loaded;
    let short = tokens
        .create("short", vec![Scope::Observe], Some(expires), None, now)
        .unwrap();
    assert!(tokens.verify(&short, now).is_some());
    assert!(tokens
        .verify(&short, now.plus(Duration::from_secs(12 * 3600)))
        .is_none());
    // Rotate invalidates the old value and keeps the scopes.
    let rotated = tokens.rotate("gate", now).unwrap();
    assert_ne!(rotated, plain);
    assert!(tokens.verify(&plain, now).is_none());
    assert_eq!(
        tokens.verify(&rotated, now).unwrap().scopes,
        vec![Scope::Observe, Scope::Agent, Scope::Operate]
    );
    // Revoke.
    tokens.revoke("short").unwrap();
    assert!(tokens.verify(&short, now).is_none());
    tokens.save(&path).unwrap();
    let loaded = Tokens::load(&path).unwrap();
    let names: Vec<&str> = loaded.list().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["gate"]);
    std::fs::remove_dir_all(dir).unwrap();
}
