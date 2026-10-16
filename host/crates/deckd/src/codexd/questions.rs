//! Nonblocking questions are agent-message items, not server RPC requests.
//! Their lifetime spans turns; replies use Codex's structured user-message
//! envelope so the same pending entry is resolved in every client.
use super::{user_message_text, Answer, CardId, CardSpec, OriginKind, Value};
use deck_model::sanitize;
use serde_json::json;
use std::collections::VecDeque;

const OPEN: &str = "<send_user_message_question_reply>";
const CLOSE: &str = "</send_user_message_question_reply>";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Question {
    pub key: String,
    title: String,
    options: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(id: &str) -> Value {
        json!({"type":"agentMessage", "delivery":"async", "id":id,
            "questions":[{"title":id, "options":["Yes","No"]}]})
    }

    fn reply(id: &str) -> Value {
        let key = json!(["request_user_input_async", id, 0]).to_string();
        json!({"type":"userMessage", "content":[{"type":"text", "text":format!(
            "{OPEN}{}{CLOSE}", json!([{"questionItemId":key,"answer":"Yes"}]))}]})
    }

    #[test]
    fn history_resolves_questions_even_beyond_the_duplicate_window() {
        let mut questions = Questions::default();
        for i in 0..1100 {
            questions.observe(&ask(&i.to_string()));
        }
        for i in 0..1100 {
            questions.observe(&reply(&i.to_string()));
        }
        assert!(questions.pending.is_empty());
    }

    #[test]
    fn an_answer_during_history_loading_is_never_resurrected() {
        let mut old = Questions::default();
        old.observe(&ask("old"));
        let mut live = Questions::default();
        live.observe(&reply("old"));
        live.observe(&ask("new"));
        live.merge_older(old);
        assert_eq!(live.pending.len(), 1);
        assert_eq!(live.pending[0].title, "new");
    }
}

impl Question {
    pub fn reply(&self, answer: &Answer) -> Option<String> {
        let text = answer
            .choice
            .and_then(|i| self.options.get(usize::from(i)))
            .or_else(|| {
                answer
                    .label
                    .as_ref()
                    .filter(|label| !label.is_empty())
                    .and_then(|label| self.options.iter().find(|o| o.starts_with(label.as_str())))
            })
            .map(String::as_str)
            .or(answer.text.as_deref())?
            .trim();
        if text.is_empty() {
            return None;
        }
        let body = json!([{"answer":text,"question":self.title,"questionItemId":self.key}]);
        Some(format!("{OPEN}\n{body}\n{CLOSE}"))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Questions {
    pending: VecDeque<Question>,
    // Only duplicate notification suppression. Reconnect reconstructs the
    // queue from history instead of retaining entire chat items or images.
    seen: VecDeque<String>,
    visible: Option<(String, Option<CardId>)>,
}

impl Questions {
    fn remember(&mut self, key: String) {
        if !self.seen.contains(&key) {
            self.seen.push_back(key);
        }
        while self.seen.len() > 1024 {
            self.seen.pop_front();
        }
    }

    pub fn observe(&mut self, item: &Value) -> Option<CardId> {
        match item["type"].as_str() {
            Some("agentMessage" | "agent_message") if item["delivery"] == "async" => {
                let id = item["id"].as_str()?;
                for (index, question) in item["questions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let Some(title) = question["title"].as_str().filter(|s| !s.trim().is_empty())
                    else {
                        continue;
                    };
                    let key = json!(["request_user_input_async", id, index]).to_string();
                    if self.seen.contains(&key) || self.pending.iter().any(|q| q.key == key) {
                        continue;
                    }
                    self.remember(key.clone());
                    self.pending.push_back(Question {
                        key,
                        title: title.into(),
                        options: question["options"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect(),
                    });
                }
                None
            }
            Some("userMessage" | "user_message") => {
                let text = user_message_text(item)?;
                // Require the whole envelope: quoted examples in ordinary
                // user prose must not dismiss a real pending question.
                let body = text.trim().strip_prefix(OPEN)?.strip_suffix(CLOSE)?;
                let replies: Vec<Value> = serde_json::from_str(body).ok()?;
                let mut clear = None;
                for reply in replies {
                    if reply["answer"].as_str().is_none() {
                        continue;
                    }
                    if let Some(key) = reply["questionItemId"].as_str() {
                        clear = self.resolve(key).or(clear);
                    }
                }
                clear
            }
            _ => None,
        }
    }

    /// Live events win over the snapshot fetched in the background. In
    /// particular, a question answered during hydration must stay resolved.
    pub fn merge_older(&mut self, older: Self) {
        let mut pending: VecDeque<_> = older
            .pending
            .into_iter()
            .filter(|q| !self.seen.contains(&q.key))
            .collect();
        pending.append(&mut self.pending);
        self.pending = pending;
        for key in older.seen {
            self.remember(key);
        }
    }

    pub fn show(&mut self) -> Option<CardSpec> {
        if self.visible.is_some() {
            return None;
        }
        let question = self.pending.front()?;
        self.visible = Some((question.key.clone(), None));
        Some(CardSpec {
            request: question.key.clone(),
            text: sanitize::cut(&sanitize::collapse(&question.title), 200),
            choices: question.options.clone(),
            rec: None,
            free_text: true,
            origin: OriginKind::Question,
            persistent: true,
        })
    }

    pub fn carded(&mut self, key: &str, card: CardId) {
        if let Some((visible, slot)) = &mut self.visible {
            if visible == key {
                *slot = Some(card);
            }
        }
    }

    pub fn answered(&mut self, card: CardId) -> Option<Question> {
        let (key, slot) = self.visible.as_ref()?;
        if *slot != Some(card) {
            return None;
        }
        let question = self.pending.iter().find(|q| &q.key == key)?.clone();
        self.visible = None;
        Some(question)
    }

    pub fn pause(&mut self) -> Option<CardId> {
        self.visible.take().and_then(|(_, card)| card)
    }

    pub fn is_visible(&self) -> bool {
        self.visible.is_some()
    }

    pub fn resolve(&mut self, key: &str) -> Option<CardId> {
        self.pending.retain(|q| q.key != key);
        self.remember(key.to_string());
        if self
            .visible
            .as_ref()
            .is_some_and(|(visible, _)| visible == key)
        {
            self.pause()
        } else {
            None
        }
    }
}
