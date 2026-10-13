//! Read the public messages Claude commits to its session transcript.
//! Bounded tail reads, no content logging, and no reasoning/thinking blocks.
use crate::{
    adapters::{Adapter, Adapters, HookPayload},
    core::handle::CoreHandle,
};
use deck_model::{AgentId, ChatRole, MessageMeta, ReplySet, Request, SessionId};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub(crate) struct Watch {
    path: PathBuf,
    handle: tokio::task::AbortHandle,
    finished: Arc<AtomicBool>,
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
pub(crate) type Watches = std::sync::Mutex<BTreeMap<AgentId, Watch>>;

fn text_blocks(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| {
            (b["type"].as_str() == Some("text"))
                .then(|| b["text"].as_str())
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn transcript_messages(bytes: &[u8], session: &str) -> Vec<(MessageMeta, String)> {
    let mut messages: Vec<(MessageMeta, String)> = Vec::new();
    for line in bytes.split(|b| *b == b'\n') {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v["sessionId"].as_str().is_some_and(|s| s != session)
            || v["isSidechain"].as_bool() == Some(true)
        {
            continue;
        }
        let Some(uuid) = v["uuid"].as_str() else {
            continue;
        };
        let role = match v["type"].as_str() {
            Some("user") => ChatRole::User,
            Some("assistant") => ChatRole::Assistant,
            _ => continue,
        };
        let content = &v["message"]["content"];
        let text = text_blocks(content);
        if !text.trim().is_empty() {
            let streaming = role == ChatRole::Assistant && v["message"]["stop_reason"].is_null();
            upsert(
                &mut messages,
                MessageMeta {
                    id: uuid.into(),
                    role,
                    streaming,
                },
                text,
            );
        }
        for block in content.as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("tool_use") => {
                    let Some(id) = block["id"].as_str() else {
                        continue;
                    };
                    let name = block["name"].as_str().unwrap_or("Tool");
                    let input = deck_model::sanitize::cut(&block["input"].to_string(), 2048);
                    upsert(
                        &mut messages,
                        MessageMeta {
                            id: format!("tool:{id}"),
                            role: ChatRole::Tool,
                            streaming: true,
                        },
                        format!("**{name}**\n\n```json\n{input}\n```"),
                    );
                }
                Some("tool_result") => {
                    let Some(id) = block["tool_use_id"].as_str() else {
                        continue;
                    };
                    let status = if block["is_error"].as_bool() == Some(true) {
                        "failed"
                    } else {
                        "completed"
                    };
                    let text = text_blocks(&block["content"]);
                    let key = format!("tool:{id}");
                    let previous = messages.iter().find(|(m, _)| m.id == key).map(|(_, text)| text.as_str());
                    let (name, input) = previous.and_then(|s| s.split_once("\n\n")).map_or(("Tool", ""), |(name, input)| (name.trim_matches('*'), input));
                    let body = format!("**{name} · {status}**\n\n{input}\n\n{}", deck_model::sanitize::cut(&text, 8192));
                    upsert(
                        &mut messages,
                        MessageMeta {
                            id: key,
                            role: ChatRole::Tool,
                            streaming: false,
                        },
                        body,
                    );
                }
                _ => {}
            }
        }
    }
    // A later item means the preceding assistant block is committed, even
    // when Claude omitted stop_reason on that JSONL record.
    let last = messages.len().saturating_sub(1);
    for (i, (meta, _)) in messages.iter_mut().enumerate() {
        if i != last && meta.role == ChatRole::Assistant {
            meta.streaming = false;
        }
    }
    messages
}

fn upsert(messages: &mut Vec<(MessageMeta, String)>, meta: MessageMeta, text: String) {
    let text = deck_model::sanitize::cut(&text, 64 * 1024);
    if let Some(old) = messages.iter_mut().find(|(m, _)| m.id == meta.id) {
        *old = (meta, text);
    } else {
        messages.push((meta, text));
    }
    while messages.len() > 64 {
        messages.remove(0);
    }
}

async fn tail(path: &Path) -> std::io::Result<Vec<u8>> {
    const MAX: u64 = 4 * 1024 * 1024;
    let mut file = tokio::fs::File::open(path).await?;
    let start = file.metadata().await?.len().saturating_sub(MAX);
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut bytes = Vec::new();
    file.take(MAX).read_to_end(&mut bytes).await?;
    if start != 0 {
        let first = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| i + 1);
        bytes.drain(..first);
    }
    Ok(bytes)
}

fn permitted_path(path: &str, session: &str) -> Option<PathBuf> {
    if session.is_empty() || session.contains(['/', '\\']) {
        return None;
    }
    let root = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))?
        .join("projects")
        .canonicalize()
        .ok()?;
    let path = Path::new(path).canonicalize().ok()?;
    (path.starts_with(root)
        && path.file_name()?.to_str()? == format!("{session}.jsonl")
        && path.is_file())
    .then_some(path)
}

