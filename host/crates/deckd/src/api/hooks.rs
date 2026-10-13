//! The hook ingestion methods (`hooks.claude`, `hooks.codex`,
//! `hooks.gemini`, `hooks.codex_notify`, `hooks.claude_statusline`), design
//! §5 / §4.11. Both doors reach this one place: `deckd hook` calls them as
//! JSON-RPC methods over the UDS, and `POST /v1/hooks/{cli}` hands the same
//! payload to [`ingest`] after its own bearer and scope check.
//!
//! **They are answered here, on the tokio side, and never as a
//! `CoreMsg::Call`** (B5). A held hook parks for the adapter's whole hold
//! budget — 120 s by default — and `Core::call` is answered synchronously
//! from inside the single core thread, so a hook answered there would stop
//! every agent, every device line and every other request for as long as
//! the operator took to press a button. `agent.wait`, the only other method
//! that may park, is special-cased out of `Core::call` for exactly that
//! reason; the hook methods are marked `reducer: false` in the method
//! table, which points a naive implementer straight at the trap. So the
//! door never gets there: `api::dispatch` routes every `hooks.` method into
//! this module before it can look at `reducer` at all, and the core has no
//! arm for them.
//!
//! Three rules run through everything below, and each one is a promise to
//! somebody sitting at a terminal:
//!
//! 1. **A hook never waits on a bug of ours.** Every failure path — an
//!    unparseable payload, a refused card, a full core, a poisoned memo,
//!    the hold cap of `[api] max_held_hooks` — answers `{}` and lets the
//!    CLI show its own dialog. The one thing the daemon must never do is
//!    hold a prompt hostage.
//! 2. **Report-only events answer before they work** (§4.11). The report
//!    hooks are synchronous http hooks with `timeout: 2`, so the daemon
//!    writes `{}` (or `SessionStart`'s beacon) and enqueues the plan on a
//!    task behind it; the operator's prompt never waits on the roster.
//!    Those tasks take one lock per adapter so the plans land in the order
//!    the events did — a `Stop` overtaking its own `UserPromptSubmit`
//!    would leave the agent reading `working` after the turn ended.
//! 3. **A payload is never logged** (§11). What reaches a log line here is
//!    the event name, the CLI and an error code; the body, the prompt, the
//!    tool input and the title stay inside the adapter.
use super::dispatch::{check_scope, ConnCtx};
use crate::adapters::{
    self, rt, Adapter, Adapters, GroupPlan, HookCtx, HookOutput, HookPayload, Ingest,
};
use crate::core::handle::{
    ClientInfo, CoreHandle, CoreMsg, Delivery, HoldResult, OpenSession, Reply, SessionKind,
};
use deck_model::{AgentId, ApiError, CardId, CardSet, PidKind, Request, Scope, SessionId};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

/// `hooks.gemini` and `deckd hook gemini` exist from v1 and are refused
/// by name: the adapter is v1.1 work (design §5.3), and a client reading
/// `/v1/schema.methods` is told when it becomes callable rather than left
/// to guess from a bare `-32601`.
pub const GEMINI_AVAILABLE: &str = "v1.1";

/// What a hook call carries: the CLI's payload, plus what the helper
/// captured around it (§4 of the week-3 contract). The TCP door posts the
/// event JSON alone, so everything but `payload` is absent there.
#[derive(Debug, Default, Deserialize)]
pub struct HookParams {
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub tty: Option<String>,
    #[serde(default)]
    pub cli_pid: Option<u32>,
    #[serde(default)]
    pub pid_kind: Option<PidKind>,
}

/// The CLI a `hooks.<cli>` method or a `/v1/hooks/{cli}` path names, when
/// this daemon has an adapter for it **on this plan layer**.
///
/// The Codex adapter sits on the other one (`adapters::rt`, which runs
/// its own plan against the core), so it does not answer here and
/// [`answers`] is what both doors ask before they refuse a CLI.
fn adapter_of<'a>(adapters: &'a Adapters, cli: &str) -> Option<&'a dyn Adapter> {
    match cli {
        // The chained status line (`deckd hook claude-statusline`) is the
        // same CLI's session seen from its other output: the same adapter
        // reads it, for the session's name; its `rate_limits` go to the
        // Usage face in [`ingest`] before the adapter sees it.
        "claude" | "claude_statusline" => Some(&adapters.claude),
        _ => None,
    }
}

/// Whether this daemon has an adapter for this CLI at all, on either
/// layer. It is the one list both doors refuse from, and the same list as
/// the `hooks.*` entries of `core::dispatch::CAPABILITIES`, which
/// `/v1/info` publishes so a client can tell "installed and answered" from
/// "installed, but no adapter in this daemon" (B8). `integrations.status`
/// does **not** read either one today — it reports what is written into
/// the CLI's config, not what this daemon would answer — so the refusal
/// below and `/v1/info.capabilities` are, for now, the only two places
/// that distinction is legible.
///
/// `codex` and `codex_notify` are one adapter behind two doors:
/// `notify`'s `agent-turn-complete` is the `Stop` hook's turn seen again,
/// and the adapter dedupes one against the other (C17). `claude` and
/// `claude_statusline` likewise: the status line is not a hook, but it is
/// the same session reporting, and the one output that carries the
/// account's rate limits.
#[must_use]
pub fn answers(cli: &str) -> bool {
    matches!(
        cli,
        "claude" | "claude_statusline" | "codex" | "codex_notify"
    )
}

