//! Ghostty 1.3.1's AppleScript dictionary, as typed calls over
//! [`Scripting`] (HOST-CONTRACT-W5 §3).
//!
//! Every string in this file was run against the real application on
//! 2026-09-22. That matters more than usual, because the design's §5.5 and
//! the contract's §1 each get part of this dictionary wrong, and a wrong
//! verb here is not a compile error — it is a `-1700` or a `-1708` at the
//! moment an operator is holding the PTT button.
//!
//! What the run confirmed:
//!
//! * **M6 is right and the design is wrong.** After pinning tab A and
//!   pushing `ESC ]2;HOSTILE_RETITLE BEL` at its tty, one enumeration
//!   returned `terminal.name = HOSTILE_RETITLE`, `tab.name = deck:cd34`.
//!   The beacon lands on [`Surface::terminal_name`]; the pin lives on
//!   [`Surface::tab_name`]; [`Surface::window_name`] mirrored the *tab*
//!   title override and is read for nothing.
//! * **M3 is right.** `send key "return"` → `-1700 Unknown key name:
//!   return`. [`ENTER`] is the spelling that works.
//! * **M4 is right.** `input text "AIMED_PAYLOAD" to <unfocused terminal>`
//!   plus `send key "enter" to <it>` landed in the *aimed* surface while
//!   another window was frontmost. Injection never needs to steal focus.
//! * **M7 is right.** `set_tab_title:` with an empty value put both names
//!   back to the live title.
//! * **M11 is right, and not always.** `new tab with configuration` answered
//!   `-1708 "Can't continue new tab"` **and created the tab** (the window
//!   went from one tab to two). `new window with configuration` answered
//!   with a window id instead. So the rule is not "these commands return
//!   `-1708`" but "**nothing may depend on what these commands return**".
//!
//! And three corrections §1 does not have:
//!
//! * **`select <tab>` does not exist.** `-1708 "… doesn't understand the
//!   select message"`. `selected of <tab>` and `index of <window>` are
//!   read-only (`-10006`). §5.5's "`activate window` + `select tab`" cannot
//!   be written. What *does* move the selection is
//!   `perform action "goto_tab:<n>" on <terminal>` — measured moving a
//!   two-tab window from tab 2 to tab 1 and answering `true`. That is what
//!   [`focus`] does.
//! * **`selected of <tab>` is per window.** Both tabs of two different
//!   windows read `true` at once, because each is the selected tab *of its
//!   own window*. The focus verification therefore compares
//!   `id of (selected tab of <window>)` with `id of <tab>`, which is the
//!   handle §1 already names.
//! * **One `tell` reaches one instance.** `open -na Ghostty.app` gives every
//!   launch its own process; with three running, `tell application
//!   "Ghostty"` saw the surfaces of exactly one of them and answered
//!   `-1728` for a `terminal.id` belonging to another. A beacon in a second
//!   instance is invisible, which the resolver reports as
//!   [`super::errors::DesktopError::Ambiguous`] rather than guessing.
//!
//! Numbers and booleans are coerced with `as text` before they are
//! concatenated: without it AppleScript builds a *list* and the result
//! comes back as `1, ^_, tab-group-…` with commas in it (measured).
use super::errors::{DesktopError, ScriptError};
use super::script::{quote, Scripting};

/// The application, as AppleScript addresses it.
pub const APP: &str = "Ghostty";
/// …and as `NSWorkspace` does (§5.5's focus fallback).
pub const BUNDLE_ID: &str = "com.mitchellh.ghostty";
/// M3: the key name that exists. `"return"` is `-1700`.
pub const ENTER: &str = "enter";
/// The `perform action` verb that holds against OSC 2 (M8). Never
/// `set_surface_title`, which the next OSC 2 overwrites.
pub const PIN_ACTION: &str = "set_tab_title:";
/// The prefix every pin this daemon writes carries, so an enumeration can
/// tell our tab titles from the operator's own.
pub const PIN_PREFIX: &str = "deck:";

/// Field separator inside one enumerated record, and record separator
/// between them: ASCII US (31) and RS (30).
///
/// A tab or a newline would both be cheaper to write and wrong — a window
/// title is arbitrary text the *agent* controls through OSC 2, so a
/// separator a title could contain is a separator an agent could forge a
/// record with.
const FS: char = '\u{1f}';
const RS: char = '\u{1e}';