/// Called after resolving ownership through the normal hook adapter.
pub(crate) async fn observe(
    adapters: &Adapters,
    core: &CoreHandle,
    session: SessionId,
    agent: &AgentId,
    payload: &HookPayload,
) {
    let principal = adapters.claude.principal();
    let event = payload.event();
    if event == "SessionEnd" {
        if let Ok(mut watches) = adapters.transcripts.lock() {
            watches.remove(agent);
        }
        return;
    }
    // Hooks give immediate start/end status even while a transcript is quiet.
    if matches!(event, "PreToolUse" | "PostToolUse" | "PostToolUseFailure") {
        if let Some(id) = payload.str("tool_use_id") {
            let name = payload.str("tool_name").unwrap_or("Tool");
            let live = event == "PreToolUse";
            let status = if live {
                "running"
            } else if event == "PostToolUseFailure" {
                "failed"
            } else {
                "completed"
            };
            let details = payload
                .get(if live { "tool_input" } else { "tool_response" })
                .map(Value::to_string)
                .unwrap_or_default();
            let req = ReplySet {
                agent: agent.clone(),
                text: format!(
                    "**{name} · {status}**\n\n```\n{}\n```",
                    deck_model::sanitize::cut(&details, 8192)
                ),
                message: Some(MessageMeta {
                    id: format!("tool:{id}"),
                    role: ChatRole::Tool,
                    streaming: live,
                }),
                speak: false,
                announce: None,
                lane: None,
            };
            let _ = crate::adapters::rt::budgeted(core.api(
                principal.clone(),
                session,
                false,
                Request::Reply(req),
            ))
            .await;
        }
    }
    let Some(cli_session) = payload.str("session_id") else {
        return;
    };
    let Some(path) = payload
        .str("transcript_path")
        .and_then(|p| permitted_path(p, cli_session))
    else {
        return;
    };
    let Ok(mut watches) = adapters.transcripts.lock() else {
        return;
    };
    watches.retain(|_, w| !w.handle.is_finished());
    if let Some(watch) = watches.get(agent).filter(|w| w.path == path) {
        if matches!(event, "Stop" | "UserPromptSubmit" | "PreToolUse") {
            watch.finished.store(event == "Stop", Ordering::Relaxed);
        }
        return;
    }
    watches.remove(agent);
    if watches.len() >= 64 {
        return;
    }
    let core = core.clone();
    let agent_id = agent.clone();
    let session_name = cli_session.to_string();
    let watched = path.clone();
    let config = adapters.config.clone();
    let finished = Arc::new(AtomicBool::new(event == "Stop"));
    let turn_finished = finished.clone();
    let task = tokio::spawn(async move {
        let mut previous = BTreeMap::new();
        let mut stamp = None;
        let mut quiet = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if !config.load().hooks.enabled {
                return;
            }
            let Ok(meta) = tokio::fs::metadata(&watched).await else {
                return;
            };
            let done = turn_finished.load(Ordering::Relaxed);
            let current = (meta.len(), meta.modified().ok(), done);
            if stamp == Some(current) {
                quiet += 1;
                if quiet > 240 {
                    return;
                }
                continue;
            }
            quiet = 0;
            stamp = Some(current);
            let Ok(bytes) = tail(&watched).await else {
                return;
            };
            for (mut message, text) in transcript_messages(&bytes, &session_name) {
                if done {
                    message.streaming = false;
                }
                let value = (message.clone(), text.clone());
                if previous.get(&message.id) == Some(&value) {
                    continue;
                }
                let request = ReplySet {
                    agent: agent_id.clone(),
                    text,
                    message: Some(message.clone()),
                    speak: false,
                    announce: None,
                    lane: None,
                };
                match crate::adapters::rt::budgeted(core.api(
                    principal.clone(),
                    session,
                    false,
                    Request::Reply(request),
                ))
                .await
                {
                    Some(Ok(_)) => {
                        previous.insert(message.id, value);
                    }
                    _ => return,
                }
            }
            // Never retain content outside the bounded tail's messages.
            if previous.len() > 128 {
                previous.clear();
            }
        }
    });
    watches.insert(
        agent.clone(),
        Watch {
            path,
            handle: task.abort_handle(),
            finished,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcript_history_tracks_tools_and_excludes_private_blocks() {
        let lines = [
            serde_json::json!({"type":"user","uuid":"u1","sessionId":"s","message":{"content":"Hello"}}),
            serde_json::json!({"type":"assistant","uuid":"a1","sessionId":"s","message":{"content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"**Working**"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"pwd"}}]}}),
            serde_json::json!({"type":"user","uuid":"u2","sessionId":"s","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"/project"}]}}),
            serde_json::json!({"type":"assistant","uuid":"a2","sessionId":"elsewhere","message":{"content":"unrelated"}}),
        ].map(|v| v.to_string()).join("\n");
        let messages = transcript_messages(lines.as_bytes(), "s");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].0.role, ChatRole::User);
        assert_eq!(messages[1].1, "**Working**");
        assert!(!messages[1].0.streaming);
        assert_eq!(messages[2].0.id, "tool:t1");
        assert!(!messages[2].0.streaming);
        assert!(messages[2].1.contains("/project"));
        assert!(messages[2].1.starts_with("**Bash · completed**"));
        assert!(messages[2].1.contains("pwd"), "expansion keeps the input and result together");
    }
}
