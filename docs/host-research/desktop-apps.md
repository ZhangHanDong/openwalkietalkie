# desktop-apps

DATE CHECKED: 2026-09-18 (macOS 27.0 build 26A428 on this Mac). Local primary evidence: /Applications/Claude.app 1.49585.0 (Electron 44.2.0 / Chrome 152.0.7977.76, bundle com.anthropic.claudefordesktop, URL scheme `claude`, NO AppleScript dictionary) and /Applications/ChatGPT.app 26.908.70816 (bundle com.openai.codex = the unified Codex/ChatGPT Electron app: "Codex Framework.framework" = Chromium 152.0.7977.83, app.asar 322 MB, URL schemes `codex`,`http`,`https`; ships Chromium's stock scripting.sdef; no Metadata.appintents in either app).

== INTEGRATION SURFACES ==
CLAUDE DESKTOP
1. Local stdio MCP servers: ~/Library/Application Support/Claude/claude_desktop_config.json {"mcpServers":{name:{command,args,env}}}, restart required, logs ~/Library/Logs/Claude/mcp*.log; or a one-click .mcpb Desktop Extension (Settings > Extensions). Tools fully supported; resources/prompts limited; sampling NOT supported; elicitation: reported unsupported (-32601) and GH #41110 closed 2026-03-31 "completed" with zero comments -> UNVERIFIED, treat as absent. MCP Apps (io.modelcontextprotocol/ui) supported. Semantics that bound the design: a server only answers tool calls the model chooses to make (no push, no starting a turn), each call is approval-gated in the UI, and the server has no access to the chat transcript. => roster/reply/question for Chat & Cowork modes are SELF-REPORTED by the model via our tools, never observed.
2. Claude Code sessions inside Claude Desktop ("Code" mode): official docs state hooks fire identically in the Desktop app; Notification hook types include agent_needs_input, agent_completed, permission_prompt, idle_prompt, elicitation_dialog; Stop/UserPromptSubmit/Elicitation hooks exist. => Code-mode sessions are first-class roster entries via the same hook path as Claude Code CLI. This is the ONLY fully reliable state source for Claude Desktop.
3. Deep links (official): claude://claude.ai/new?q=<text> prefills a NEW chat (no auto-submit, ~14,000-char cap); claude://claude.ai/chat/{uuid}; claude://code/new?q=&folder=; claude://cowork/new?q=&folder=&file=. No link targets the composer of an existing chat with text.
4. Quick Entry: global hotkey (default double-tap Option; configurable to Option+Space), opens a text box for a new chat; app may stay in background.
5. Accessibility: Electron builds its AX tree only when an AT is detected; a third party can force it via AXUIElementSetAttributeValue(app,"AXManualAccessibility",true). Community tool claudectl (eldrgeek/mac-controller) reads mode/sessions/composer/tasks and fills+submits prompts, but documents "builds its accessibility tree only while it is the front app" and "a Claude Desktop update can break it".
6. No AppleScript dictionary, no App Intents/Shortcuts on macOS (Claude app intents are iOS-only).

CHATGPT DESKTOP
1. MCP: remote-only. Official help (updated ~Aug 2026): "ChatGPT connects to remote MCP servers... use Secure MCP Tunnel" for servers on a developer machine; full (write) MCP only on Business/Enterprise/Edu, Pro = read/fetch-only in developer mode; developer mode gated by admin/policy. Secure MCP Tunnel = `tunnel-client` on the Mac polling OpenAI over outbound HTTPS, wraps a stdio (`--mcp-command`) or HTTP MCP server; needs a Platform org, tunnel_id, runtime API key. => every deck tool call round-trips through OpenAI's cloud, is plan/policy-gated, and remains model-initiated. Same self-reporting limits as Claude, plus cloud dependency.
2. "Work with Apps" is ChatGPT READING other apps (Terminal/iTerm/Warp/Prompt, IDEs, Notes...) through the Accessibility API; editing only in IDEs; no third-party API. Not an inbound surface for us.
3. Chat bar: Option+Space (Alt+Space globalShortcut confirmed in app.asar), changeable in Settings > Keyboard Shortcut; companion window; advanced voice not launchable from companion.
4. URL scheme: `codex://` routes embedded in app.asar: threads/, threads/new, review?pr=&path=&line=&side=, space, settings/connections, plugins/, launch, shared-thread. No documented prompt parameter; `chatgpt://` scheme is NOT registered by this app (only one string chatgpt://safety-faster-model). Old native app (com.openai.chat) absent on this Mac.
5. AppleScript: NSAppleScriptEnabled=1 with Chromium's stock sdef (windows/tabs, `execute javascript`), and pref string browser.allow_javascript_apple_events present — but in Electron the Chromium AppleScript classes are almost certainly not wired (BrowserCrApplication appears 2x as a string only) and Chromium gates JS-from-AppleEvents behind a View>Developer toggle. UNVERIFIED (testing triggers an Automation TCC prompt); treat as unavailable.
6. Accessibility: same Electron/AXManualAccessibility story; chat is a web DOM -> brittle selectors.
7. Codex threads inside ChatGPT.app run a private `codex app-server` child (strings "app-server", "app-server is not available"); not externally attachable. Codex participation belongs to the Codex CLI/app-server topic, not this one.

== macOS GENERIC MECHANISMS (both apps) ==
- Frontmost/running app: NSWorkspace.shared.frontmostApplication / runningApplications + didActivateApplicationNotification — no TCC. Crates: objc2-app-kit 0.3.2, objc2 0.6.4.
- Window titles: CGWindowListCopyWindowInfo omits kCGWindowName without Screen Recording permission (since 10.15) — avoid; use AX (kAXWindows/kAXTitle) under Accessibility instead. Crates: core-graphics 0.25.0, accessibility/accessibility-sys 0.2.0, cidre 0.29.0.
- AX read/observe (AXObserver on kAXValueChanged/kAXFocusedUIElementChanged): needs Accessibility; Electron tree only after AXManualAccessibility; DOM-derived, changes per release.
- Keystrokes: CGEventPost needs Accessibility only; `System Events` keystroke needs Automation (Apple Events) + Accessibility and an embedded Info.plist with NSAppleEventsUsageDescription (+ com.apple.security.automation.apple-events entitlement under hardened runtime) for a CLI/daemon.
- Pasteboard: writing is free; programmatic READS will raise Apple's Pasteboard Privacy alert once enforced (announced 15.4 preview, accessBehavior API; not default-enforced as of mid-2026) — a save/restore-clipboard paste strategy reads the pasteboard, so prefer AX value-set or typed text.
- Notifications: no API to observe another app's UNUserNotifications; DistributedNotificationCenter only if the app posts (neither documented to); Notification Center SQLite (~/Library/Group Containers/group.com.apple.usernoted/db2/db, TCC/Full-Disk-Access protected) is a polling forensic hack — do not build on it.
- TCC & packaging: grants are keyed to code signature+path; ad-hoc/rebuilt binaries re-prompt; a launchd LaunchAgent gets the grant on the executable the plist names (appears by binary name), a process spawned from Terminal gets attributed to Terminal (responsible process); an MCP stdio child gets attributed to the MCP client (Claude/ChatGPT) — so a deck MCP server must NOT itself do AX/keystrokes; it must proxy to the daemon. Grants require a logged-in GUI session (fine for a per-user LaunchAgent, not for SSH). Today's host already installs a per-user LaunchAgent (host/README.md:16,75; hidapi 2.6.6).

== FEASIBILITY MATRIX (capability x app x mechanism -> robustness) ==
ROSTER STATE (working/question/ready)
- Claude Desktop Code mode / hooks -> HIGH (official, event-driven).
- Claude Desktop Chat/Cowork / our MCP tools (report_status, ask_operator, done) -> MEDIUM-LOW: self-reported, model-initiated, approval-gated, no "working" signal, nothing if model forgets.
- Claude Desktop / AX observation -> LOW (front-app only, DOM-fragile).
- ChatGPT / MCP via Secure MCP Tunnel -> LOW-MEDIUM: same self-reporting limits + cloud round-trip + Business/Enterprise plan or Pro read-only + Platform API key.
- ChatGPT / AX -> LOW. ChatGPT / notifications -> NOT FEASIBLE cleanly.
PTT TARGET (speech -> text in composer)
- Claude Desktop / claude://claude.ai/new?q= -> HIGH for "new chat with this order" (no TCC, prefill only; user presses Enter) ; cannot append to current chat.
- Claude Desktop / AX set composer value + Return (Accessibility) -> MEDIUM (must activate app; Electron tree; selectors may break).
- Claude Desktop / pasteboard+Cmd-V+Return -> MEDIUM (clipboard clobber, future read alert).
- Claude Desktop / MCP ask_operator blocking tool returning the transcript as the tool result -> HIGH while a question is pending (this is the clean path for "answer" ctx), N/A for unsolicited orders.
- ChatGPT / codex:// -> UNVERIFIED/LOW (no prompt param found). ChatGPT / Option+Space + typed keystrokes -> MEDIUM-LOW (new chat only). ChatGPT / AX or paste -> MEDIUM. ChatGPT / MCP blocking tool via tunnel -> MEDIUM-LOW (cloud).
REPLY TEXT FOR TTS
- Either app / our MCP tool `say`/`reply` the model calls when done -> MEDIUM (compliance + approvals; ChatGPT additionally cloud/plan-gated).
- Either app / AX read of last message -> LOW-MEDIUM. Copy-button + pasteboard read -> LOW (pasteboard privacy).
API CLIENT OF OUR SERVICE
- Claude Desktop: ship `deck-mcp` (Rust, stdio, distributable as .mcpb) that connects to the daemon's local API and exposes report_status / ask_operator (blocks until `answer`/PTT or timeout, mapped to `card type:q`) / say (drives TTS + `reply`). HIGH feasibility, MEDIUM fidelity.
- ChatGPT: the same server behind `tunnel-client --mcp-command` or a public HTTPS+OAuth endpoint. LOW-MEDIUM (plan, cloud, ops burden).

== RECOMMENDATION FOR v1 ==
1. Treat Claude Desktop Code-mode sessions as ordinary Claude Code agents (hooks) — reliable, zero extra work beyond the CLI integration.
2. Add Claude Desktop Chat via a shipped stdio MCP server with three tools; show such rows as kind "claude" with a "self-reported" badge; question cards come only from ask_operator (blocking tool result carries the deck answer or PTT text); reply/TTS only from `say`. Include a pasteable Project/system-prompt snippet instructing the model to call them. Do not attempt "working" detection.
3. PTT to Claude Desktop Chat when no card is pending: open `claude://claude.ai/new?q=<transcript>` (new chat, user confirms with Enter). Ship AX/paste injection into the current chat only as an opt-in "experimental" behind the Accessibility grant.
4. ChatGPT desktop: OUT of v1 for roster/reply (no local MCP; tunnel needs Business/Enterprise or Pro-read-only + Platform key + cloud hop). Offer only the generic "frontmost app" PTT injection (Accessibility) as a best-effort target with no state. Revisit when OpenAI ships local MCP for the desktop app.
5. Never build on AX scraping, window titles via CGWindowList, notification DB, or pasteboard reads for state.
6. Package the daemon as a Developer-ID-signed, stable-identifier app bundle (LSUIElement, embedded NSAppleEventsUsageDescription, registered via SMAppService/LaunchAgent) so Accessibility/Automation grants persist across builds; the gpui GUI and the MCP server are separate clients of the daemon's local API and must not themselves touch AX/keystrokes (their TCC identity would be Claude/ChatGPT or the GUI).

## Facts
- [verified] Claude.app on this Mac is version 1.49585.0, bundle com.anthropic.claudefordesktop, Electron Framework present, Chrome/152.0.7977.76 Electron/44.2.0, CFBundleURLSchemes = [claude, msauth.com.anthropic.claudefordesktop]; no NSAppleScriptEnabled, no OSAScriptingDefinition, no .sdef, no Metadata.appintents.
  - source: /Applications/Claude.app/Contents/Info.plist and Frameworks/ inspected via defaults/plutil/strings on 2026-09-18
- [verified] ChatGPT.app on this Mac is version 26.908.70816 (build 9275), bundle com.openai.codex, min macOS 13.0, an Electron app (Resources/app.asar 322,016,890 bytes, electron.asar/ELECTRON_RUN_AS_NODE strings, default_app, electron.icns) whose 'Codex Framework.framework' is Chromium 152.0.7977.83 with helpers 'Codex (Renderer).app' etc.; URL schemes [codex, http, https]; NSAppleScriptEnabled=1 with Resources/scripting.sdef (Chromium Standard + Chromium Suite incl. 'execute javascript'); links UserNotifications.framework; contains strings AXEnhancedUserInterface and browser.allow_javascript_apple_events; no Metadata.appintents.
  - source: /Applications/ChatGPT.app/Contents/{Info.plist,Resources,Frameworks} inspected via defaults/plutil/otool/strings/grep on 2026-09-18
- [verified] codex:// routes embedded in ChatGPT.app app.asar: codex://threads/, codex://threads/new, codex://review?pr=PR_URL&path=FILE_PATH&line=LINE&side=right, codex://space, codex://settings/connections, codex://plugins/, codex://launch, codex://shared-thread; only one chatgpt:// string (chatgpt://safety-faster-model); 'Alt+Space' appears with globalShortcut; strings 'app-server' / 'app-server is not available' present. No prompt/query parameter for threads/new was found.
  - source: grep -a on /Applications/ChatGPT.app/Contents/Resources/app.asar, 2026-09-18
- [verified] The old native ChatGPT app (bundle com.openai.chat) is not present on this Mac (mdfind returns nothing); a leftover /Applications/Codex.app 26.901.20858 with the same bundle id com.openai.codex also exists.
  - source: mdfind and /Applications/Codex.app/Contents/Info.plist, 2026-09-18
- [verified] In July 2026 the standalone Codex macOS app self-updated into the unified ChatGPT.app: 'Self-update removed Codex.app, official Codex download installs ChatGPT instead' (issue opened 2026-07-09, closed); related issues #31944, #32022, #32631 about `codex app` and Launchpad.
  - source: https://github.com/openai/codex/issues/31866 (gh issue view, 2026-09-18); https://github.com/openai/codex/issues/32631
- [verified] Claude Desktop local MCP: config at ~/Library/Application Support/Claude/claude_desktop_config.json with {"mcpServers":{name:{command,args,env}}}; servers are launched by Claude Desktop (stdio); 'completely quit Claude Desktop and restart it' to load config; 'All actions require your explicit approval before execution'; logs in ~/Library/Logs/Claude/mcp.log and mcp-server-NAME.log.
  - source: https://modelcontextprotocol.io/docs/2026-07-28/develop/connect-local-servers.md (fetched 2026-09-18)
- [verified] Claude Desktop supports one-click Desktop Extensions (.mcpb, formerly .dxt) installed via Settings > Extensions or 'Advanced settings' > Extension Developer > select .mcpb; 'Restart Claude Desktop to refresh the extension registry'.
  - source: https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop (article dated 2026-06-30, fetched 2026-09-18)
- [likely] Claude Desktop MCP transport/feature summary (secondary source): stdio for local servers, Streamable HTTP via Custom Connectors for remote, legacy SSE not natively supported; 'Claude Desktop fully supports MCP tools. Support for resources and prompts...is limited compared to Claude Code'; no mention of sampling/elicitation/roots support.
  - source: https://mcpverdict.com/mcp/clients/claude-desktop/ (page updated 2026-08-24, fetched 2026-09-18)
- [uncertain] Claude Desktop does not support MCP elicitation: feature request '[FEATURE] MCP elicitation support in Claude Desktop app' opened 2026-03-30, closed 2026-03-31 with stateReason COMPLETED and no human comment; an Oct-2025 server changelog reports Claude Desktop answering elicitation/create with -32601 Method not found; sampling reported unsupported in Claude Desktop.
  - source: https://github.com/anthropics/claude-code/issues/41110 (gh issue view 2026-09-18); https://glama.ai/mcp/servers/@portel-dev/ncp/blob/15e66a895070b491d2839485dc6a759e72812009/tests/ELICITATION-TESTING-GUIDE.md; https://www.mcpjam.com/clients/claude
- [verified] Claude Desktop and ChatGPT both support the MCP Apps extension (io.modelcontextprotocol/ui); ChatGPT additionally 'Partial' for Skills over MCP.
  - source: https://modelcontextprotocol.io/extensions/client-matrix.md (fetched 2026-09-18)
- [verified] Claude Desktop deep links: claude://claude.ai/new?q=... prefills the prompt without auto-submitting ('Prompt text passed in q is truncated to roughly 14,000 characters'); claude://claude.ai/chat/{conversation-id}; claude://claude.ai/project/{project-id}; claude://code/new?q=&folder= (folder confirmed by user); claude://cowork/new?q=&folder=&file=; none auto-submit.
  - source: https://support.claude.com/en/articles/14729294-open-claude-desktop-with-a-link (article dated 2026-06-30, fetched 2026-09-18)
- [likely] Claude Desktop Quick Entry: default shortcut is double-tap Option, configurable to Option+Space or custom; opens a text box that starts a new chat; Claude Desktop must be running but can stay in the background; needs macOS 12+.
  - source: https://support.claude.com/en/articles/12626668-use-quick-entry-with-claude-desktop-on-mac (via WebSearch summary, 2026-09-18)
- [verified] Claude Code hooks run inside the Claude Desktop app: 'Claude Code fires the same hook events wherever it runs: sessions in the terminal, IDE extensions, the Desktop app, and cloud sessions.' Notification hook types include permission_prompt, idle_prompt, elicitation_dialog, agent_needs_input, agent_completed; events include Stop, UserPromptSubmit, Elicitation, ElicitationResult, SessionEnd.
  - source: https://code.claude.com/docs/en/hooks (fetched 2026-09-18)
- [verified] Electron apps enable the Chromium accessibility tree only when assistive technology is detected; third-party software can force it by setting the AXManualAccessibility attribute with AXUIElementSetAttributeValue(appRef, "AXManualAccessibility", true).
  - source: https://www.electronjs.org/docs/latest/tutorial/accessibility (fetched 2026-09-18)
- [verified] Community tool claudectl (eldrgeek/mac-controller, Python, MIT) controls Claude Desktop via the Accessibility API: reads mode/sessions/composer/tasks, fills and submits prompts; README warns 'Claude Desktop builds its accessibility tree only while it is the front app' and 'a Claude Desktop update can break it'; requires Accessibility permission for the calling app.
  - source: https://github.com/eldrgeek/mac-controller (fetched 2026-09-18)
- [likely] ChatGPT MCP is remote-only: 'ChatGPT connects to remote MCP servers. If your MCP server runs on a private network, on-premises, or on a developer machine, use Secure MCP Tunnel'; full MCP (write) on Business/Enterprise/Edu; Pro users 'connect MCPs with read/fetch permissions in developer mode'; developer mode enabled by admins (Business) or via Settings > Apps > Advanced Settings; 'MCP apps available on mobile? No - web only'.
  - source: https://help.openai.com/en/articles/12584461-developer-mode-and-mcp-apps-in-chatgpt (fetched via r.jina.ai proxy 2026-09-18; page says last updated '27 days ago')
- [verified] OpenAI developer docs: a ChatGPT-connected MCP server needs a 'Public HTTPS endpoint' (streamable HTTP, typically /mcp) or 'Secure MCP Tunnel'; 'A development tunnel or another HTTPS forwarding service can also provide an endpoint for local testing'; developer mode: Settings > Security and login > Developer mode, availability 'can depend on account and workspace policy'.
  - source: https://developers.openai.com/plugins/deploy/connect-chatgpt (fetched 2026-09-18)
- [verified] Secure MCP Tunnel: 'lets you connect private MCP servers to supported OpenAI products without opening inbound firewall ports'; tunnel-client runs where the server is, polls OpenAI over outbound HTTPS; supports --mcp-command (stdio) or --mcp-server-url (HTTP); requires tunnel_id, runtime API key and Platform Tunnels Read+Use permissions; products: ChatGPT developer mode, Codex, Responses API.
  - source: https://developers.openai.com/api/docs/guides/secure-mcp-tunnels (fetched 2026-09-18)
- [likely] ChatGPT 'Work with Apps' on macOS reads content from Terminal, iTerm, Warp, Prompt, Xcode, VS Code family, JetBrains family, Apple Notes, Notion, TextEdit, Quip via the macOS Accessibility API; 'Editing is only available with IDEs'; chat bar opens with Option+Space (changeable in ChatGPT > Settings > Keyboard Shortcut); no third-party integration API; 'You can't launch advanced voice mode from the companion window yet.'
  - source: https://help.openai.com/en/articles/10119604-work-with-apps-on-macos (fetched via r.jina.ai proxy 2026-09-18; page says updated 'last month')
- [verified] Chromium's AppleScript 'execute javascript' is disabled by default in Chrome stable and must be re-enabled via View > Developer > Allow JavaScript from Apple Events; Chromium recommends extensions/native messaging instead.
  - source: https://www.chromium.org/developers/applescript/ (fetched 2026-09-18)
- [likely] Since macOS 10.15, CGWindowListCopyWindowInfo omits kCGWindowName for other apps' windows unless the caller has Screen Recording permission; the call itself never prompts.
  - source: https://gist.github.com/chockenberry/164ab2d3dd76736f81c9e9eed63d81bf (via WebSearch 2026-09-18)
- [likely] Apple's Pasteboard Privacy: programmatic NSPasteboard reads outside a user paste will show an alert / per-app Allow-Deny; NSPasteboard gains accessBehavior and detect-pattern APIs; previewed in macOS 15.4 via defaults write <bundle> EnablePasteboardPrivacyDeveloperPreview -bool yes; reported as not enforced by default as of mid-2026.
  - source: https://mjtsai.com/blog/2025/05/12/pasteboard-privacy-preview-in-macos-15-4/ ; https://dev.to/jrw0ng/macos-tahoe-has-built-in-clipboard-history-heres-what-its-still-missing-for-developers-2026-4eko (via WebSearch 2026-09-18)
- [likely] TCC attribution: 'TCC stores a grant against a code signature and a path'; rebuilt ad-hoc-signed binaries are treated as different programs and re-prompt; a launchd-started process needs the grant on the executable the plist names, a Terminal-spawned process is attributed to Terminal (responsible process, responsibility_get_pid_responsible_for_pid); an MCP stdio server child is attributed to the MCP client app; grants are tied to a logged-in GUI session (fails over SSH).
  - source: https://openowl.dev/blog/macos-accessibility-screen-recording-permissions (dated 2026-08-07, fetched 2026-09-18); https://www.qt.io/blog/the-curious-case-of-the-responsible-process
- [verified] AXIsProcessTrustedWithOptions with kAXTrustedCheckOptionPrompt=true presents the system Accessibility prompt or redirects to Settings > Privacy & Security > Accessibility.
  - source: https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions
- [likely] A CLI tool/daemon sending Apple Events (e.g. System Events keystroke) must embed an Info.plist with NSAppleEventsUsageDescription (e.g. via -sectcreate __TEXT __info_plist) and, with hardened runtime, the com.apple.security.automation.apple-events entitlement; the user then approves an Automation grant per target app.
  - source: https://steipete.me/posts/2025/applescript-cli-macos-complete-guide ; https://mjtsai.com/blog/2018/08/23/apple-events-usage-description/ (via WebSearch 2026-09-18)
- [likely] macOS Notification Center database moved in Sequoia to ~/Library/Group Containers/group.com.apple.usernoted/db2/db (TCC-protected Group Container); it stores bundle id, title, body, timestamps of all apps' delivered notifications in SQLite.
  - source: https://9to5mac.com/2024/09/01/security-bite-apple-addresses-privacy-concerns-around-notification-center-database-in-macos-sequoia/ (fetched 2026-09-18)
- [verified] Rust crate versions on crates.io as of 2026-09-18: accessibility 0.2.0 and accessibility-sys 0.2.0 (2025-03-22, eiz), objc2 0.6.4 (2026-02-26), objc2-app-kit 0.3.2 and objc2-foundation 0.3.2 (2025-10-04), core-graphics 0.25.0 (2025-05-27), cidre 0.29.0 (2026-09-10), macos-accessibility-client 0.0.2 (2026-01-18).
  - source: https://crates.io/api/v1/crates/<name> queried 2026-09-18
- [likely] Claude app intents ('Ask Claude' via Siri/Spotlight) are documented for iOS only; the macOS-27 'Claude as Siri model provider' demos use private APIs and are not an official surface.
  - source: https://support.claude.com/en/articles/10263469-use-claude-app-intents-shortcuts-and-widgets-on-ios ; https://www.macrumors.com/2026/09/14/siri-can-be-swapped-out-for-chatgpt-claude/ (via WebSearch 2026-09-18)
- [verified] Current host code is a Codex icon-sync LaunchAgent: host/Cargo.toml depends on hidapi =2.6.6, toml =0.8.23, serde_json =1.0.151; host/src/main.rs 218 lines, host/src/config.rs 278 lines; host/README.md line 16 installs into ~/Library/Application Support/OpenMicroDial/ and starts a per-user LaunchAgent, line 75: launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.conol.openmicrodial.icon-sync.plist.
  - source: /Users/tony/projects/conol/openmicrodial/host/Cargo.toml:1-12; /Users/tony/projects/conol/openmicrodial/host/README.md:16,75
- [verified] Deck protocol fields that desktop-app integration must feed: `agent` {id 1..8, kind claude/codex/gemini/other, name(24), state working/question/ready/error/idle, title(64)}; `card` {agent,id,type q/r,text(128),choices<=4x32,rec}; `reply` text(200); device `ptt` {act,agent,ctx talk/answer/name/codex,card,abort} and `answer` {agent,card,choice}; line limit 1024 bytes; <=4 agent updates/s/agent.
  - source: /Users/tony/projects/conol/openmicrodial/docs/deck-protocol.md:114-116,134-135,23-24,44-45
- [verified] Spec intent: roster generated live with no manual config, agents write their own titles and the daemon does no summarization; PTT = host STT injected into the active agent's prompt; agent replies via TTS; auto-announce on Ready.
  - source: scratchpad/spec.md lines 29, 39, 106-116 (/private/tmp/claude-501/-Users-tony-projects-conol-openmicrodial/753eb9de-97f0-480e-9319-617fbf0a5d73/scratchpad/spec.md)
- [likely] MCP 2026-07-28 spec deprecates sampling (kept >=12 months); Claude Code CLI added elicitation support in March 2026 (Elicitation hooks) — so the CLI, not Desktop, is where server-initiated user prompts work.
  - source: https://stacktr.ee/blog/mcp-2026-spec-changes ; https://www.builder.io/blog/claude-code-updates (via WebSearch 2026-09-18); https://code.claude.com/docs/en/hooks
- [verified] Codex app-server is a documented JSON-RPC protocol in the openai/codex repo (codex-rs/app-server/README.md covers thread/start, turn/start, MCP App UI fields, elicitation IDs); the unified ChatGPT.app bundles Resources/codex and codex-code-mode-host and runs its own app-server child.
  - source: gh api repos/openai/codex/contents/codex-rs/app-server/README.md (2026-09-18); /Applications/ChatGPT.app/Contents/Resources listing

## Options
### A. Claude Desktop Code-mode sessions via Claude Code hooks (reuse the CLI hook integration)
- pros: Officially supported ('same hook events... the Desktop app'); event-driven working/question/ready/idle via Notification(agent_needs_input, agent_completed, permission_prompt, idle_prompt), Stop, UserPromptSubmit, Elicitation; zero extra permissions; identical code path to Claude Code CLI.
- cons: Covers only Code mode, not Chat/Cowork; hooks read ~/.claude/settings.json so the daemon must install/merge its hook entries.
- recommendation: Ship in v1 as the primary Claude Desktop path.
### B. Shipped stdio MCP server for Claude Desktop Chat/Cowork (deck_report_status / deck_ask_operator blocking / deck_say) proxying to the daemon's local API; distribute as .mcpb + claude_desktop_config.json
- pros: Tools fully supported locally (stdio); no cloud; no TCC; blocking ask_operator gives a clean question-card round trip (deck `answer`/PTT text returned as the tool result); deck_say gives clean TTS text; packaging as .mcpb is one-click.
- cons: State is self-reported and model-initiated (no push, no 'working' signal, model may forget); each call is approval-gated unless user allows always; elicitation/sampling unavailable in Desktop; tool-call timeout for a blocking tool undocumented; restart required after config edit.
- recommendation: Ship in v1 with a 'self-reported' roster badge and a pasteable project-prompt snippet; verify blocking-tool timeout first.
### C. PTT into Claude Desktop via claude://claude.ai/new?q=<transcript>
- pros: Official, stable, no permissions, ~14k-char capacity, works when app is backgrounded.
- cons: Always a NEW chat; prefill only (operator must press Enter on the Mac); cannot append to the current conversation.
- recommendation: Default v1 PTT path for Claude Chat when no ask_operator card is pending.
### D. Accessibility-API injection/reading (AXManualAccessibility + AX value set/observe, or pasteboard+Cmd-V, CGEvent Return) for either app
- pros: Only way to type into the CURRENT chat or read the last message without model cooperation; works for both Electron apps; Rust bindings exist (accessibility 0.2.0, objc2-app-kit 0.3.2, cidre 0.29.0).
- cons: Needs Accessibility grant on the daemon identity (stable signing required); Electron tree is DOM-derived and breaks on app updates; Claude reportedly builds the tree only when frontmost; must activate/steal focus; pasteboard reads will trigger Apple's privacy alert; no reliable working/question signal.
- recommendation: Opt-in 'experimental' PTT injection only; never a state source in v1.
### E. ChatGPT via MCP over Secure MCP Tunnel (tunnel-client --mcp-command) or public HTTPS+OAuth
- pros: Only sanctioned programmatic surface for ChatGPT; same MCP server code as B; tunnel supports stdio servers.
- cons: Cloud round-trip through OpenAI for every deck call; write tools require Business/Enterprise/Edu (Pro = read-only developer mode); needs a Platform org, tunnel_id and runtime API key; admin/policy gating; desktop-app parity with web unstated; same self-reporting limits as B.
- recommendation: Defer past v1; document as 'possible for Business/Enterprise users' and revisit if OpenAI adds local MCP to the desktop app.
### F. ChatGPT via URL scheme / AppleScript / notifications
- pros: codex:// scheme exists (threads/new, launch, review...); Chromium sdef is present in the bundle; app posts UserNotifications.
- cons: No documented prompt parameter for codex://threads/new; Electron almost certainly does not implement Chromium's AppleScript classes and JS-from-AppleEvents is gated anyway (unverified); no API to observe another app's notifications short of the TCC-protected Notification Center SQLite.
- recommendation: Do not build on it; at most spot-test codex://threads/new parameters and osascript once.
### G. Daemon packaging for TCC: Developer-ID-signed app bundle (LSUIElement) with embedded NSAppleEventsUsageDescription, apple-events entitlement, registered as login item (SMAppService / per-user LaunchAgent) — GUI and MCP server are separate clients
- pros: Accessibility/Automation grants land on a stable identity and survive rebuilds; grant prompts name the product; matches the existing per-user LaunchAgent install; keeps the MCP server child (attributed to Claude/ChatGPT by TCC) and the gpui GUI free of AX/keystroke code.
- cons: Requires a signing identity and notarization discipline; ad-hoc dev builds will re-prompt; grants need a logged-in GUI session.
- recommendation: Adopt for v1 regardless of desktop-app support level; avoid Screen Recording (use AX window titles) and pasteboard reads.

## Open questions
- Does Claude Desktop 1.49585.0 actually implement elicitation/create now? Issue #41110 was closed 'completed' on 2026-03-31 with no comment while other sources still say unsupported — test with a minimal server before designing ask_operator as a blocking tool vs an elicitation.
- What is Claude Desktop's timeout for a long-blocking MCP tool call (needed for ask_operator waiting on a deck `answer`/PTT)? Undocumented; measure.
- Do Claude Desktop tool approvals persist ('always allow') per server so deck_report_status/deck_say do not prompt on every call? Otherwise self-reported state is too noisy.
- Does codex://threads/new accept a prompt/query parameter (inspect the app.asar JS bundle further or test by hand)? None found by string scan.
- Is the Chromium AppleScript dictionary bundled in ChatGPT.app functional under Electron (e.g. `tell application "ChatGPT" to get title of tabs of windows`)? Testing triggers an Automation TCC prompt; unverified.
- Does the ChatGPT macOS desktop app (Electron web UI) expose developer-mode MCP connectors identically to chatgpt.com? Help center only states mobile is unsupported.
- Secure MCP Tunnel commercial terms/latency, and whether deck tools (report/ask/say) count as 'write' actions that Pro read-only developer mode blocks.
- On macOS 27.0 (this Mac) is Pasteboard Privacy enforced by default? Sources only cover 15.4 preview and mid-2026 status.
- Once AXManualAccessibility is set, does Claude Desktop keep its AX tree populated while backgrounded, or is claudectl's 'front app only' observation still true in 1.49585.0?
- For Claude Desktop Code mode: confirm the Desktop app reads ~/.claude/settings.json hooks on macOS (docs say yes; a Windows bug report claims otherwise) and how session ids map to Desktop 'tasks' for roster naming.