/// [`answers`], by method name rather than CLI name.
#[must_use]
pub fn answered(method: &str) -> bool {
    answers(method.strip_prefix("hooks.").unwrap_or_default())
}

/// The two method names the Codex adapter answers.
fn is_codex(cli: &str) -> bool {
    matches!(cli, "codex" | "codex_notify")
}

/// The refusal a CLI with no adapter gets, by name.
///
/// `available` is a **promise**, so it is written only where there is one
/// to make: the one channel this daemon has scheduled. A CLI it has never
/// heard of (`cursor`, a typo, a client from the future) gets the bare
/// `-32601` — naming a release for it would tell a client to wait for
/// something nobody is building, and `integrations.status` reads this same
/// vocabulary to tell "installed and answered" from "installed, no adapter
/// in this daemon" (B8).
fn no_adapter(method: &str, cli: &str) -> ApiError {
    let mut err = ApiError::method_not_found(method);
    if cli == "gemini" {
        err.data["available"] = GEMINI_AVAILABLE.into();
    }
    err
}

/// Every `hooks.*` call from a line door (UDS/WS), which is how `deckd
/// hook` reaches the daemon.
///
/// The scope gate runs here rather than in `api::dispatch` because that
/// dispatch hands `hooks.*` over before it checks anything (B5); a caller
/// without `hooks:ingest` is refused `-32002 {need:"hooks:ingest"}` just
/// as it would be for any other method.
pub async fn dispatch(ctx: &ConnCtx, method: &str, params: serde_json::Value) -> Reply {
    let Some(spec) = super::dispatch::method(method) else {
        return Reply::err(ApiError::method_not_found(method));
    };
    if let Err(e) = check_scope(&ctx.scopes, spec) {
        return Reply::err(e);
    }
    let cli = method.strip_prefix("hooks.").unwrap_or_default();
    if !answers(cli) {
        return Reply::err(no_adapter(method, cli));
    }
    ingest(&ctx.adapters, &ctx.core, cli, params).await
}

/// The one handler both doors reach.
///
/// `cli` is `claude` or `codex`, already authenticated and scoped by the
/// caller. The `Reply` is the CLI's own hook output: `{}` for everything
/// that is not a decision.
pub async fn ingest(
    adapters: &Arc<Adapters>,
    core: &CoreHandle,
    cli: &str,
    params: serde_json::Value,
) -> Reply {
    // A CLI this daemon has no adapter for is refused **by name**, on both
    // doors, before anything else is read. It is not a failure to fail open
    // on: `{}` here would be indistinguishable from "answered, nothing to
    // do", and telling the two apart is the whole point of the
    // `-32601 {available}` shape — a client reading it can say "installed,
    // but no adapter in this daemon" rather than "installed and working"
    // (B8), and `deckd hook` writes the release into its one line on
    // stderr. Nobody waits on it either way: the refusal is immediate and
    // the helper still prints `{}` and exits 0.
    let refuse = |cli: &str| Reply::err(no_adapter(&format!("hooks.{cli}"), cli));
    let Some(cli) = canonical(cli).filter(|cli| answers(cli)) else {
        return refuse(cli);
    };
    // A malformed params object is our problem, not the CLI's: `{}` and
    // one line in the log.
    let params: HookParams = match serde_json::from_value(if params.is_null() {
        serde_json::json!({})
    } else {
        params
    }) {
        Ok(params) => params,
        Err(_) => {
            tracing::warn!(cli, "a hook call with params this daemon cannot read");
            return empty();
        }
    };
    let payload = HookPayload(params.payload);
    let event = payload.event().to_string();
    // `[hooks] enabled = false` is the operator saying "stop answering":
    // the CLIs keep their own dialogs and nothing reaches the deck.
    if !adapters.enabled() {
        return empty();
    }
    // The status line is the one call that carries the account's rate
    // limits (`rate_limits`, Pro/Max only): they go to the Usage face from
    // here, and the adapter then reads the rest of the payload for the
    // session's name. `send` is bounded, so a stalled core costs the line
    // its 200 ms and nothing more.
    if cli == "claude_statusline" && adapters.usage_enabled(crate::usage::Provider::Claude) {
        let windows = crate::usage::claude_statusline_windows(&payload.0);
        if !windows.is_empty() {
            let _ = core
                .send(CoreMsg::Usage {
                    provider: crate::usage::Provider::Claude,
                    windows,
                    account: crate::usage::account_label(payload.0["_openwalkietalkie_account"].as_str()),
                })
                .await;
        }
    }
    let Some(adapter) = adapter_of(adapters, cli) else {
        // The Codex adapter resolves its own agent and runs its own plan
        // (`adapters::rt::handle`), so the door hands it the call whole
        // rather than walking a plan here. Everything around that call is
        // the same on both layers: the same scope gate, the same
        // `[hooks] enabled` switch, and `{}` on every failure.
        if is_codex(cli) {
            let call = rt::HookCall {
                payload: payload.0,
                env: params.env,
                tty: params.tty,
                cli_pid: params.cli_pid,
                pid_kind: params.pid_kind,
            };
            let codex = core.adapters().clone();
            let (value, after_write) = rt::handle(core, &codex.codex, call).await;
            return Reply {
                result: Ok(value),
                after_write,
            };
        }
        return refuse(cli);
    };
    // The session's message inbox, learned from the helper's environment
    // (`crate::inbox`): recorded against the agent the plan resolves, so an
    // order can be posted into the session instead of typed at it.
    let inbox = crate::inbox::Inbox::from_env(&params.env);
    let ctx = HookCtx {
        env: params.env,
        tty: params.tty,
        cli_pid: params.cli_pid,
        pid_kind: params.pid_kind,
        now: None,
        // Per routed CLI, never a constant: `hold_s` is this adapter's
        // own key (B2) and the doors are one handler for every CLI.
        cfg: adapters.cfg(cli),
    };
    match adapter.ingest(&payload, &ctx) {
        Ingest::Nothing => empty(),
        // §4.11: the answer goes out first and the plan runs behind it.
        Ingest::Report(plan) => {
            defer(adapters.clone(), core.clone(), cli, payload, plan, inbox);
            empty()
        }
        Ingest::Beacon(out, plan) => {
            defer(adapters.clone(), core.clone(), cli, payload, plan, inbox);
            Reply::ok(out.json())
        }
        Ingest::Held {
            before,
            cmd,
            note,
            group,
        } => {
            tracing::debug!(cli, event = %event, "a held hook");
            let (out, after_write) = held(
                adapters, core, cli, &payload, before, cmd, note, group, inbox,
            )
            .await;
            // §4.9: the card is cleared by the door that wrote the
            // response, never by this handler — see [`held`].
            Reply {
                result: Ok(out.json()),
                after_write,
            }
        }
    }
}

