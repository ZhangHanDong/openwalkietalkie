//! Restore history without asking the server for a single unbounded frame.
use super::{chat_item, questions::Questions, Effect, Model, Rpc};
use serde_json::{json, Value};

#[derive(Default)]
pub(super) struct History {
    questions: Questions,
    messages: Vec<(deck_model::MessageMeta, String)>,
}

pub(super) async fn read(rpc: &Rpc, thread: &str) -> History {
    let mut history = History::default();
    let mut cursor = Value::Null;
    loop {
        let page = match rpc
            .request(
                "thread/items/list",
                json!({
                    "threadId": thread, "cursor": cursor, "limit": 8, "sortDirection": "asc"
                }),
            )
            .await
        {
            Ok(page) => page,
            Err(why) => {
                tracing::debug!(
                    why,
                    "codexd: history page unavailable; live prompts remain connected"
                );
                break;
            }
        };
        let Some(entries) = page["data"].as_array() else {
            break;
        };
        for entry in entries {
            let item = &entry["item"];
            history.questions.observe(item);
            if let Some((meta, text)) = chat_item(item, item["status"] == "inProgress") {
                history
                    .messages
                    .push((meta, deck_model::sanitize::cut(&text, 64 * 1024)));
                if history.messages.len() > 64 {
                    history.messages.remove(0);
                }
            }
        }
        let next = &page["nextCursor"];
        if entries.is_empty() || next.is_null() || *next == cursor {
            break;
        }
        cursor = next.clone();
    }
    history
}

impl Model {
    pub(super) fn hydrated(&mut self, thread: &str, history: History) -> Vec<Effect> {
        let Some(t) = self.threads.get_mut(thread) else {
            return Vec::new();
        };
        t.async_questions.merge_older(history.questions);
        // Never replace newer streamed messages with an earlier snapshot.
        if t.messages.is_empty() {
            for (meta, text) in history.messages {
                self.record_message(thread, meta, text);
            }
        }
        let mut effects = self.flush_messages();
        effects.extend(self.show_async_question(thread));
        effects
    }
}