/// One Ghostty surface, with the three title strings M6 proved are three
/// strings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surface {
    /// `terminal.id`: a UUID, and the only safe map key (§5.5 step 4).
    pub id: String,
    /// `terminal.name` — where the **beacon** lands, and what the next
    /// OSC 2 overwrites.
    pub terminal_name: String,
    /// `tab.name` — where the **pin** lives, and what survives OSC 2.
    pub tab_name: String,
    /// `window.name` — read for nothing. Kept because a field that is
    /// collected and discarded is evidence for the next reader that it was
    /// considered, and M6 says it mirrors whichever tab is focused.
    pub window_name: String,
    pub window_id: String,
    pub tab_id: String,
    /// `tab.selected` — **per window**, not per application.
    pub selected: bool,
    /// `tab.index`, 1-based: the argument `goto_tab` takes.
    pub index: u32,
}

impl Surface {
    /// The `deck:<id4>` this surface is pinned with, if any.
    #[must_use]
    pub fn pin(&self) -> Option<&str> {
        self.tab_name.strip_prefix(PIN_PREFIX)
    }

    /// Whether this surface carries `label` as its **pin** (`tab.name`,
    /// M6): the re-resolution test of §5.5 step 5.
    #[must_use]
    pub fn pinned_with(&self, label: &str) -> bool {
        self.tab_name == label
    }

    /// Whether this surface carries `nonce` as its **beacon**
    /// (`terminal.name`, M6): the first-resolution test of §5.5 step 2.
    #[must_use]
    pub fn beacons(&self, nonce: &str) -> bool {
        self.terminal_name == nonce
    }
}

/// `deck:<id4>` — the pin label for an agent, from the last four
/// characters of its id (§5.5 step 3).
#[must_use]
pub fn pin_label(id4: &str) -> String {
    format!("{PIN_PREFIX}{id4}")
}

/// `ESC ]2; deck:<nonce> BEL` — what `SessionStart` answers as
/// `terminalSequence` (§5.5 step 1).
///
/// It is built here rather than in the adapter because the resolver has to
/// match what it produced, and one place that writes the beacon cannot
/// disagree with the place that reads it.
#[must_use]
pub fn beacon(nonce: &str) -> String {
    format!("\u{1b}]2;{PIN_PREFIX}{nonce}\u{7}")
}

/// …and the `terminal.name` that beacon leaves behind.
#[must_use]
pub fn beacon_name(nonce: &str) -> String {
    format!("{PIN_PREFIX}{nonce}")
}

/// A multi-statement script, wrapped in the `tell` block it needs.
///
/// `tell application "X" to <stmt>` accepts **one** statement (a
/// multi-line body after `to` dies with `-1708`), so everything with a
/// `repeat` in it goes through here.
fn tell(body: &str) -> String {
    format!("tell application \"{APP}\"\n{body}\nend tell\n")
}

/// The one-statement form, for the calls that are one statement.
fn tell_one(stmt: &str) -> String {
    format!("tell application \"{APP}\" to {stmt}\n")
}

/// `first terminal whose id is "<uuid>"` — the injection target form.
fn target(id: &str) -> String {
    format!("(first terminal whose id is {})", quote(id))
}

/// Every surface of the Ghostty instance this `tell` reaches, with all
/// three title strings (M6).
///
/// The **nested** walk and not the flat `terminals of application
/// "Ghostty"`: the flat form is real and cheaper, and it cannot carry
/// `tab.name`, which M6 makes mandatory. One round trip either way
/// (M12: ~118 ms), so the cheaper one buys nothing and loses the field the
/// resolver cannot work without.
pub fn enumerate(script: &dyn Scripting) -> Result<Vec<Surface>, ScriptError> {
    let src = tell(&format!(
        "set fs to character id {}
set rs to character id {}
set out to \"\"
repeat with w in windows
  set wn to name of w
  set wi to id of w
  repeat with tb in tabs of w
    set tn to name of tb
    set ti to id of tb
    set ts to (selected of tb) as text
    set tx to (index of tb) as text
    repeat with tm in terminals of tb
      set out to out & (id of tm) & fs & (name of tm) & fs & tn & fs & wn & fs & wi & fs & ti & \
         fs & ts & fs & tx & rs
    end repeat
  end repeat
end repeat
return out",
        FS as u32, RS as u32
    ));
    Ok(parse_surfaces(&script.run(&src)?))
}

