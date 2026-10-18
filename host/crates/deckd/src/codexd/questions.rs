//! Nonblocking questions are agent-message items, not server RPC requests.
//! Their lifetime spans turns; replies use Codex's structured user-message
//! envelope so the same pending entry is resolved in every client.
use super::{user_message_text, Answer, CardId, CardSpec, OriginKind, Value};
use deck_model::sanitize;
use serde_json::json;
use std::collections::VecDeque;

const OPEN: &str = "<send_user_message_question_reply>";
const CLOSE: &str = "</send_user_message_question_reply>";

#[derive(serde::Deserialize)]
pub(super) struct QuestionReply {
    answer: String,
    question: Option<String>,
    #[serde(rename = "questionItemId")]
    key: Option<String>,
}

/// Recognize only a complete protocol envelope. A quoted example, ordinary
/// XML or malformed JSON remains ordinary user text and cannot resolve cards.
pub(super) fn parse_reply(text: &str) -> Option<Vec<QuestionReply>> {
    let body = text.trim().strip_prefix(OPEN)?.strip_suffix(CLOSE)?;
    let replies: Vec<QuestionReply> = serde_json::from_str(body).ok()?;
    (!replies.is_empty()).then_some(replies)
}

fn literal(text: &str) -> String {
    let mut out = String::new();
    for c in sanitize::strip_ansi(text).chars() {
        if c.is_ascii_punctuation() { out.push('\\'); }
        out.push(c);
    }
    out
}

/// Presentation only: the original JSON is still used for delivery and card
/// resolution. Existing rich-text headings and quote bars work on every deck.
pub(super) fn reply_text(text: &str) -> Option<String> {
    let replies = parse_reply(text)?;
    let mut out = if replies.len() == 1 { "### Your answer" } else { "### Your answers" }.to_string();
    for reply in replies {
        if let Some(question) = reply.question.filter(|s| !s.trim().is_empty()) {
            out.push_str(&format!("\n\n**{}**", literal(&sanitize::collapse(&question))));
        }
        out.push_str("\n\n");
        for (i, line) in literal(&reply.answer).split('\n').enumerate() {
            if i != 0 { out.push_str("  \n"); }
            out.push_str("> ");
            out.push_str(line.trim_end_matches('\r'));
        }
    }
    Some(out)
}

pub(super) fn question_text(item: &Value) -> Option<String> {
    if item["delivery"] != "async" { return None; }
    let questions = item["questions"].as_array()?;
    let mut blocks = Vec::new();
    for question in questions {
        let Some(title) = question["title"].as_str().filter(|s| !s.trim().is_empty()) else { continue; };
        let mut block = format!("**{}**", literal(&sanitize::collapse(title)));
        for option in question["options"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            block.push_str(&format!("\n\n- {}", literal(&sanitize::collapse(option))));
        }
        blocks.push(block);
    }
    if blocks.is_empty() { return None; }
    let heading = if blocks.len() == 1 { "### Question" } else { "### Questions" };
    let intro = item["text"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or_default();
    Some(format!("{heading}\n\n{}{}{}", literal(intro), if intro.is_empty() { "" } else { "\n\n" }, blocks.join("\n\n")))
}

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

    #[test]
    fn answers_show_question_context_without_transport_keys_or_markup() {
        let text = format!("{OPEN}{}{CLOSE}", json!([
            {"question":"Which database?", "answer":"SQLite", "questionItemId":"internal-key"},
            {"question":"项目名称？", "answer":"中文项目\n<notes> **literal**", "questionItemId":"another-key"}
        ]));
        let shown = reply_text(&text).unwrap();
        assert!(shown.starts_with("### Your answers"));
        assert!(shown.contains("**Which database\\?**\n\n> SQLite"));
        assert!(shown.contains("**项目名称？**\n\n> 中文项目  \n> \\<notes\\> \\*\\*literal\\*\\*"));
        assert!(!shown.contains("questionItemId") && !shown.contains("internal-key") && !shown.contains(OPEN));
        for size in 0..=2 {
            let (_, lines) = deck_wrap::wrap_rich_reply(&shown, size);
            assert!(lines.iter().any(|line| line.starts_with("q1") && line.contains("SQLite")));
            assert!(lines.iter().all(|line| line.len() <= deck_proto::caps().doc_line));
        }
        let no_title = format!("{OPEN}{}{CLOSE}", json!([{"answer":"Yes"}]));
        assert_eq!(reply_text(&no_title).unwrap(), "### Your answer\n\n> Yes");
    }

    #[test]
    fn prose_examples_and_invalid_envelopes_are_not_replies() {
        let valid = user_message_text(&reply("pending")).unwrap();
        for text in [
            format!("Example: {valid}"), format!("```\n{valid}\n```"),
            format!("{OPEN}not JSON{CLOSE}"), format!("{OPEN}[]{CLOSE}"),
            format!("{OPEN}{{\"answer\":\"Yes\"}}{CLOSE}"),
            format!("{OPEN}[{{\"answer\":42}}]{CLOSE}"),
            format!("{OPEN}[{{\"answer\":\"Yes\"}}]"),
        ] {
            assert!(reply_text(&text).is_none(), "{text}");
            let mut questions = Questions::default();
            questions.observe(&ask("pending"));
            questions.observe(&json!({"type":"userMessage","text":text}));
            assert_eq!(questions.pending.len(), 1);
        }
    }

    #[test]
    fn async_question_history_includes_titles_and_all_options() {
        let shown = question_text(&ask("Choose a database")).unwrap();
        assert!(shown.starts_with("### Question"));
        assert!(shown.contains("**Choose a database**"));
        assert!(shown.contains("- Yes") && shown.contains("- No"));
        assert!(question_text(&json!({"type":"agentMessage","text":"ordinary reply"})).is_none());
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
                let replies = parse_reply(&text)?;
                let mut clear = None;
                for reply in replies {
                    if let Some(key) = reply.key {
                        clear = self.resolve(&key).or(clear);
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
