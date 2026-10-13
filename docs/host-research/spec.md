<!-- Text of the product spec note https://conol.ai/share/note/i5mpd4opi9yoiabb7uy0z3gf as extracted on 2026-09-17; the mockups are in ../mockups/. Line numbers are cited by host-service.md and the research reports. -->

Shared by Tony Huang on Conol
Both Get CreditsJoin Conol

📟

Agent Command Deck — Feature & Interaction Spec

Overview
Working name: Agent Command Deck — a handheld command center for people who run multiple AI agents in parallel on their computer. The target feeling: a team leader with a walkie-talkie, directing the whole team. First-party integration target: Omarchy Linux (Hyprland), with a combined media event with DHH planned.
Hardware
SCROLL — wheel, top-left. Not clickable (dedicated ENTER exists instead).
PTT — push-to-talk, left edge.
ENTER / CANCEL / SHIFT — right edge, top to bottom.
LEFT / RIGHT — front face.
Display — 480×480, 16-bit OLED, capacitive touch. Rounded-corner square, 2.16" diagonal (≈38.8 mm side), 6 mm corner radius (≈74 px at 480 px) — keep content inside the corner arcs.
Audio — speaker (front grille) + microphone.
Radio — BLE for HID, classic Bluetooth for audio in/out (already implemented).
Grip — left hand. Thumb owns the front face (LEFT/RIGHT, lower touchscreen, reaches PTT on the left edge like a real radio); index curls over the top to SCROLL; middle/ring/pinky fingertips rest on the right-edge ENTER/CANCEL/SHIFT. Validate on the physical prototype that ENTER lands under a strong enough finger for ~100 presses/day.
Target user and role
Runs ~4 agent sessions in parallel, all in full autopilot. The user is an exception handler and dispatcher, not a gatekeeper — approve/reject is just one kind of AskUser question, not a privileged flow.
Interaction model
Three channels, strict division of labor:
Screen = the status board (triage: who needs me?)
Voice = the radio (direct: talk to the team)
Buttons = the decisions (confirm, reject, switch)
The OLED shows team status, never the work itself. The computer monitor is the work surface; this device is mission control. Resist making it a tiny computer.
Channel model
Each agent session is a walkie-talkie channel. One active channel at a time — that discipline keeps PTT unambiguous. Two-way sync with the desktop: flipping channel on the device focuses that agent's window in Hyprland; switching windows on the PC updates the device. The roster is generated live from hyprctl state (window titles) — zero manual configuration.
State model
Each agent is always in exactly one state, shown as a colored dot + live title text:
State | Meaning | What it wants from you | 
🟦 Working | Running; title shows live task | Nothing — glance only | 
🟥 Question | Blocked on an AskUser | A decision (SCROLL + ENTER, or voice) | 
🟩 Ready | Stage complete, awaiting instruction | A direction (PTT the next task) | 
⚠️ Error | Crashed / stuck | Attention | 
⬜ Idle | Not started / offline | Nothing | 

Roster sorts 🟥 then 🟩 to the top. The one-liner under each agent name is the live window title, verbatim — agents write their own titles (even TUI agents), so the daemon does no summarization; it only parses the agent type (claude / codex / gemini) from the title for the icon.
Navigation grammar
LEFT / RIGHT — move sideways among siblings at the current level: workspaces on desktop, agents on the roster, pending cards when several need attention.
SCROLL — move within the current view: choices on a card, rows in the roster, windows on the desktop carousel.
ENTER — confirm / descend one level (select choice, open channel).
CANCEL — ascend one level (card → channel → roster); at a face's top level, return to the home face.
PTT — talk, from anywhere, barge-in always; targets the active channel.
SHIFT — pure modifier, never acts alone.
Context-sensitivity rule: hardware keys always serve what's on screen. Question card showing → ENTER answers it; roster showing → ENTER opens the channel; desktop showing → ENTER = SUPER+Enter. One rule, predictable, no mode switch.
Interaction primitives
Question card (🟥)
Agent blocked on an AskUser. SCROLL navigates the choices, ENTER confirms the highlighted selection, touch picks an option directly, PTT gives a free-form voice answer when no option fits, CANCEL snoozes the card back into the queue.
Ready card (🟩)
Stage complete. Shows agent name, stage summary (title + last message), and any suggested next steps the agent offered. ENTER opens the channel; the primary affordance is hold PTT and give it its next orders.
Attention queue
Priority: 🟥 Question > 🟩 Ready. Multiple pending cards: free navigation — LEFT/RIGHT moves between cards, answer in any order (no forced FIFO).
Faces
A Face is one operating mode: what the display shows plus what the buttons and touchscreen do. Faces are fully rendered peers arranged on a horizontal strip; exactly one is on screen at a time.
The strip and switching
SHIFT+SCROLL slides along the strip — no menu, no labels: the faces themselves slide horizontally, one face per detent, snapping into place. Universal: works from inside any face.
The strip has hard ends (subtle bounce, no wrap-around) — first and last are fixed spatial anchors.
Agent Sessions is the home face, pinned first. CANCEL at a face's top level returns home; an idle timeout also returns home (configurable).
Horizontal touch swipes do not switch faces — touch belongs to the face layer, so the strip only moves via SHIFT+SCROLL. No accidental face changes while tapping virtual keys.

[IMAGE: The face strip — SHIFT+SCROLL slides fully rendered faces horizontally]