/// The enumeration's wire format, as [`Surface`]s.
///
/// A record with the wrong number of fields is **skipped**, not guessed at:
/// the fields after the id are titles an agent can write, and a resolver
/// that reassembled a short record could be handed a `tab_name` the agent
/// chose.
#[must_use]
pub fn parse_surfaces(out: &str) -> Vec<Surface> {
    out.split(RS)
        .filter(|record| !record.trim().is_empty())
        .filter_map(|record| {
            let f: Vec<&str> = record.split(FS).collect();
            let [id, terminal_name, tab_name, window_name, window_id, tab_id, selected, index] =
                f.as_slice()
            else {
                return None;
            };
            Some(Surface {
                id: (*id).to_string(),
                terminal_name: (*terminal_name).to_string(),
                tab_name: (*tab_name).to_string(),
                window_name: (*window_name).to_string(),
                window_id: (*window_id).to_string(),
                tab_id: (*tab_id).to_string(),
                selected: *selected == "true",
                index: index.parse().unwrap_or(0),
            })
        })
        .collect()
}

/// Pin `id` with `deck:<id4>` (M5, M8): an **override** that the next OSC 2
/// cannot clear.
pub fn pin(script: &dyn Scripting, id: &str, id4: &str) -> Result<(), ScriptError> {
    let action = format!("{PIN_ACTION}{}", pin_label(id4));
    script.run(&tell_one(&format!(
        "perform action {} on {}",
        quote(&action),
        target(id)
    )))?;
    Ok(())
}

/// Put the live title back (M7): the **empty** value clears the override.
///
/// Every tab this daemon pins is unpinned — on session end, on uninstall
/// and at shutdown — because §6's rule is to leave the operator's Ghostty
/// as we found it, and a `deck:ab12` left on a tab after the daemon is gone
/// is a title they cannot get rid of without knowing about us.
pub fn unpin(script: &dyn Scripting, id: &str) -> Result<(), ScriptError> {
    script.run(&tell_one(&format!(
        "perform action {} on {}",
        quote(PIN_ACTION),
        target(id)
    )))?;
    Ok(())
}

/// `input text <text> to <terminal>` (M4). Literal text; never a shell.
pub fn input_text(script: &dyn Scripting, id: &str, text: &str) -> Result<(), ScriptError> {
    script.run(&tell_one(&format!(
        "input text {} to {}",
        quote(text),
        target(id)
    )))?;
    Ok(())
}

/// `send key <name> to <terminal>` (M3, M4). The caller passes [`ENTER`].
pub fn send_key(script: &dyn Scripting, id: &str, key: &str) -> Result<(), ScriptError> {
    script.run(&tell_one(&format!(
        "send key {} to {}",
        quote(key),
        target(id)
    )))?;
    Ok(())
}