/// The CLI name as a `'static` string, so a deferred task can carry it.
fn canonical(cli: &str) -> Option<&'static str> {
    match cli {
        "claude" => Some("claude"),
        "claude_statusline" => Some("claude_statusline"),
        "codex" => Some("codex"),
        // The notify channel keeps its own name to the end: the adapter
        // reads it to tell a `notify` turn from the `Stop` that fired for
        // the same one (C17).
        "codex_notify" => Some("codex_notify"),
        "gemini" => Some("gemini"),
        _ => None,
    }
}

fn empty() -> Reply {
    Reply::ok(HookOutput::Empty.json())
}

/// A core round trip that did not come back inside [`rt::CORE_BUDGET`].
///
/// `CoreHandle::send` is bounded already (200 ms, then `-32008`), but the
/// **reply** is a bare `oneshot`: a core thread that is alive and stalled —
/// it does synchronous `flush_state()`/`write_status()` file I/O on the
/// same thread — would otherwise park the hook until the CLI's own timeout
/// killed it. The installed `PreToolUse{AskUserQuestion}` hook has
/// `timeout: 130`, and C8 measured that event's terminal dialog as
/// *suppressed* while the hook is open, so those 130 s are 130 s in which
/// the operator sees nothing at all. Rule 1: a hook never waits on a bug
/// of ours.
fn core_stalled() -> ApiError {
    ApiError::internal("the core did not answer within the hook budget")
}

/// Run a report-only plan behind the answer that already went out.
fn defer(
    adapters: Arc<Adapters>,
    core: CoreHandle,
    cli: &'static str,
    payload: HookPayload,
    plan: Vec<Request>,
    inbox: Option<crate::inbox::Inbox>,
) {
    if plan.is_empty() {
        return;
    }
    tokio::spawn(async move {
        // One plan at a time per daemon: the events arrived in an order
        // that means something (`working` then `ready`), and two tasks
        // racing would land them in either.
        let _order = adapters.order().await;
        run(&adapters, &core, cli, &payload, plan, inbox).await;
    });
}

/// The adapter's own internal session (`adapter:<cli>` on one
/// `SessionKind::Internal`), opened on first use and kept for the life of
/// the daemon: an adapter's agents are idempotent by `key` **from the
/// owning session**, so a new session per hook would answer `-32004
/// conflict` on the second event of every CLI session.
pub(crate) async fn session_of(
    adapter: &dyn Adapter,
    core: &CoreHandle,
) -> Result<SessionId, ApiError> {
    if let Some(session) = adapter.session() {
        return Ok(session);
    }
    let open = core.open_session(OpenSession {
        kind: SessionKind::Internal,
        principal: adapter.principal(),
        scopes: vec![Scope::Agent, Scope::Observe],
        trusted: false,
        client: ClientInfo {
            name: adapter.client().to_string(),
            version: crate::core::dispatch::VERSION.to_string(),
        },
    });
    let opened = rt::budgeted(open)
        .await
        .unwrap_or_else(|| Err(core_stalled()))?;
    let kept = adapter.set_session(opened);
    if kept != opened {
        // Two hooks raced to open one. An internal session never expires,
        // so the loser is closed rather than left in the registry.
        let _ = core.send(CoreMsg::CloseSession(opened)).await;
    }
    Ok(kept)
}

