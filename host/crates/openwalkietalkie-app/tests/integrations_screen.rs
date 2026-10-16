//! What the **Integrations** screen shows, driven through the real reducer
//! and the real reply shapes (§8.2's Integrations row).
//!
//! These are the screen's tests as far as a test can go without a window:
//! the daemon's own answers in, and every line of §8.2's Integrations row
//! read back out through `openwalkietalkie_app::present` — the same functions
//! the layout calls. What they cannot check is pixels; what they do check is
//! that the daemon's answers end up saying the right thing on screen, which
//! is where a reply shape and a view most easily drift apart.
//!
//! The last one is contract §6's exit test: **a token created in the GUI is
//! shown once.**
use deck_gui_core::admin::{Hijack, Plan, Report, Secret, Status, TokenRow, Verb};
use deck_gui_core::{ApiEvent, AppEvent, HostState, UiAction};
use deck_model::{Event, EventKind, Listener, Principal, Snapshot, Tone, Ts};
use openwalkietalkie_app::integrations::{self, Shown};
use openwalkietalkie_app::present;
use serde_json::{json, Value};

/// A fixed stamp: nothing under test reads a clock.
const TS: Ts = Ts::from_millis(1_789_760_523_120);

fn event(rev: u64, kind: EventKind, data: Value) -> AppEvent {
    AppEvent::Api(ApiEvent::Event(Event {
        rev,
        ts: TS,
        kind,
        principal: Principal::Uid(501),
        data,
    }))
}

fn live() -> HostState {
    let snapshot = Snapshot {
        rev: 10,
        ..Snapshot::default()
    };
    let mut state = HostState::detached();
    state.handle_event(AppEvent::Api(ApiEvent::Connected { snapshot }));
    state
}

/// `integrations.status` as the daemon answers it (`{targets, warnings}`),
/// with one row per party and a status word apiece.
fn status_reply() -> Value {
    json!({
        "targets": [
            {
                "target": "claude",
                "status": "outdated",
                "detected": {"path": "/opt/homebrew/bin/claude", "version": "2.1.0"},
                "files": ["/Users/t/.claude/settings.json"],
                "backups": ["/Users/t/Library/Application Support/OpenWalkieTalkie/backups/settings.json.1764.bak"],
                "detail": "allowedHttpHookUrls names one pattern that is not deckd's",
                "limits": [
                    "settings.json is the lowest-precedence level, so \"installed\" means our entries are in the user file, not that they win"
                ]
            },
            {
                "target": "codex",
                "status": "untrusted",
                "detected": {"path": "/usr/local/bin/codex", "version": "0.9.1"},
                "files": ["/Users/t/.codex/hooks.json", "/Users/t/.codex/config.toml"],
                "backups": [],
                "detail": "written, never reviewed in /hooks"
            },
            {"target": "codex-desktop", "status": "installed", "files": ["/Users/t/Applications/Codex with OpenWalkieTalkie.app/Contents/Info.plist"]},
            {"target": "gemini", "status": "absent"},
            {"target": "claude-desktop", "status": "installed"}
        ],
        "warnings": ["this bundle is outside /Applications"]
    })
}