Input layers
Global layer (never remappable, works in every face): PTT, SHIFT+SCROLL (switch face), double-SHIFT (face manager), swipe down from the top edge (notification drawer).
Face layer (defined per face by its template/config): LEFT/RIGHT, SCROLL, ENTER, CANCEL, all other SHIFT chords, and every touchscreen interaction (tap targets, virtual keys, in-face scrolling).
Face manager
Double-SHIFT zooms out: the strip becomes a row of outlined cards (live thumbnails) that can be managed.
Reorder — drag on touch, or ENTER to grab → SCROLL to move → ENTER to drop.
Remove — CANCEL on a card, with confirmation.
Add — trailing "+" card → template gallery → pick a template → name the instance by holding PTT and speaking it.
Multiple instances of one template are allowed (e.g. two Macropads with different configs).
Settings is a system face: always present, pinned to the end of the strip, not removable.

[IMAGE: Face manager — double-SHIFT zooms out to outlined, reorderable cards]

Templates
Face | Display | Buttons + touch | 
Agent Sessions (home) | Roster: status dots, names, live titles, attention on top | The full navigation grammar (LEFT/RIGHT channels, SCROLL rows, ENTER open, question/ready cards) | 
Omarchy Remote | Current workspace, window list, workspace numbers | See Default key map below | 
Codex Micro Keyboard | Key legend tuned to the Codex TUI | Mirrors openmicrokbd's Codex simulation mode (key set TBD) | 
Macropad | Grid of labeled keys; touchscreen adds virtual keys beyond the physical buttons | Every key user-configurable; config in a host dotfile, pushed to the device | 
Usage Report | Per-subscription usage bars + reset countdowns | Mostly inert dashboard; SCROLL scrolls providers, ENTER refreshes | 
Settings (system) | Panels: volume (speaker + mic gain), auto-announce, alert behavior, brightness, host connection, about | SCROLL adjusts values, LEFT/RIGHT or touch switches panels | 

[IMAGE: Omarchy Remote face — workspace chips and the window list]

[IMAGE: Macropad face — dashed keys are touch-only]

[IMAGE: Usage Report face — per-subscription limits and reset times]

Notifications and the top drawer
On an alert (🟥 question, 🟩 ready): the chime plays and a heads-up toast slides down from the top; tapping it jumps to the relevant face and context.
Missed or dismissed toasts collect in the top drawer — swipe down from the top edge in any face. Tap an item to jump to its face/context; swipe to dismiss; CANCEL closes the drawer.
While the drawer has pending items, every face shows a thin indicator at the top edge.
Optional setting (Settings face): auto-jump straight to questions, for maximum interruptibility.
Default key map (Omarchy Remote face)
Control | Omarchy action | Rationale | 
RIGHT / LEFT | SUPER+Tab / SUPER+Shift+Tab | Thumb flips workspaces, forward = right | 
SCROLL rotate | Alt+Tab / Alt+Shift+Tab | Wheel = window carousel, one detent per window | 
ENTER | SUPER+Enter | "Give me a terminal here" | 
SHIFT+ENTER | SUPER+S | Scratchpad = summon the hidden terminal | 
Channel 1–4 | Ctrl+Alt+Shift+1..4 | Device channels map 1:1 to agent binds | 
SHIFT+CANCEL | Ctrl+SUPER+C | Screenshot / video record | 
PTT (hold) | Talk to active channel | STT on host → text injected into that agent's prompt | 

Voice pipeline
Hold PTT → host-side STT → text injected directly into the active agent's terminal prompt. Works for answering question cards and for dispatching new work.
Agent replies via TTS on the speaker.
Barge-in: pressing PTT during playback cuts TTS immediately and starts listening.
Check-in: select an agent and ask "report" → it answers with a TTS status summary.
Audio design
Question chime — two-tone rising (decision needed now).
Ready chime — single soft knock (come dispatch when free).
Auto-announce — ON by default: when an agent goes Ready, the speaker says a one-liner ("Agent 3: tests passing, ready"). User setting to disable.
Host-side audio mixer with priority queue: questions > ready > chatter. PTT preempts everything.
Screen views (inside the Agent Sessions face)
Roster (default) — rows: status dot, agent icon, name, live title one-liner; attention items floated to top.
Channel — agent name, last message, live transcript while PTT is held, big touch Approve/Reject when a card is pending.
Card — full-screen question or ready summary, large touch targets.

[IMAGE: Roster — the Agent Sessions home face, attention items floated to top]

[IMAGE: Channel view while PTT is held — live transcript and waveform]

[IMAGE: Question card — SCROLL chooses, ENTER confirms, PTT answers by voice]

Host architecture
A daemon on Omarchy discovers agents via hyprctl (window titles), normalizes events (question / ready / error), routes audio, and pushes a compact state protocol to the device.
The device renders UI locally from that state — no frame streaming (a 480×480×16-bit frame is 460 KB, too heavy for BLE interactivity).
All buttons arrive as custom HID reports, so every mapping and chord is software-defined and remappable.
Ships as an Omarchy module: daemon + default Hyprland binds + roster config, installable like any Omarchy package. This is the artifact for the DHH event: "plug it in, your workspaces are on the screen."
Open decisions
Per-agent voice identity — distinct TTS voice per agent vs distinct chime only. Recommendation: voices; you'd know who's talking without looking, and it makes the radio fantasy real.
Broadcast mode — SHIFT+PTT to address all agents. Recommendation: cut for v1; one-channel-at-a-time is the discipline that keeps PTT unambiguous.
Settings surface — resolved: an on-device Settings face (system face, pinned last on the strip); host dotfile config still available for power users.
Codex Micro Keyboard key set — to be specified from openmicrokbd's Codex simulation mode.

Published with ConolTerms
Privacy
Report content
Discord

We value your privacy
Conol uses cookies that are strictly necessary to sign you in and keep the app secure. With your consent, we also use analytics cookies to understand usage and improve the product. Read our Privacy Policy and Cookie Policy.

CustomizeReject non-essentialAccept all

Agent Command Deck — Feature & Interaction Spec · Conol

