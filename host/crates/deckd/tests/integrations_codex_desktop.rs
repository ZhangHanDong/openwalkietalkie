//! Desktop installation and launch behavior, entirely inside a temp HOME.
//! The launch test substitutes fake open/osascript/Codex executables; it
//! never opens or quits the user's app, or starts their app-server daemon.
#![cfg(target_os = "macos")]

use deckd::integrations::{self, codex_desktop, testdir::TestDir, Status, Target, Verb};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const INSTALL: Verb = Verb::Install { trust: false };

fn executable(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn desktop_install_is_independent_idempotent_and_reversible() {
    let dir = TestDir::new("desktop-install");
    let env = integrations::test_env(&dir);
    let hooks = integrations::codex::hooks_path(&env);
    std::fs::write(&hooks, b"{\"hooks\":{}}\n").unwrap();
    let before = std::fs::read(&hooks).unwrap();
    let app = codex_desktop::launcher(&env);
    let preview = integrations::plan(&env, Target::CodexDesktop, INSTALL).unwrap();
    assert_eq!(preview.changes().len(), 2);
    assert!(!app.exists(), "dry-run writes nothing");
    let first = integrations::run(&env, Target::CodexDesktop, INSTALL).unwrap();
    assert_eq!(first.changed.len(), 2);
    assert_eq!(codex_desktop::status(&env).status, Status::Installed);
    let script = app.join("Contents/MacOS/launch-codex");
    assert_eq!(
        std::fs::metadata(&script).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(std::process::Command::new("/bin/sh")
        .arg("-n")
        .arg(&script)
        .status()
        .unwrap()
        .success());
    assert!(std::process::Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(app.join("Contents/Info.plist"))
        .output()
        .unwrap()
        .status
        .success());
    assert!(integrations::run(&env, Target::CodexDesktop, INSTALL)
        .unwrap()
        .changed
        .is_empty());
    integrations::run(&env, Target::CodexDesktop, Verb::Uninstall).unwrap();
    assert!(!app.exists());
    assert_eq!(codex_desktop::status(&env).status, Status::Absent);
    assert_eq!(std::fs::read(&hooks).unwrap(), before);
    assert!(
        integrations::run(&env, Target::CodexDesktop, Verb::Uninstall)
            .unwrap()
            .changed
            .is_empty()
    );
}

#[test]
fn modified_launchers_are_preserved_and_partial_installs_are_repaired() {
    let dir = TestDir::new("desktop-repair");
    let env = integrations::test_env(&dir);
    integrations::run(&env, Target::CodexDesktop, INSTALL).unwrap();
    let app = codex_desktop::launcher(&env);
    let plist = app.join("Contents/Info.plist");
    std::fs::remove_file(&plist).unwrap();
    assert_eq!(codex_desktop::status(&env).status, Status::Outdated);
    integrations::run(&env, Target::CodexDesktop, Verb::Repair { trust: false }).unwrap();
    std::fs::write(&plist, b"my own application").unwrap();
    assert_eq!(codex_desktop::status(&env).status, Status::Conflict);
    for verb in [INSTALL, Verb::Uninstall] {
        assert!(integrations::run(&env, Target::CodexDesktop, verb).is_err());
        assert_eq!(std::fs::read(&plist).unwrap(), b"my own application");
        assert!(app.join("Contents/MacOS/launch-codex").exists());
    }
}

#[test]
fn launcher_uses_bundled_cli_and_keeps_home_literal_and_existing_sessions_alive() {
    let dir = TestDir::new("desktop-launch");
    let mut env = integrations::test_env(&dir);
    // Both apostrophes and shell substitutions are literal path characters.
    env.home = dir.sub("home with ' quotes $(touch INJECTED)");
    env.codex_home = Some(dir.sub("codex with ' quotes $(touch INJECTED)"));
    integrations::run(&env, Target::CodexDesktop, INSTALL).unwrap();
    let fake_app = env.home.join("Applications/Codex.app");
    let log = dir.path().join("calls");
    let osa = dir.path().join("osascript");
    let open = dir.path().join("open");
    executable(
        &osa,
        "#!/bin/sh\nif [ \"$1\" = -e ]; then printf '%s\\n' \"${FAKE_RUNNING:-false}\"; fi\n",
    );
    executable(
        &open,
        "#!/bin/sh\nprintf 'open\\n' >> \"$CALL_LOG\"\nprintf '%s\\n' \"$@\" >> \"$CALL_LOG\"\n",
    );
    executable(&fake_app.join("Contents/Resources/codex-cli/bin/codex"),
        "#!/bin/sh\nprintf 'codex\\n%s\\n' \"$CODEX_HOME\" >> \"$CALL_LOG\"\nprintf '%s\\n' \"$@\" >> \"$CALL_LOG\"\nexit \"${FAKE_START_EXIT:-0}\"\n");
    let archive = fake_app.join("Contents/Resources/app.asar");
    std::fs::write(&archive, "CODEX_APP_SERVER_USE_LOCAL_DAEMON").unwrap();
    let script_path = codex_desktop::launcher(&env).join("Contents/MacOS/launch-codex");
    let script = std::fs::read_to_string(script_path)
        .unwrap()
        .replace("/usr/bin/osascript", &osa.display().to_string())
        .replace("/usr/bin/open", &open.display().to_string());
    let run = dir.write("launch.sh", &script);
    let command = || {
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg(&run).env("CALL_LOG", &log).current_dir(dir.path());
        cmd
    };
    assert!(command().output().unwrap().status.success());
    let calls = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<_> = calls.lines().collect();
    assert_eq!(
        &lines[..8],
        &[
            "codex",
            env.codex_dir().to_str().unwrap(),
            "-c",
            "features.code_mode_host=true",
            "app-server",
            "daemon",
            "start",
            "open"
        ]
    );
    assert!(calls.contains("CODEX_APP_SERVER_USE_LOCAL_DAEMON=1\n"));
    assert!(calls.contains(&format!("CODEX_HOME={}\n", env.codex_dir().display())));
    assert!(!dir.path().join("INJECTED").exists());
    std::fs::remove_file(&log).unwrap();
    assert!(!command()
        .env("FAKE_RUNNING", "true")
        .output()
        .unwrap()
        .status
        .success());
    assert!(!log.exists(), "an already running desktop is left alone");
    assert!(!command()
        .env("FAKE_START_EXIT", "1")
        .output()
        .unwrap()
        .status
        .success());
    assert!(
        !std::fs::read_to_string(&log).unwrap().contains("open\n"),
        "do not launch when daemon start fails"
    );
    std::fs::remove_file(&log).unwrap();
    std::fs::write(&archive, "older desktop").unwrap();
    assert!(!command().output().unwrap().status.success());
    assert!(
        !log.exists(),
        "an unsupported desktop must not start a daemon or launch"
    );
}