#[test]
fn every_party_gets_a_card_with_its_binary_its_status_word_and_its_limits() {
    let (rows, warnings) = Report::list(&status_reply());
    assert_eq!(rows.len(),
        5, "CLI and desktop integrations each get a card");
    assert_eq!(warnings, vec!["this bundle is outside /Applications"]);
    // The screen draws its cards from its own list, so every party has one
    // whatever the daemon answered.
    for target in present::TARGETS {
        let row = rows.iter().find(|r| r.target == target);
        assert!(row.is_some(), "{target} has no card");
        assert!(!present::party_name(target).is_empty());
    }
    let claude = rows.iter().find(|r| r.target == "claude").unwrap();
    assert_eq!(claude.status, Status::Outdated);
    assert_eq!(present::status_word(&claude.status), "outdated");
    assert_eq!(present::status_tone(&claude.status), Tone::Amber);
    assert!(present::status_help(&claude.status).contains("plugin"));
    assert_eq!(
        present::detected_line("claude", claude.detected.as_ref()),
        "2.1.0 \u{b7} /opt/homebrew/bin/claude"
    );
    // §4.11's honest limits are shown, not summarised.
    assert_eq!(claude.limits.len(), 1);
    assert!(claude.limits[0].contains("lowest-precedence"));
    // §8.2's "last event time": this daemon never fills it in, and the card
    // says which of the two that is.
    assert!(present::last_event_line(claude.last_event_at.as_deref()).contains("not reported"));
    // §8.2's "backups with Restore": they are listed by their own names.
    assert_eq!(
        present::backup_label(&claude.backups[0]),
        "settings.json.1764.bak"
    );

    // §5.2's trust gate, which is a status of its own and not a failure.
    let codex = rows.iter().find(|r| r.target == "codex").unwrap();
    assert_eq!(codex.status, Status::Untrusted);
    assert!(present::status_help(&codex.status).contains("/hooks"));

    // `absent` is the only word that offers Install rather than
    // Update/Repair/Remove.
    let gemini = rows.iter().find(|r| r.target == "gemini").unwrap();
    assert!(!gemini.status.installed());
    assert_eq!(present::status_tone(&gemini.status), Tone::Gray);
    for installed in ["claude", "codex", "claude-desktop"] {
        let row = rows.iter().find(|r| r.target == installed).unwrap();
        assert!(row.status.installed(), "{installed}");
    }
}

#[test]
fn hooks_hijack_risk_reaches_the_integrations_screen_off_the_mirror_with_its_listener() {
    let mut state = live();
    // §5.7: `integrations.status` cannot produce this word — it answers from
    // its own `Paths`/`Config` with no handle on the daemon's service state.
    let (rows, _) = Report::list(&status_reply());
    assert!(
        !rows
            .iter()
            .any(|r| r.status.as_str() == "hooks_hijack_risk"),
        "the installer cannot see the port"
    );
    assert_eq!(Hijack::of(&state.mirror().service), None);

    // §4.1 puts it on `service.degraded{reason, port, listener}` instead,
    // and the reducer folds it into the mirror the screen reads.
    let effects = state.handle_event(event(
        11,
        EventKind::ServiceDegraded,
        json!({
            "reason": "hooks_hijack_risk",
            "port": 47201,
            "listener": {"pid": 812, "command": "node /usr/local/bin/other"}
        }),
    ));
    assert_eq!(effects.len(), 1, "the one-time toast");
    assert_eq!(
        state.mirror().service.listener,
        Some(Listener {
            pid: 812,
            command: Some("node /usr/local/bin/other".into())
        })
    );
    let hijack = Hijack::of(&state.mirror().service).expect("a stranger answers on the port");
    let line = present::hijack_line(&hijack);
    assert!(line.contains("47201"));
    assert!(line.contains("pid 812"), "§8.2 asks for the pid");
    assert!(
        line.contains("node /usr/local/bin/other"),
        "§8.2 asks for the command"
    );
    // It belongs on Claude's card: the risk is Claude's **http** hooks.
    assert!(present::allowlist_applies("claude"));
    assert!(!present::allowlist_applies("codex"));

    // A bind that merely lost is not this: nothing answers, so nothing is
    // being sent to a stranger.
    state.handle_event(event(
        12,
        EventKind::ServiceDegraded,
        json!({"reason": "port_in_use", "port": 47201, "listener": {"pid": 812}}),
    ));
    assert_eq!(Hijack::of(&state.mirror().service), None);
}