/// Run a plan against the core as the adapter, and report back what the
/// core answered (the agent a register resolved, the cards it created).
///
/// The plan's head is the `agent.register` that resolves the CLI session
/// to an agent; everything behind it is addressed to that agent. A
/// request still holding the placeholder after that is dropped rather
/// than sent: there is nobody to send it to.
async fn run(
    adapters: &Adapters,
    core: &CoreHandle,
    cli: &'static str,
    payload: &HookPayload,
    plan: Vec<Request>,
    inbox: Option<crate::inbox::Inbox>,
) -> Option<AgentId> {
    let adapter = adapter_of(adapters, cli)?;
    let session = match session_of(adapter, core).await {
        Ok(session) => session,
        Err(e) => {
            refused(cli, "session.open", &e);
            return None;
        }
    };
    let principal = adapter.principal();
    let mut agent: Option<AgentId> = None;
    for mut req in plan {
        if matches!(req, Request::AgentRegister(_)) {
            let call = core.api(principal.clone(), session, false, req);
            match rt::budgeted(call)
                .await
                .unwrap_or_else(|| Err(core_stalled()))
            {
                Ok(result) => {
                    let id = result["agent"].as_str().map(|id| AgentId(id.to_string()));
                    if let Some(id) = id {
                        adapter.resolved(payload, &id);
                        // The inbox rides beside the agent, in the daemon's
                        // own table: the token must never reach a snapshot.
                        if let Some(inbox) = &inbox {
                            core.adapters().set_inbox(id.clone(), inbox.clone());
                        }
                        agent = Some(id);
                    }
                }
                Err(e) => {
                    refused(cli, "agent.register", &e);
                    return None;
                }
            }
            continue;
        }
        if let Some(agent) = &agent {
            adapters::address(&mut req, agent);
        }
        if adapters::agent_of(&req).is_some_and(adapters::is_pending) {
            continue;
        }
        let card = match &req {
            Request::CardSet(cmd) => Some(cmd.clone()),
            _ => None,
        };
        let method = method_of(&req);
        let call = core.api(principal.clone(), session, false, req);
        match rt::budgeted(call)
            .await
            .unwrap_or_else(|| Err(core_stalled()))
        {
            Ok(result) => {
                if let (Some(cmd), Some(id)) = (card, card_id(&result)) {
                    adapter.carded(payload, &cmd, id);
                }
            }
            Err(e) => refused(cli, method, &e),
        }
    }
    if let Some(id) = &agent {
        crate::chat::observe(adapters, core, session, id, payload).await;
    }
    agent
}

/// Every failure on the held path: `{}` to the CLI, and no delivery for
/// the door to confirm — no card of ours is waiting on a response.
fn nothing() -> (HookOutput, Option<Delivery>) {
    (HookOutput::Empty, None)
}