/// Bring `id`'s surface to the front (`op.focus`, device `channel`).
///
/// Three steps and a verification, in the order the dictionary allows:
/// `activate` the application (standard suite), `goto_tab:<index>` on the
/// target terminal to move its window's selection, then read
/// `id of (selected tab of w)` back. `Ok(true)` means the selected tab is
/// the target's; `Ok(false)` means the script ran and it is not (the
/// operator moved something under us, or Ghostty is on another Space).
/// Failures are silent on the device — the `focus{ack}` already went out —
/// and visible in the GUI (§5.5).
pub fn focus(script: &dyn Scripting, id: &str) -> Result<bool, ScriptError> {
    let src = tell(&format!(
        "activate
set out to \"\"
repeat with w in windows
  repeat with tb in tabs of w
    repeat with tm in terminals of tb
      if (id of tm) is {id} then
        try
          perform action (\"{PIN_ACTION_GOTO}\" & ((index of tb) as text)) on tm
        end try
        if (id of (selected tab of w)) is (id of tb) then set out to \"selected\"
      end if
    end repeat
  end repeat
end repeat
return out",
        id = quote(id),
        PIN_ACTION_GOTO = "goto_tab:"
    ));
    Ok(script.run(&src)?.trim() == "selected")
}

/// Is Ghostty scriptable at all, and how many windows does it have?
///
/// The benign probe M13 used: `version & (count of windows)` returned in
/// band with no prompt on a machine whose Automation grant was already
/// given. It is the one call that is safe to make to answer
/// `perm.status.automation.ghostty`, because it reads nothing an operator
/// would mind and writes nothing at all.
pub fn probe(script: &dyn Scripting) -> Result<(String, u32), ScriptError> {
    let out = script.run(&tell(&format!(
        "return (version as text) & (character id {}) & ((count of windows) as text)",
        FS as u32
    )))?;
    let (version, windows) = out.trim().split_once(FS).unwrap_or((out.trim(), "0"));
    Ok((version.to_string(), windows.parse().unwrap_or(0)))
}

/// Activate Ghostty through `NSWorkspace`, by bundle id (§5.5's fallback
/// for when `activate` itself could not be sent).
///
/// This is not an Apple Event and takes no Automation grant: it is the same
/// call `open -b com.mitchellh.ghostty` makes, in process. It can raise the
/// application and nothing finer — no window, no tab — which is exactly why
/// it is the fallback and not the path.
#[cfg(target_os = "macos")]
pub fn activate_by_bundle_id() -> Result<(), DesktopError> {
    use objc2_app_kit::{NSApplicationActivationOptions, NSWorkspace};
    use objc2_foundation::NSString;

    // Every call here is safe in objc2's bindings: three reads of AppKit's
    // own singleton and its running-app list, and one `activate` on a
    // member of that list. Nothing is retained past the call.
    let workspace = NSWorkspace::sharedWorkspace();
    let wanted = NSString::from_str(BUNDLE_ID);
    let raised = workspace
        .runningApplications()
        .iter()
        .find(|app| app.bundleIdentifier().is_some_and(|id| *id == *wanted))
        .is_some_and(|app| {
            app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows)
        });
    if raised {
        return Ok(());
    }
    // Not running: there is nothing to raise, and `op.focus` has no
    // business launching a terminal the operator closed.
    Err(DesktopError::Script(ScriptError::Osa {
        number: super::errors::ERR_NO_PROCESS,
        message: format!("{BUNDLE_ID} is not running"),
    }))
}

