//! What the **Service** screen shows, driven through the real reducer and
//! the real reply shapes (§8.2's Service row).
//!
//! The row it is built to prove is the awkward one: three of this screen's
//! buttons call methods that are being added to the daemon in parallel
//! (contract §3), so half of these tests are about what the screen says when
//! a call comes back `-32601` — which must never read as the thing the
//! button was for.
use deck_client::ClientError;
use deck_gui_core::admin::{method_missing, ApiConfig, Info, Tail};
use deck_gui_core::{ApiEvent, AppEvent, ConfigValue, HostState, UiAction};
use deck_model::{ApiError, Event, EventKind, Principal, Snapshot, Ts};
use openwalkietalkie_app::present::{self, Launchd};
use serde_json::{json, Value};

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
    let mut snapshot = Snapshot {
        rev: 10,
        ..Snapshot::default()
    };
    snapshot.service.port = 47_201;
    snapshot.service.version = "0.1.0".into();
    let mut state = HostState::detached();
    state.handle_event(AppEvent::Api(ApiEvent::Connected { snapshot }));
    state
}

fn info_reply() -> Value {
    json!({
        "api": "1",
        "daemon": "0.1.0",
        "platform": "macos",
        "capabilities": ["device", "hooks.claude"],
        "uds": "/tmp/owt-gate/deckd.sock",
        "port": 47201,
        "mcp": null,
        "degraded": null,
        "listener": null,
        "sessions": 2
    })
}

#[test]
fn the_screen_says_whether_the_service_is_up_which_one_it_is_and_where_its_socket_is() {
    let state = live();
    let info = Info::parse(&info_reply());
    assert_eq!(info.daemon.as_deref(), Some("0.1.0"));
    assert_eq!(info.uds.as_deref(), Some("/tmp/owt-gate/deckd.sock"));
    assert_eq!(
        present::service_headline(state.connected(), &info),
        "Running \u{b7} deckd 0.1.0 \u{b7} 2 sessions"
    );
    // The socket dropping is what says the service has gone — not a reply.
    let mut gone = live();
    gone.handle_event(AppEvent::Api(ApiEvent::Disconnected));
    assert!(!gone.connected());
    assert!(present::service_headline(gone.connected(), &info).starts_with("Not running"));
    assert_eq!(UiAction::ServiceInfo.request().0, "service.info");
}

#[test]
fn the_port_banner_is_folded_from_the_daemons_own_degraded_event_and_offers_the_one_repair() {
    let mut state = live();
    assert_eq!(present::degraded_banner(&state.mirror().service), None);
    state.handle_event(event(
        11,
        EventKind::ServiceDegraded,
        json!({
            "reason": "hooks_hijack_risk",
            "port": 47201,
            "listener": {"pid": 812, "command": "node /usr/local/bin/other"}
        }),
    ));
    let banner = present::degraded_banner(&state.mirror().service).expect("a degraded service");
    assert!(banner.contains("47201"));
    assert!(
        banner.contains("Change the port"),
        "§4.1's one repair in v1"
    );
    // §4.1 makes `service.info` answer **no port** while the port is
    // somebody else's, and the screen must not print the configured one as
    // though it were held.
    let held = Info::parse(&json!({"port": null, "degraded": "hooks_hijack_risk"}));
    let config = ApiConfig {
        port: Some(47_201),
        ..ApiConfig::default()
    };
    let line = present::port_line(&held, &config);
    assert!(line.contains("not held"), "{line}");
    assert!(!line.contains("listening"), "{line}");
    // Recovered.
    state.handle_event(event(
        12,
        EventKind::ServiceDegraded,
        json!({"reason": null}),
    ));
    assert_eq!(present::degraded_banner(&state.mirror().service), None);
}

#[test]
fn changing_the_port_is_one_action_and_never_a_port_change_on_its_own() {
    // §8.2: "change port + re-install hooks". The pair is what keeps every
    // installed Claude hook from POSTing to a port the daemon has left.
    let (method, params) = UiAction::ConfigSet {
        key: "api.port".into(),
        value: ConfigValue::Int(47_202),
    }
    .request();
    assert_eq!(method, "config.set");
    // §4.11's wire shape is a patch, not a flat pair: the daemon refuses
    // `{key, value}` outright, so a bare pair here would be `-32602` and the
    // port would never move.
    assert_eq!(params["patch"]["api.port"], 47_202);
    assert!(params.get("key").is_none());
    let (repair, params) = UiAction::Integrations {
        verb: deck_gui_core::Verb::Repair,
        target: "claude".into(),
        trust: false,
        dry_run: false,
    }
    .request();
    assert_eq!(repair, "integrations.repair");
    assert_eq!(params["target"], "claude");
    assert_eq!(
        params["dry_run"], false,
        "the operator asked for the port change; the hooks follow it"
    );
}