/// The held path (facts-seams §2d), card by card.
///
/// A single permission card is a group of one. Each card is parked on its
/// own hold; what the group shares is the deadline, which the store
/// inherits for every card after the first. The answers are accumulated
/// here and the CLI's one response is written after the last card.
///
/// The [`Delivery`] that comes back beside the output is the §4.9 seam:
/// this handler does **not** tell the core the answer was written, because
/// it has not been — the door reports `core.delivered(agent, card, ok)`
/// after the flush, exactly as it does for an `agent.wait` result. That is
/// what makes `op.answer`'s `delivered:true` the truth rather than a
/// promise, and it is what lets a failed write (a closed socket, a full
/// `resp_tx`, an aborted HTTP body) reach the store's own honest path:
/// `on_delivered(ok:false)` fails the card `undeliverable` instead of
/// clearing it as delivered.
#[allow(clippy::too_many_arguments)]
async fn held(
    adapters: &Arc<Adapters>,
    core: &CoreHandle,
    cli: &'static str,
    payload: &HookPayload,
    before: Vec<Request>,
    cmd: CardSet,
    note: serde_json::Value,
    group: Option<GroupPlan>,
    inbox: Option<crate::inbox::Inbox>,
) -> (HookOutput, Option<Delivery>) {
    let Some(adapter) = adapter_of(adapters, cli) else {
        return nothing();
    };
    // The prologue resolves the agent. It takes the same lock as a report
    // plan so a `SessionStart` cannot still be in flight behind it — under
    // the same budget as the round trips it guards, because a plan wedged
    // on the core would otherwise hold this queue for the CLI's timeout.
    let agent = {
        let Some(_order) = rt::budgeted(adapters.order()).await else {
            return nothing();
        };
        run(adapters, core, cli, payload, before, inbox).await
    };
    let Some(agent) = agent else {
        return nothing();
    };
    let Some(session) = adapter.session() else {
        return nothing();
    };
    let principal = adapter.principal();
    let mut cards = vec![cmd];
    if let Some(plan) = &group {
        cards.extend(plan.rest.iter().cloned());
    }
    // `cards` always holds at least `cmd`, so the subtraction cannot wrap
    // today — but the length is a **payload-derived** count (a question
    // group's size comes straight out of `tool_input`), and an arithmetic
    // panic here is a hook that answers a CLI with a dead socket. It is
    // saturating for the same reason nothing else in this path indexes.
    let last = cards.len().saturating_sub(1);
    let mut answers: Vec<Option<deck_model::Answer>> = Vec::with_capacity(cards.len());
    for (index, mut cmd) in cards.into_iter().enumerate() {
        cmd.agent = agent.clone();
        let park = core.set_card_held(principal.clone(), session, cmd.clone());
        let parked = rt::budgeted(park)
            .await
            .unwrap_or_else(|| Err(core_stalled()));
        let (card, hold, mut guard) = match parked {
            Ok(parked) => parked,
            Err(e) => {
                // `-32008` past `[api] max_held_hooks`, `-32003` when the
                // session ended under us: the CLI shows its own dialog.
                // No card was raised, so there is none to remember: the
                // CLI's own dialog is the whole of what the operator sees.
                refused(cli, "agent.card.set", &e);
                return nothing();
            }
        };
        adapter.carded(payload, &cmd, card);
        match hold.await {
            Ok(HoldResult::Answer(answer)) => {
                answers.push(Some(answer));
                guard.disarm();
                adapter.uncarded(payload, card);
                if index == last {
                    let out = output(adapter, &note, &cmd, group.as_ref(), &answers);
                    // The door clears the card once it has written this,
                    // and reports the failure when it could not: nothing
                    // here says `delivered` on the response's behalf.
                    return (
                        out,
                        Some(Delivery {
                            agent: agent.clone(),
                            card,
                        }),
                    );
                }
                // An intermediate group card is already delivered and
                // cleared by the store, with the group's deadline kept;
                // card k+1 follows.
            }
            // Snooze, the hold budget, `agent.card.clear`, shutdown. The
            // reason goes through with the card because the two shapes are
            // not the same memo write: a snooze or an expiry leaves the
            // card on the deck degraded to `⌨ Focus terminal` under its
            // own id, while a clear, a retire or an ack takes it off
            // altogether (`Release::degrades_card`).
            Ok(HoldResult::Released { reason }) => {
                tracing::debug!(cli, ?reason, "a released hold");
                guard.disarm();
                adapter.released(payload, &cmd, card, reason, Instant::now());
                return nothing();
            }
            // The core dropped the sender (shutdown). The guard stays
            // armed: it is the only thing that can still report this card,
            // and what it reports is a `HoldGone` — which is what the core
            // will degrade the card under, so that is the reason the
            // adapter is told.
            Err(_) => {
                adapter.released(
                    payload,
                    &cmd,
                    card,
                    deck_model::Release::HoldGone,
                    Instant::now(),
                );
                return nothing();
            }
        }
    }
    nothing()
}

/// The CLI's response once every card of a hold is in.
fn output(
    adapter: &dyn Adapter,
    note: &serde_json::Value,
    cmd: &CardSet,
    group: Option<&GroupPlan>,
    answers: &[Option<deck_model::Answer>],
) -> HookOutput {
    match group {
        Some(plan) => adapter.group_to_output(note, plan, answers),
        None => adapter.answer_to_output(note, cmd, answers.last().and_then(Option::as_ref)),
    }
}

fn card_id(result: &serde_json::Value) -> Option<CardId> {
    serde_json::from_value(result["card"]["id"].clone()).ok()
}

fn method_of(req: &Request) -> &'static str {
    match req {
        Request::AgentRegister(_) => "agent.register",
        Request::AgentUpdate(_) => "agent.update",
        Request::AgentRelease { .. } => "agent.release",
        Request::CardSet(_) => "agent.card.set",
        Request::CardClear(_) => "agent.card.clear",
        Request::Reply(_) => "agent.reply",
        _ => "agent.*",
    }
}

