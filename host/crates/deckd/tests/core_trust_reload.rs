//! Contract §3.7 lists `trust.*` among the hot keys, and `config.reload`
//! answers `applied: ["trust.gui_requirement", …]`. That answer has to be
//! true: the verifier the UDS door consults must re-read its requirements,
//! both when `config.reload` is called and when the file watcher pushes a
//! `CoreMsg::Config`.
mod core_support;

use deckd::config::Config;
use deckd::core::{CallCtx, CoreMsg};
use deckd::trust::{MacVerifier, PeerVerdict, PeerVerifier};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A verifier that records every `[trust]` reload it is told about.
#[derive(Debug, Default)]
struct Recorder {
    seen: Mutex<Vec<(String, Option<String>)>>,
}

impl Recorder {
    fn last(&self) -> Option<(String, Option<String>)> {
        self.seen.lock().unwrap().last().cloned()
    }
}

impl PeerVerifier for Recorder {
    fn verify(&self, _pid: u32) -> PeerVerdict {
        PeerVerdict::default()
    }

    fn reconfigure(&self, gui_requirement: &str, dev_requirement: Option<&str>) {
        self.seen.lock().unwrap().push((
            gui_requirement.to_string(),
            dev_requirement.map(String::from),
        ));
    }
}

const NEXT_GUI: &str = r#"anchor apple generic and identifier "com.conol.reloaded""#;
const NEXT_DEV: &str = r#"cdhash H"1111111111111111111111111111111111111111""#;

#[test]
fn a_trust_reload_reaches_the_verifier_the_uds_door_consults() {
    let (mut core, parts) = core_support::build("trust-reload", 64);
    let recorder = Arc::new(Recorder::default());
    core.set_verifier(recorder.clone());
    let first = Config::default();
    core.set_config(Arc::new(first.clone()));
    assert_eq!(
        recorder.last().map(|(gui, _)| gui),
        Some(first.trust.gui_requirement.clone()),
        "the verifier starts from the effective config"
    );

    // `config.reload` re-reads `config.toml` from the data dir.
    std::fs::write(
        parts.dir.join("config.toml"),
        format!("[trust]\ngui_requirement = {NEXT_GUI:?}\ndev_requirement = {NEXT_DEV:?}\n"),
    )
    .unwrap();
    let reply = core.call(
        CallCtx {
            principal: deck_model::Principal::Uid(501),
            session: deck_model::SessionId(1),
            scopes: vec![deck_model::Scope::Admin],
            trusted: false,
            own: std::collections::BTreeSet::new(),
        },
        "config.reload",
        serde_json::json!({}),
        Instant::now(),
    );
    let result = reply.result.expect("config.reload");
    let applied: Vec<String> = serde_json::from_value(result["applied"].clone()).unwrap();
    assert!(
        applied.iter().any(|k| k == "trust.gui_requirement")
            && applied.iter().any(|k| k == "trust.dev_requirement"),
        "config.reload reports them applied: {applied:?}"
    );
    assert_eq!(
        recorder.last(),
        Some((NEXT_GUI.to_string(), Some(NEXT_DEV.to_string()))),
        "…and they really reached the verifier"
    );

    // The file watcher's path is `CoreMsg::Config`, which must apply too.
    let mut watched = Config::default();
    watched.trust.gui_requirement = "identifier \"com.conol.watched\"".into();
    watched.trust.dev_requirement = None;
    core.on_msg(CoreMsg::Config(Arc::new(watched)), Instant::now());
    assert_eq!(
        recorder.last(),
        Some(("identifier \"com.conol.watched\"".to_string(), None))
    );
    let _ = std::fs::remove_dir_all(&parts.dir);
}

/// The real verifier swaps the requirement strings it checks against, so a
/// reload changes what the *next* peer is measured by.
#[test]
fn the_mac_verifier_swaps_its_requirements_on_a_reload() {
    let verifier = MacVerifier::new("identifier \"before\"".into(), None);
    assert_eq!(verifier.requirements().gui, "identifier \"before\"");
    assert_eq!(verifier.requirements().dev, None);
    verifier.reconfigure(NEXT_GUI, Some(NEXT_DEV));
    assert_eq!(verifier.requirements().gui, NEXT_GUI);
    assert_eq!(verifier.requirements().dev.as_deref(), Some(NEXT_DEV));
    // Reconfiguring with the same values is a no-op, and the verifier
    // keeps working: this process is never the signed GUI.
    verifier.reconfigure(NEXT_GUI, Some(NEXT_DEV));
    assert_eq!(verifier.requirements().gui, NEXT_GUI);
    assert!(!verifier.verify(std::process::id()).trusted);
}