/// §13: no AppKit, no fallback.
#[cfg(not(target_os = "macos"))]
pub fn activate_by_bundle_id() -> Result<(), DesktopError> {
    Err(DesktopError::Script(ScriptError::Unavailable(
        "NSWorkspace is macOS-only".into(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::fakes::ScriptLog;

    /// The exact bytes the enumeration answered on 2026-09-22, before and
    /// after a hostile OSC 2 — M6, as a fixture.
    ///
    /// Two records: the surface the beacon and then the pin were written
    /// on, whose `terminal.name` the OSC 2 has taken over, and a second
    /// window's surface. This string is the resolver's whole world in
    /// every test below.
    fn measured_enumeration() -> String {
        [
            // id, terminal.name, tab.name, window.name, window.id, tab.id, selected, index
            "69BE9F52-9890-44C2-AF58-21E7E307A3B0\u{1f}HOSTILE_RETITLE\u{1f}deck:cd34\u{1f}\
             deck:cd34\u{1f}tab-group-7ac3a81ea0\u{1f}tab-7ac2ae0e00\u{1f}true\u{1f}1",
            "61942837-7B64-41AE-AAC7-56AA0C6FD97E\u{1f}👻\u{1f}👻\u{1f}👻\u{1f}\
             tab-group-7ac3eeabc0\u{1f}tab-7ac2376200\u{1f}true\u{1f}1",
        ]
        .join("\u{1e}")
            + "\u{1e}"
    }

    /// M6's headline, as a parse: the beacon is on one field and the pin is
    /// on another, and a resolver reading either one alone is wrong half
    /// the time.
    #[test]
    fn the_enumeration_carries_three_separate_title_strings_per_surface() {
        let surfaces = parse_surfaces(&measured_enumeration());
        assert_eq!(surfaces.len(), 2);
        let pinned = &surfaces[0];
        assert_eq!(pinned.id, "69BE9F52-9890-44C2-AF58-21E7E307A3B0");
        assert_eq!(pinned.terminal_name, "HOSTILE_RETITLE");
        assert_eq!(pinned.tab_name, "deck:cd34");
        assert_eq!(pinned.window_name, "deck:cd34");
        assert_eq!(pinned.index, 1);
        assert!(pinned.selected);
        // The pin is found on `tab.name` …
        assert!(pinned.pinned_with("deck:cd34"));
        assert_eq!(pinned.pin(), Some("cd34"));
        // … and the beacon it replaced is gone from `terminal.name`, which
        // is the fact a `terminal.name`-only re-resolver would trip over.
        assert!(!pinned.beacons("deck:cd34"));
        assert!(pinned.beacons("HOSTILE_RETITLE"));
        // A surface with no pin says so rather than offering its title.
        assert_eq!(surfaces[1].pin(), None);
        assert_eq!(surfaces[1].terminal_name, "👻");
    }

    /// The separators are control characters because a title is attacker
    /// text: a record built with a tab or a newline could be forged from
    /// inside a terminal by writing one OSC 2.
    #[test]
    fn a_record_an_agent_could_forge_from_a_title_is_dropped_not_reassembled() {
        // A title containing tabs, newlines and commas parses as one field.
        let hostile =
            "ID\u{1f}a\tb\nc, d\u{1f}deck:aaaa\u{1f}w\u{1f}wid\u{1f}tid\u{1f}false\u{1f}2\u{1e}";
        let surfaces = parse_surfaces(hostile);
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0].terminal_name, "a\tb\nc, d");
        assert_eq!(surfaces[0].pin(), Some("aaaa"));
        assert!(!surfaces[0].selected);
        assert_eq!(surfaces[0].index, 2);
        // A short record — the only way a title *can* affect the field
        // count, by carrying a US of its own — is skipped whole.
        let short = "ID\u{1f}only-two\u{1e}";
        assert!(parse_surfaces(short).is_empty());
        // As is a long one.
        let long = "ID\u{1f}a\u{1f}b\u{1f}c\u{1f}d\u{1f}e\u{1f}f\u{1f}1\u{1f}extra\u{1e}";
        assert!(parse_surfaces(long).is_empty());
        // Empty output is no surfaces and not an error: Ghostty with no
        // windows answered exactly this on 2026-09-22.
        assert!(parse_surfaces("").is_empty());
        assert!(parse_surfaces("\u{1e}").is_empty());
    }

    /// M3 and M4 in the generated source, because this is the one place a
    /// typo is a runtime `-1700` at the worst possible moment.
    #[test]
    fn the_generated_injection_uses_enter_and_the_to_parameter() {
        let log = ScriptLog::new();
        let id = "69BE9F52-9890-44C2-AF58-21E7E307A3B0";
        input_text(&log, id, "ship it").unwrap();
        send_key(&log, id, ENTER).unwrap();
        let sources = log.sources();
        assert_eq!(
            sources[0],
            "tell application \"Ghostty\" to input text \"ship it\" to (first terminal whose id \
             is \"69BE9F52-9890-44C2-AF58-21E7E307A3B0\")\n"
        );
        assert_eq!(
            sources[1],
            "tell application \"Ghostty\" to send key \"enter\" to (first terminal whose id is \
             \"69BE9F52-9890-44C2-AF58-21E7E307A3B0\")\n"
        );
        // M3: the word that does not exist must not appear anywhere.
        assert!(!sources.iter().any(|s| s.contains("\"return\"")));
        // M4: `to`, never `on`, for these two verbs.
        assert!(sources.iter().all(|s| !s.contains(" on (first terminal")));
        // A quote in the order text cannot end the literal early.
        input_text(&log, id, "say \"hi\"").unwrap();
        assert!(log.sources()[2].contains("input text \"say \\\"hi\\\"\" to"));
    }

    /// M5 and M8 for the pin, M7 for the unpin: `set_tab_title`, `on`, and
    /// the empty value to clear.
    #[test]
    fn the_pin_uses_set_tab_title_on_the_terminal_and_the_empty_value_unpins() {
        let log = ScriptLog::new();
        pin(&log, "UUID-1", "cd34").unwrap();
        unpin(&log, "UUID-1").unwrap();
        let sources = log.sources();
        assert_eq!(
            sources[0],
            "tell application \"Ghostty\" to perform action \"set_tab_title:deck:cd34\" on (first \
             terminal whose id is \"UUID-1\")\n"
        );
        assert_eq!(
            sources[1],
            "tell application \"Ghostty\" to perform action \"set_tab_title:\" on (first terminal \
             whose id is \"UUID-1\")\n"
        );
        // M8: never the action the next OSC 2 overwrites.
        assert!(!sources.iter().any(|s| s.contains("set_surface_title")));
        // M5: `on`, never `to`, for `perform action`.
        assert!(sources.iter().all(|s| !s.contains("\" to (first terminal")));
        assert_eq!(pin_label("cd34"), "deck:cd34");
    }

    /// The beacon the adapter writes and the `terminal.name` the resolver
    /// looks for are one fact, built in one place.
    #[test]
    fn the_beacon_sequence_and_the_name_it_leaves_behind_agree() {
        assert_eq!(beacon("cd34"), "\u{1b}]2;deck:cd34\u{7}");
        assert_eq!(beacon_name("cd34"), "deck:cd34");
        let surface = Surface {
            terminal_name: beacon_name("cd34"),
            ..Surface::default()
        };
        assert!(surface.beacons(&beacon_name("cd34")));
        // …and the beacon is *not* a pin: before the pin is written,
        // `tab.name` happens to equal it (M7 explains why the design
        // thought they were one string), but nothing keys off that.
        assert!(!surface.pinned_with(&beacon_name("cd34")));
    }

    /// Focus does what the dictionary allows and verifies what §1 says to
    /// verify. `select` is absent from the source because `select` is
    /// absent from Ghostty (measured `-1708`).
    #[test]
    fn focus_activates_goes_to_the_tab_and_verifies_the_selected_tab() {
        let log = ScriptLog::new().answering("selected");
        assert!(focus(&log, "UUID-1").unwrap());
        let src = &log.sources()[0];
        assert!(
            src.starts_with("tell application \"Ghostty\"\nactivate\n"),
            "{src}"
        );
        assert!(src.contains("perform action (\"goto_tab:\" & ((index of tb) as text)) on tm"));
        assert!(src.contains("if (id of (selected tab of w)) is (id of tb)"));
        assert!(
            !src.contains("select tb"),
            "`select` is not a Ghostty command"
        );
        assert!(!src.contains("set selected of"), "`selected` is read-only");
        assert!(!src.contains("set index of"), "`index` is read-only");
        // An empty answer is "the script ran and the tab is not selected",
        // which is a silent failure on the device and a visible one in the
        // GUI — not an error.
        let missed = ScriptLog::new().answering("");
        assert!(!focus(&missed, "UUID-1").unwrap());
    }

    /// M13's benign probe, which is also `perm.status`'s only honest
    /// source for `automation.ghostty`.
    #[test]
    fn the_benign_probe_reads_the_version_and_the_window_count_in_one_trip() {
        let log = ScriptLog::new().answering("1.3.1\u{1f}2");
        assert_eq!(probe(&log).unwrap(), ("1.3.1".into(), 2));
        assert_eq!(log.sources().len(), 1, "one round trip (M12: ~118 ms)");
        // Ghostty with no windows: measured `0` on 2026-09-22, and not an
        // error — "scriptable with nothing open" is a real state.
        let empty = ScriptLog::new().answering("1.3.1\u{1f}0");
        assert_eq!(empty.name(), "script-log");
        assert_eq!(probe(&empty).unwrap().1, 0);
    }

    /// Enumeration is one round trip and reads `tab.name`, which is the
    /// whole reason it is not the cheaper flat form.
    #[test]
    fn the_enumeration_is_one_round_trip_and_reads_the_field_the_flat_form_cannot() {
        let log = ScriptLog::new().answering(&measured_enumeration());
        let surfaces = enumerate(&log).unwrap();
        assert_eq!(surfaces.len(), 2);
        assert_eq!(log.sources().len(), 1);
        let src = &log.sources()[0];
        assert!(src.contains("name of tb"), "tab.name is mandatory (M6)");
        assert!(
            src.contains("name of tm"),
            "terminal.name is mandatory (M6)"
        );
        assert!(
            !src.contains("terminals of application"),
            "the flat form cannot carry tab.name"
        );
        // `as text` on every number and boolean: without it AppleScript
        // builds a list and the answer comes back with commas in it.
        assert!(src.contains("(selected of tb) as text"));
        assert!(src.contains("(index of tb) as text"));
    }
}