/// One line about a refused request: the CLI, which method, and the error
/// code. Never the payload, never the card text (§11).
fn refused(cli: &str, method: &str, error: &ApiError) {
    tracing::warn!(
        cli,
        method,
        code = error.code,
        why = %error.message,
        "the core refused a hook's request"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_model::Principal;

    fn ctx(core: CoreHandle, scopes: &[Scope]) -> ConnCtx {
        ConnCtx {
            core,
            principal: Principal::Uid(501),
            session: SessionId(2),
            scopes: scopes.to_vec(),
            trusted: false,
            client: ClientInfo::default(),
            adapters: Arc::new(Adapters::default()),
        }
    }

    fn payload(event: &str) -> serde_json::Value {
        serde_json::json!({
            "payload": {
                "session_id": "abc123",
                "cwd": "/w/project",
                "hook_event_name": event,
            }
        })
    }

    /// B5: a held hook parks for the whole hold budget and `Core::call`
    /// answers from inside the single core thread, so no `hooks.*` call
    /// may become a `CoreMsg::Call` — the adapters talk to the core
    /// through `set_card_held`/`api` and nothing else.
    ///
    /// The core here never answers, so the two Codex methods each spend
    /// `rt::CORE_BUDGET` before they give up and answer `{}` — the very
    /// budget that keeps a wedged core off a terminal. That is real time
    /// this test pays deliberately: the receiver has to stay alive for
    /// the assertion below to see what was sent.
    #[tokio::test]
    async fn no_hooks_method_becomes_a_core_call() {
        let (core, rx) = CoreHandle::detached();
        let ctx = ctx(core, &[Scope::HooksIngest]);
        for method in [
            "hooks.claude",
            "hooks.codex",
            "hooks.gemini",
            "hooks.codex_notify",
            "hooks.claude_statusline",
        ] {
            let reply = crate::api::dispatch::dispatch(&ctx, method, payload("Stop")).await;
            let _ = reply.result;
            while let Ok(msg) = rx.try_recv() {
                assert!(
                    !matches!(msg, CoreMsg::Call { .. }),
                    "{method} reached Core::call"
                );
            }
        }
    }

    /// Both adapters answer on their own door, and a daemon whose core
    /// never replies still answers `{}` rather than making the CLI wait
    /// (rule 1). The two layers are one promise from the terminal's side:
    /// `hooks.claude` and `hooks.claude_statusline` walk their plan here,
    /// `hooks.codex` and `hooks.codex_notify` hand the call to
    /// `adapters::rt`, and what the CLI reads back is the same empty
    /// object.
    #[tokio::test]
    async fn both_adapters_answer_on_their_own_door() {
        let (core, rx) = CoreHandle::detached();
        // A core that is gone, rather than one that is merely slow: every
        // send fails at once and no adapter waits out its budget.
        drop(rx);
        let ctx = ctx(core, &[Scope::HooksIngest]);
        for method in [
            "hooks.claude",
            "hooks.claude_statusline",
            "hooks.codex",
            "hooks.codex_notify",
        ] {
            let cli = method.strip_prefix("hooks.").unwrap();
            assert!(answers(cli), "{method}");
            let reply = dispatch(&ctx, method, serde_json::json!({})).await;
            assert_eq!(
                reply.result.expect("a hook is answered, never refused"),
                serde_json::json!({}),
                "{method}"
            );
        }
        // And the ones with no adapter on either layer still refuse by
        // name, so `integrations.status` can tell them apart (B8).
        for cli in ["gemini", "cursor"] {
            assert!(!answers(cli), "{cli}");
        }
    }

    /// The status line's `rate_limits` reach the core as usage windows
    /// before the adapter reads the line, and a line without them (an API
    /// key account, a build without the field) feeds nothing.
    #[tokio::test]
    async fn the_status_line_feeds_the_usage_face() {
        let (core, rx) = CoreHandle::detached();
        let ctx = ctx(core, &[Scope::HooksIngest]);
        let line = serde_json::json!({"payload": {
            "session_id": "abc123",
            "_openwalkietalkie_account": "work@example.com",
            "cwd": "/w/project",
            "model": {"id": "claude-opus-4-1", "display_name": "Opus"},
            "workspace": {"current_dir": "/w/project", "project_dir": "/w/project"},
            "rate_limits": {
                "five_hour": {"used_percentage": 42, "resets_at": 1_791_644_400},
                "seven_day": {"used_percentage": 8, "resets_at": 1_791_900_000}
            }
        }});
        let reply = ingest(&ctx.adapters, &ctx.core, "claude_statusline", line).await;
        assert_eq!(reply.result.unwrap(), serde_json::json!({}));
        let msg = rx.try_recv().expect("the windows reach the core first");
        match msg {
            CoreMsg::Usage { provider, windows, account } => {
                assert_eq!(provider, crate::usage::Provider::Claude);
                assert_eq!(account.as_deref(), Some("work@example.com"));
                assert_eq!(
                    windows
                        .iter()
                        .map(|w| (w.label.as_str(), w.percent))
                        .collect::<Vec<_>>(),
                    vec![("session", 42), ("week", 8)]
                );
            }
            other => panic!("{other:?}"),
        }
        let bare = serde_json::json!({"payload": {"session_id": "abc123", "cwd": "/w/project", "workspace": {}}});
        let reply = ingest(&ctx.adapters, &ctx.core, "claude_statusline", bare).await;
        assert_eq!(reply.result.unwrap(), serde_json::json!({}));
        // Whatever the deferred plan sent, no usage message came of it.
        tokio::task::yield_now().await;
        while let Ok(msg) = rx.try_recv() {
            assert!(!matches!(msg, CoreMsg::Usage { .. }), "{msg:?}");
        }
    }

    /// The CLIs whose adapter is not in this daemon are refused by name,
    /// with the release that will answer them — on **both** doors. A `{}`
    /// here would read as "answered, nothing to do", and `integrations.
    /// status` could no longer tell "installed and working" from
    /// "installed, no adapter in this daemon" (B8).
    ///
    /// `available` is a promise, so it is made only where there is one.
    /// The week-3 build answered every one of these `"week 3"`, which is a
    /// week that had already gone: a client asking when the method becomes
    /// callable was told it already had, forever. A CLI this daemon has
    /// never heard of carries no `available` at all.
    #[tokio::test]
    async fn a_cli_without_an_adapter_is_refused_by_name() {
        let (core, _rx) = CoreHandle::detached();
        let ctx = ctx(core, &[Scope::HooksIngest]);
        for (method, available) in [("hooks.gemini", GEMINI_AVAILABLE)] {
            let reply = dispatch(&ctx, method, payload("Stop")).await;
            let err = reply.result.expect_err(method);
            assert_eq!(err.code, ApiError::METHOD_NOT_FOUND, "{method}");
            assert_eq!(err.data["method"], method, "{method}");
            assert_eq!(err.data["available"], available, "{method}");
            assert_ne!(available, "week 3", "{method}: a week that has passed");
        }
        // The TCP door reaches `ingest` directly, with the path's `{cli}`
        // rather than a method name: same refusal, same release.
        for (cli, available) in [("gemini", Some(GEMINI_AVAILABLE)), ("cursor", None)] {
            let reply = ingest(&ctx.adapters, &ctx.core, cli, payload("Stop")).await;
            let err = reply.result.expect_err(cli);
            assert_eq!(err.code, ApiError::METHOD_NOT_FOUND, "{cli}");
            assert_eq!(err.data["method"], format!("hooks.{cli}"), "{cli}");
            match available {
                Some(available) => assert_eq!(err.data["available"], available, "{cli}"),
                None => assert!(
                    err.data.get("available").is_none(),
                    "{cli}: no release makes an unknown CLI callable, so none is named: {}",
                    err.data
                ),
            }
        }
    }

    /// The scope gate is this module's own (`api::dispatch` hands
    /// `hooks.*` over before it checks anything, B5), and it refuses by
    /// name rather than pretending the method does not exist.
    #[tokio::test]
    async fn a_caller_without_the_scope_is_refused_forbidden() {
        let (core, rx) = CoreHandle::detached();
        for scopes in [
            vec![Scope::Observe],
            vec![Scope::Agent, Scope::Operate],
            Vec::new(),
        ] {
            let ctx = ctx(core.clone(), &scopes);
            // Both layers are behind the same gate: a caller without
            // `hooks:ingest` may not hand either adapter a payload.
            for method in ["hooks.claude", "hooks.codex", "hooks.codex_notify"] {
                let reply = dispatch(&ctx, method, payload("Stop")).await;
                let err = reply.result.expect_err("no hooks:ingest");
                assert_eq!(err.code, ApiError::FORBIDDEN, "{method} {scopes:?}");
                assert_eq!(err.data["need"], "hooks:ingest", "{method}");
            }
        }
        assert!(rx.try_recv().is_err(), "nothing reached the core");
    }

    /// Rule 1: a hook never waits on a bug of ours. Params this daemon
    /// cannot read, a payload with no event, an event nobody handles —
    /// all `{}`, none of them an error the CLI has to interpret.
    #[tokio::test]
    async fn every_unusable_call_answers_empty() {
        let (core, rx) = CoreHandle::detached();
        let ctx = ctx(core, &[Scope::HooksIngest]);
        for params in [
            serde_json::json!({"payload": "not an object"}),
            serde_json::json!({"env": 7}),
            serde_json::json!({}),
            serde_json::Value::Null,
            serde_json::json!({"payload": {"hook_event_name": "Stop"}}),
            payload("WorktreeCreate"),
        ] {
            let reply = dispatch(&ctx, "hooks.claude", params.clone()).await;
            assert_eq!(
                reply.result.expect("a hook is answered, never refused"),
                serde_json::json!({}),
                "{params}"
            );
        }
        assert!(rx.try_recv().is_err(), "nothing reached the core");
    }

    /// A `PermissionRequest` exactly as `deckd hook claude` posts one.
    fn permission() -> serde_json::Value {
        serde_json::json!({
            "payload": {
                "session_id": "abc123",
                "cwd": "/w/project",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "tool_input": {"command": "psql -f migrate.sql staging"},
            }
        })
    }

    /// Rule 1, against the case it was written for: a core that is **alive
    /// and stalled**, not one that is gone.
    ///
    /// The receiver is kept, so every `send` succeeds and every reply
    /// `oneshot` is simply never answered — which is what a core thread
    /// doing synchronous `flush_state()` file I/O looks like from here.
    /// Unbudgeted, the hook would sit on that `rx.await` until Claude's own
    /// `timeout: 130` killed it, and C8 measured `PreToolUse`'s terminal
    /// dialog as **suppressed** for as long as the hook is open: 130 s in
    /// which the operator sees nothing at all. This pays one `CORE_BUDGET`
    /// of real time on purpose — the receiver has to stay alive for the
    /// assertions below to mean anything.
    #[tokio::test]
    async fn a_stalled_core_never_holds_the_claude_door_open() {
        let (core, rx) = CoreHandle::detached();
        let ctx = ctx(core, &[Scope::HooksIngest]);
        let at = std::time::Instant::now();
        let reply = dispatch(&ctx, "hooks.claude", permission()).await;
        assert_eq!(
            reply.result.expect("a hook is answered, never refused"),
            serde_json::json!({}),
            "the CLI shows its own dialog"
        );
        assert!(reply.after_write.is_none(), "no card, nothing to deliver");
        assert!(
            at.elapsed() < 2 * rt::CORE_BUDGET,
            "one budget, not the CLI's 130 s: {:?}",
            at.elapsed()
        );
        // It did reach for the core; what it did not do is wait forever.
        assert!(
            matches!(rx.try_recv(), Ok(CoreMsg::OpenSession { .. })),
            "the adapter's session is the round trip it gave up on"
        );
    }

    /// §4.9: the held path hands the door a `Delivery` and tells the core
    /// **nothing**.
    ///
    /// `hold_written` from inside the handler made `delivered:true` a
    /// promise: it ran before the response was serialised and written, so
    /// a closed socket, a full `resp_tx` or an aborted HTTP body still
    /// cleared the card as delivered and the store's own honest path for a
    /// failed write (`on_delivered(ok:false)` → `fail_card
    /// "undeliverable"`) was unreachable for a `hold` card. The seam built
    /// for this is `Reply::after_write`, which both doors already honour
    /// after the flush — this is the route that was not using it.
    #[tokio::test]
    async fn the_held_path_hands_the_door_the_delivery_and_clears_nothing_itself() {
        use deck_model::{Answer, AnswerSource, Principal as P};
        let (core, rx) = CoreHandle::detached();
        let ctx = ctx(core, &[Scope::HooksIngest]);
        // A core that answers: a session, an agent, and a parked hold the
        // operator resolves with `Allow`.
        let fake = std::thread::spawn(move || {
            let mut holds = crate::core::holds::Holds::new();
            let mut seen: Vec<&'static str> = Vec::new();
            while let Ok(msg) = rx.recv() {
                match msg {
                    CoreMsg::OpenSession { reply, .. } => {
                        seen.push("open_session");
                        let _ = reply.send(Ok(SessionId(7)));
                    }
                    CoreMsg::Api { req, reply, .. } => {
                        seen.push("api");
                        let value = match *req {
                            Request::AgentRegister(_) => serde_json::json!({"agent": "ag_1"}),
                            _ => serde_json::json!({}),
                        };
                        let _ = reply.send(Ok(value));
                    }
                    CoreMsg::SetCardHeld { reply, .. } => {
                        seen.push("set_card_held");
                        let (token, _parked) = holds.park(AgentId("ag_1".into()), CardId(7));
                        let (tx, hold) = tokio::sync::oneshot::channel();
                        let _ = reply.send(Ok((CardId(7), token, hold)));
                        let _ = tx.send(HoldResult::Answer(Answer {
                            choice: Some(0),
                            label: Some("Allow".into()),
                            text: None,
                            source: AnswerSource::Deck,
                            principal: P::Uid(501),
                            session: None,
                            utterance: None,
                        }));
                    }
                    CoreMsg::HoldWritten { .. } => seen.push("hold_written"),
                    CoreMsg::Delivered { .. } => seen.push("delivered"),
                    _ => seen.push("other"),
                }
            }
            seen
        });

        let reply = dispatch(&ctx, "hooks.claude", permission()).await;
        let out = reply.result.expect("the operator's decision");
        assert_eq!(
            out["hookSpecificOutput"]["decision"]["behavior"], "allow",
            "{out}"
        );
        let delivery = reply
            .after_write
            .expect("the door confirms this one after the flush");
        assert_eq!(delivery.card, CardId(7));
        assert_eq!(delivery.agent.0, "ag_1");
        // Nothing reported the write on the response's behalf: the door
        // does that, with the `ok` only it knows.
        drop(ctx);
        let seen = fake.join().expect("the fake core");
        assert!(seen.contains(&"set_card_held"), "{seen:?}");
        assert!(
            !seen.contains(&"hold_written") && !seen.contains(&"delivered"),
            "the handler cleared the card before the response was written: {seen:?}"
        );
    }

    /// `[hooks] enabled = false`: the daemon stops answering and the CLIs
    /// keep their own dialogs.
    #[tokio::test]
    async fn disabled_hooks_answer_empty_and_do_nothing() {
        let (core, rx) = CoreHandle::detached();
        let mut config = crate::config::Config::default();
        config.hooks.enabled = false;
        let mut ctx = ctx(core, &[Scope::HooksIngest]);
        ctx.adapters = Arc::new(Adapters::new(Arc::new(arc_swap::ArcSwap::from_pointee(
            config,
        ))));
        let reply = dispatch(&ctx, "hooks.claude", payload("SessionStart")).await;
        assert_eq!(reply.result.unwrap(), serde_json::json!({}));
        assert!(rx.try_recv().is_err(), "nothing reached the core");
    }
}