#[test]
fn what_this_writes_shows_the_diff_and_the_allowed_http_hook_urls_note_before_installing() {
    // The daemon's own dry run, notes and all.
    let daemon_note = "ADDS allowedHttpHookUrls, which did not exist: once this key is defined \
                       at any settings level, EVERY http hook on this machine \u{2014} yours, a \
                       project's, a plugin's \u{2014} runs only if its URL matches the merged \
                       allowlist";
    let reply = json!({
        "target": "claude",
        "dry_run": true,
        "changes": [{"path": "/Users/t/.claude/settings.json", "action": "update"}],
        "notes": [daemon_note, "writes the SessionStart hook in exec form"],
        "conflicts": ["a foreign PreToolUse hook is left alone"],
        "next_steps": []
    });
    assert!(Plan::is_dry_run(&reply), "nothing is written by a preview");
    let plan = Plan::parse(&reply);
    assert_eq!(
        present::plan_headline(Verb::Install, &plan.target),
        "Install \u{2014} Claude Code"
    );
    assert_eq!(
        present::change_line(&plan.changes[0]),
        "update /Users/t/.claude/settings.json"
    );
    // The installer's sentence is shown verbatim...
    assert_eq!(plan.allowlist_notes(), vec![daemon_note]);
    // ...and the window's own standing warning goes on every Claude plan,
    // including the ones whose notes never mention the key, because "the
    // list already exists" still leaves this machine inside a merged
    // allowlist.
    let appends = Plan::parse(&json!({
        "target": "claude",
        "dry_run": true,
        "changes": [{"path": "/Users/t/.claude/settings.json", "action": "update"}],
        "notes": ["appends http://127.0.0.1:47201/v1/hooks/* to the existing allowedHttpHookUrls"]
    }));
    assert_eq!(appends.allowlist_notes().len(), 1);
    let silent = Plan::parse(&json!({
        "target": "claude",
        "dry_run": true,
        "changes": [],
        "notes": ["nothing about the allowlist at all"]
    }));
    assert!(silent.allowlist_notes().is_empty());
    assert!(
        present::allowlist_applies("claude"),
        "so the sheet still carries the standing note"
    );
    assert!(present::ALLOWLIST_NOTE.contains("EVERY http hook"));
    // A plan that writes nothing says so and offers no Write button.
    assert!(silent.is_noop());
    assert!(present::NOTHING_TO_WRITE.contains("already"));

    // Codex's trust opt-in is the only verb-and-target pair that carries a
    // tick, and it is never on an uninstall.
    assert!(Verb::Install.takes_trust() && Verb::Repair.takes_trust());
    assert!(!Verb::Uninstall.takes_trust());
    // And a preview is a preview: `dry_run` is always in the params.
    let (method, params) = UiAction::Integrations {
        verb: Verb::Install,
        target: "claude".into(),
        trust: false,
        dry_run: true,
    }
    .request();
    assert_eq!(method, "integrations.install");
    assert_eq!(params["dry_run"], true);
}

#[test]
fn ghostty_automation_is_a_grant_and_never_offers_an_install_button() {
    // It is not an `integrations.*` target, so it is not in the status
    // answer and there is no file to write for it.
    let (rows, _) = Report::list(&status_reply());
    assert!(!rows.iter().any(|r| r.target.contains("ghostty")));
    assert!(!present::TARGETS.contains(&"ghostty"));

    // `perm.status` is where it comes from (§5.5).
    assert_eq!(UiAction::PermStatus.request().0, "perm.status");
    let reply = json!({
        "microphone": "granted",
        "speech": "not_determined",
        "automation": {"ghostty": "denied"},
        "accessibility": "unknown",
        "limits": []
    });
    let ghostty = reply["automation"]["ghostty"].as_str();
    let (word, tone, help) = present::automation_line(ghostty);
    assert_eq!((word, tone), ("denied", Tone::Red));
    assert!(help.contains("System Settings"));
    // Until something has tried to inject, macOS has not been asked at all.
    let (word, tone, _) = present::automation_line(Some("unknown"));
    assert_eq!((word, tone), ("unknown", Tone::Gray));
}