#[test]
fn a_method_this_daemon_does_not_have_is_reported_as_missing_and_never_as_done() {
    let refused = |method: &str| ClientError::Rpc {
        code: ApiError::METHOD_NOT_FOUND,
        message: "method not found".into(),
        data: Some(json!({"method": method})),
    };
    // Contract §3's three, none of which is in the daemon's table yet.
    for method in ["config.set", "service.stop", "service.restart"] {
        let error = refused(method);
        assert!(method_missing(&error), "{method}");
        let line = present::missing_method_line(method);
        assert!(line.contains(method));
        assert!(line.contains("not in its method table"));
        assert!(line.contains("nothing happened"));
        assert!(
            !line.contains("stopped") && !line.contains("restarted"),
            "{line} reads as the thing the button was for"
        );
    }
    // Everything else is an ordinary failure and is said as one.
    for other in [
        ClientError::Rpc {
            code: ApiError::FORBIDDEN,
            message: "forbidden".into(),
            data: None,
        },
        ClientError::Rpc {
            code: ApiError::INVALID_PARAMS,
            message: "invalid params".into(),
            data: None,
        },
        ClientError::Disconnected,
    ] {
        assert!(!method_missing(&other), "{other:?}");
    }
    // The three are asked for by the names contract §3 gives them.
    assert_eq!(UiAction::ServiceStop.request().0, "service.stop");
    assert_eq!(UiAction::ServiceRestart.request().0, "service.restart");
    assert_eq!(
        UiAction::ConfigSet {
            key: "hooks.enabled".into(),
            value: ConfigValue::Bool(false),
        }
        .request()
        .0,
        "config.set"
    );
}

#[test]
fn the_door_shows_its_origins_and_a_curl_that_needs_a_token() {
    let config = ApiConfig::parse(&json!({
        "api": {
            "port": 47201,
            "allowed_origins": [],
            "uds_default_scope": "observe",
            "uds_max_scope": "admin",
            "max_held_hooks": 64
        }
    }));
    assert_eq!(config.port, Some(47_201));
    // An empty list is §4.1's default and a policy, not a gap.
    assert!(present::origins_line(&config.allowed_origins).contains("403"));
    let curl = present::curl_example(config.port);
    assert!(curl.contains("http://127.0.0.1:47201/v1/rpc"));
    assert!(curl.contains("Authorization: Bearer"));
    assert!(!curl.contains("--unix-socket"), "the socket is not HTTP");
    assert_eq!(UiAction::ConfigGet.request().0, "config.get");
}

#[test]
fn the_log_tail_filters_by_level_and_hides_content_when_the_toggle_is_on() {
    let reply = json!({
        "file": "/tmp/owt-gate/logs/deckd.log.2026-09-22",
        "lines": [
            "2026-09-22T10:00:00Z  INFO deckd: ready rev=7",
            "2026-09-22T10:00:01Z DEBUG deckd::core: card.set agent=ag_1 text=\"Bash: psql -f migrate.sql\"",
            "2026-09-22T10:00:02Z  WARN deckd::api: port_in_use port=47201"
        ]
    });
    let tail = Tail::parse(&reply);
    assert_eq!(tail.lines.len(), 3);
    assert!(tail.file.is_some());

    // The filter is the daemon's own substring test, so the local pass and
    // the server pass cannot disagree while a re-tail is in flight.
    let kept: Vec<&String> = tail
        .lines
        .iter()
        .filter(|l| present::log_level_matches(l, Some("WARN")))
        .collect();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].contains("port_in_use"));
    assert_eq!(
        tail.lines
            .iter()
            .filter(|l| present::log_level_matches(l, None))
            .count(),
        3
    );
    let (method, params) = UiAction::LogTail {
        lines: 200,
        level: Some("WARN".into()),
    }
    .request();
    assert_eq!(method, "log.tail");
    assert_eq!(params["level"], "WARN");

    // §3.7 says nothing at `info` or above carries content, so the toggle is
    // for the `debug` line below it.
    let masked = present::redact_line(&tail.lines[1], Some("/Users/tony"));
    assert!(!masked.contains("psql"), "{masked}");
    assert!(masked.contains("DEBUG") && masked.contains("card.set"));
    assert!(masked.contains("agent=ag_1"), "the ids stay readable");
    // An `info` line has nothing to mask, and comes back as itself.
    assert_eq!(
        present::redact_line(&tail.lines[0], Some("/Users/tony")),
        tail.lines[0]
    );
    // No log written yet is not an error.
    assert_eq!(
        Tail::parse(&json!({"file": null, "lines": []})),
        Tail::default()
    );
}

#[test]
fn launchds_view_of_the_agent_is_read_for_the_literals_it_prints_and_nothing_else() {
    assert_eq!(
        present::launchd_state("state = running\npid = 1234\n"),
        Launchd::Running(Some(1234))
    );
    assert!(present::launchd_line(&Launchd::Running(Some(1234))).contains("pid 1234"));
    assert_eq!(
        present::launchd_state("Could not find service \"x\" in domain for login"),
        Launchd::NotLoaded
    );
    assert_eq!(present::launchd_state("state = waiting\n"), Launchd::Loaded);
    // A `launchctl` that could not be run at all is this app's failure, and
    // must not be reported as "not registered".
    assert_eq!(present::launchd_state(""), Launchd::Unknown);
    // And the screen keeps the two objects apart: that line is the
    // *daemon's* launchd agent, and the login item under it is this
    // window's own (§10.2). An operator who read one as the other would
    // turn off the wrong one.
    assert!(present::LOGIN_ITEM_NOTE.contains("different thing"));
}