#[test]
fn the_token_list_reads_its_scopes_and_says_what_revoking_the_hooks_token_breaks() {
    let reply = json!([
        {
            "name": "hooks",
            "scopes": ["hooks:ingest"],
            "created": "2026-09-20T10:00:00.000Z",
            "expires": null,
            "embedded_in": ["~/.claude/settings.json"],
            "client": null
        },
        {"name": "script", "scopes": ["observe", "operate"], "embedded_in": []}
    ]);
    let rows = TokenRow::list(&reply);
    assert_eq!(rows.len(), 2);
    assert_eq!(present::token_scopes_line(&rows[0]), "hooks:ingest");
    assert!(present::token_life_line(&rows[0]).contains("no expiry"));
    let embedded = present::token_embedded_line(&rows[0]).expect("the hooks token is embedded");
    assert!(embedded.contains("~/.claude/settings.json"));
    assert!(embedded.contains("Repair"));
    assert_eq!(present::token_embedded_line(&rows[1]), None);
    assert_eq!(present::token_scopes_line(&rows[1]), "observe, operate");
    // Revoke and rotate name the token and nothing else.
    assert_eq!(
        UiAction::TokenRevoke {
            name: "script".into()
        }
        .request()
        .1["name"],
        "script"
    );
}

/// Contract §6's exit test: **a token created in the GUI is shown once.**
///
/// The plaintext exists in one reply and in one [`Shown`], and then it is
/// gone: it is never in the mirror, never in `HostState`, never in the
/// request that made it, and never in a formatted string — which is what
/// every log line, toast and panic message is built from.
#[test]
fn a_token_created_in_the_gui_is_shown_once_and_is_in_no_mirror_state_or_log_line_after() {
    const PLAINTEXT: &str = "owt_live_3f8a91c4d70b25e6";
    let mut state = live();

    // The call that makes one carries a name and scopes, and never a value.
    let (method, params) = UiAction::TokenCreate {
        name: "script".into(),
        scopes: vec!["observe".into()],
        expires: None,
    }
    .request();
    assert_eq!(method, "token.create");
    assert!(!params.to_string().contains(PLAINTEXT));

    // The reply is the only place it ever is. Read once, into a `Secret`.
    let reply = json!({"token": PLAINTEXT});
    let secret = Secret::from_reply(&reply).expect("a created token");
    let mut shown = Some(Shown::new("script", "Created", secret));
    assert_eq!(shown.as_ref().map(Shown::reveal), Some(PLAINTEXT));

    // While it is on screen it is in *one* place, and nowhere a log line
    // could reach: `Secret` has no `Clone`, no `Display` and a `Debug` that
    // prints a length.
    let haystacks = [
        format!("{shown:?}"),
        present::token_made_toast("Created", "script"),
        present::TOKEN_SHOWN_ONCE.to_string(),
        format!("{state:?}"),
        serde_json::to_string(state.mirror()).unwrap(),
    ];
    for hay in &haystacks {
        assert!(
            !hay.contains(PLAINTEXT),
            "the plaintext escaped into: {hay}"
        );
    }

    // The sheet closes.
    assert!(integrations::close(&mut shown), "there was one to dismiss");
    assert!(shown.is_none());
    assert!(!integrations::close(&mut shown), "and only one");

    // And afterwards it is in nothing this process still holds. The mirror
    // is folded forward across a further event first: `token.*` emits none,
    // so nothing can put it there later either.
    state.handle_event(event(11, EventKind::UsageUpdated, json!({"usage": {}})));
    let after = [
        format!("{shown:?}"),
        format!("{state:?}"),
        serde_json::to_string(state.mirror()).unwrap(),
        // `token.list` is the only way back to a token, and it answers a
        // name and its scopes — the daemon keeps a hash and nothing else.
        format!(
            "{:?}",
            TokenRow::list(&json!([{"name": "script", "scopes": ["observe"]}]))
        ),
    ];
    for hay in &after {
        assert!(
            !hay.contains(PLAINTEXT),
            "the plaintext survived the sheet in: {hay}"
        );
    }
    // Nothing in the mirror grew a token-shaped field, either.
    let mirror = serde_json::to_string(state.mirror()).unwrap();
    assert!(!mirror.contains("\"token\""), "{mirror}");
}
