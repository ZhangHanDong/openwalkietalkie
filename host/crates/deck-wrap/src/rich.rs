//! CommonMark/GFM to the deck's bounded, independently styled line format.
use crate::{lane_width, ui};
use deck_proto::{DocFont, DocKind};
use markdown::mdast::Node;
use ui::{
    rich::{self, BOLD, CODE, EMPHASIS, LINK},
    theme::Font,
};

type Glyph = (char, u8);

pub(super) fn push(out: &mut Vec<Glyph>, text: &str, flags: u8) {
    for c in text.chars() {
        if c == '\t' {
            out.extend([(' ', flags); 4]);
        } else if c == '\n' || (!c.is_control() && c != rich::ESC) {
            out.push((c, flags));
        }
    }
}

fn inline(node: &Node, flags: u8, out: &mut Vec<Glyph>) {
    match node {
        Node::Text(n) => push(out, &n.value.replace('\n', " "), flags),
        Node::InlineCode(n) => push(out, &n.value, flags | CODE),
        Node::InlineMath(n) => push(out, &n.value, flags | CODE),
        Node::Break(_) => out.push(('\n', flags)),
        Node::Strong(n) => n.children.iter().for_each(|n| inline(n, flags | BOLD, out)),
        Node::Emphasis(n) => n
            .children
            .iter()
            .for_each(|n| inline(n, flags | EMPHASIS, out)),
        Node::Delete(n) => n
            .children
            .iter()
            .for_each(|n| inline(n, flags | EMPHASIS, out)),
        Node::Link(n) => {
            n.children.iter().for_each(|n| inline(n, flags | LINK, out));
            if n.children.iter().map(Node::to_string).collect::<String>() != n.url {
                push(out, &format!(" ({})", n.url), flags | LINK);
            }
        }
        Node::Image(n) => push(out, &format!("[Image: {}]", n.alt), flags),
        Node::ImageReference(n) => push(out, &format!("[Image: {}]", n.alt), flags),
        Node::Html(n) => push(out, &n.value, flags | CODE),
        _ => {
            if let Some(children) = node.children() {
                children.iter().for_each(|n| inline(n, flags, out));
            }
        }
    }
}

fn encode(kind: char, indent: u8, glyphs: &[Glyph]) -> String {
    let mut s = format!("{kind}{}", char::from(b'0' + indent.min(9)));
    let mut current = 0;
    for &(c, flags) in glyphs {
        if flags != current {
            s.push(rich::ESC);
            s.push(char::from(b'A' + flags));
            current = flags;
        }
        s.push(c);
    }
    s
}

struct Layout {
    base: Font,
    lines: Vec<String>,
}

impl Layout {
    fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| l != "b0") {
            self.lines.push("b0".into());
        }
    }

    fn block(&mut self, kind: char, indent: u8, glyphs: Vec<Glyph>) {
        let width = lane_width(DocKind::Reply);
        let cap = deck_proto::caps().doc_line;
        for hard in glyphs.split(|g| g.0 == '\n') {
            let mut rest = hard;
            if rest.is_empty() {
                self.lines.push(encode(kind, indent, &[]));
            }
            while !rest.is_empty() {
                let mut end = 0;
                let mut space = None;
                while end < rest.len() {
                    let line = encode(kind, indent, &rest[..end + 1]);
                    if line.len() > cap || rich::measure(self.base, &line) > width {
                        break;
                    }
                    if rest[end].0 == ' ' && end > 0 {
                        space = Some(end);
                    }
                    end += 1;
                }
                end = end.max(1);
                if kind != 'c' && end < rest.len() {
                    if let Some(at) = space {
                        end = at;
                    }
                }
                self.lines.push(encode(kind, indent, &rest[..end]));
                rest = &rest[end..];
                if kind != 'c' {
                    while rest.first().is_some_and(|g| g.0 == ' ') {
                        rest = &rest[1..];
                    }
                }
            }
        }
    }

    fn node(&mut self, node: &Node, indent: u8, kind: char, prefix: &str) {
        match node {
            Node::Paragraph(_) | Node::Heading(_) => {
                let heading = matches!(node, Node::Heading(_));
                let mut glyphs = Vec::new();
                push(&mut glyphs, prefix, 0);
                inline(node, if heading { BOLD } else { 0 }, &mut glyphs);
                self.block(if heading { 'h' } else { kind }, indent, glyphs);
                self.blank();
            }
            Node::Code(n) => {
                if let Some(lang) = &n.lang {
                    let mut g = Vec::new();
                    push(&mut g, lang, BOLD);
                    self.block('h', indent, g);
                }
                let g = crate::syntax::highlight(&n.value, n.lang.as_deref());
                self.block('c', indent, g);
                self.blank();
            }
            Node::List(n) => {
                for (i, item) in n.children.iter().enumerate() {
                    let Node::ListItem(item) = item else {
                        continue;
                    };
                    let bullet = match item.checked {
                        Some(true) => "[x] ".into(),
                        Some(false) => "[ ] ".into(),
                        None if n.ordered => format!("{}. ", n.start.unwrap_or(1) as usize + i),
                        None => "• ".into(),
                    };
                    for (j, child) in item.children.iter().enumerate() {
                        self.node(
                            child,
                            indent.saturating_add(u8::from(j != 0)).min(9),
                            kind,
                            if j == 0 { &bullet } else { "" },
                        );
                        if self.lines.last().is_some_and(|l| l == "b0") {
                            self.lines.pop();
                        }
                    }
                }
                self.blank();
            }
            Node::Blockquote(n) => n
                .children
                .iter()
                .for_each(|n| self.node(n, indent.saturating_add(1).min(9), 'q', "")),
            Node::ThematicBreak(_) => self.lines.push("r0".into()),
            Node::Table(n) => {
                for (i, row) in n.children.iter().enumerate() {
                    let mut g = Vec::new();
                    if let Some(cells) = row.children() {
                        for (j, cell) in cells.iter().enumerate() {
                            if j != 0 {
                                push(&mut g, " | ", 0);
                            }
                            inline(cell, if i == 0 { BOLD } else { 0 }, &mut g);
                        }
                    }
                    self.block('p', indent, g);
                }
                self.blank();
            }
            Node::Definition(_) => {}
            _ => {
                if let Some(children) = node.children() {
                    children
                        .iter()
                        .for_each(|n| self.node(n, indent, kind, prefix));
                } else {
                    let mut g = Vec::new();
                    inline(node, 0, &mut g);
                    if !g.is_empty() {
                        self.block(kind, indent, g);
                        self.blank();
                    }
                }
            }
        }
    }
}

pub fn wrap_rich_reply(text: &str, size: u8) -> (DocFont, Vec<String>) {
    let (font, base) = match size {
        0 => (DocFont::RichRow, Font::Row),
        1 => (DocFont::RichText, Font::Text),
        _ => (DocFont::RichTitle, Font::Reading),
    };
    let mut layout = Layout {
        base,
        lines: Vec::new(),
    };
    if let Ok(root) = markdown::to_mdast(text, &markdown::ParseOptions::gfm()) {
        layout.node(&root, 0, 'p', "");
    } else {
        let mut g = Vec::new();
        push(&mut g, text, 0);
        layout.block('p', 0, g);
    }
    while layout.lines.last().is_some_and(|l| l == "b0") {
        layout.lines.pop();
    }
    if layout.lines.is_empty() {
        layout.lines.push("b0".into());
    }
    (font, layout.lines)
}

/// One bounded row regardless of how much output the tool has produced.
pub fn tool_header(text: &str, size: u8, index: u8, expanded: bool, streaming: bool) -> String {
    let first = text.lines().find(|s| !s.trim().is_empty()).unwrap_or("Tool");
    let name = first.split('·').next().unwrap_or(first).trim_matches(|c: char| c == '*' || c == '#' || c == '`' || c.is_whitespace());
    let mut name: String = name.chars().filter(|c| !c.is_control() && *c != rich::ESC).take(64).collect();
    if name.is_empty() { name.push_str("Tool"); }
    let status_text = first.split_once('·').map_or("", |(_, status)| status).to_ascii_lowercase();
    let failed = status_text.contains("failed") || status_text.contains("error");
    let status = if streaming { "running" } else if failed { "failed" } else { "done" };
    let base = match size { 0 => Font::Row, 1 => Font::Text, _ => Font::Reading };
    let mut clipped = false;
    loop {
        let row = format!("{}0{index:02x}{name}{} · {status}", if expanded { 'T' } else { 't' }, if clipped { "…" } else { "" });
        if row.len() <= deck_proto::caps().doc_line && rich::measure(base, &row) <= lane_width(DocKind::Reply) { return row; }
        if name.pop().is_none() { return format!("t0{index:02x}Tool"); }
        clipped = true;
    }
}

/// A compact activity row. Verb and target share a single bounded line.
pub fn activity_row(verb: &str, target: &str, size: u8) -> String {
    let mut glyphs = Vec::new();
    push(&mut glyphs, verb, LINK);
    push(&mut glyphs, " ", 0);
    push(&mut glyphs, &target.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).take(120).collect::<String>(), 0);
    bounded_row('p', 1, glyphs, size)
}

pub fn tool_disclosure(size: u8, index: u8, expanded: bool, count: usize) -> String {
    let label = format!("{} details · {count}", if expanded { "Hide" } else { "Show" });
    let mut glyphs = Vec::new();
    push(&mut glyphs, &label, 0);
    let mut row = bounded_row('p', 2, glyphs, size);
    row.replace_range(..2, &format!("{}0{index:02x}", if expanded { 'T' } else { 't' }));
    // The disclosure's extra index bytes count against the wire limit.
    while row.len() > deck_proto::caps().doc_line { row.pop(); }
    row
}

fn bounded_row(kind: char, indent: u8, mut glyphs: Vec<Glyph>, size: u8) -> String {
    let base = match size { 0 => Font::Row, 1 => Font::Text, _ => Font::Reading };
    let mut clipped = false;
    loop {
        let mut visible = glyphs.clone();
        if clipped { visible.push(('…', 0)); }
        let row = encode(kind, indent, &visible);
        if row.len() <= deck_proto::caps().doc_line && rich::measure(base, &row) <= lane_width(DocKind::Reply) { return row; }
        if glyphs.pop().is_none() { return format!("{kind}{indent}"); }
        clipped = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_headers_stay_one_row_with_unicode_and_long_names() {
        for size in 0..=2 {
            for open in [false, true] {
                let row = tool_header(&format!("**{} · completed**\n\nprivate output", "日本語-tool ".repeat(20)), size, 63, open, false);
                assert_eq!(rich::tool(&row), Some((63, open)));
                assert!(row.len() <= deck_proto::caps().doc_line);
                let base = match size { 0 => Font::Row, 1 => Font::Text, _ => Font::Reading };
                assert!(rich::measure(base, &row) <= lane_width(DocKind::Reply));
                assert!(row.ends_with(" · done"));
                assert!(!row.contains("private output"));
            }
        }
        assert_eq!(rich::tool("t0ffbad"), None);
        assert_eq!(rich::tool("t0é"), None);
    }
    #[test]
    fn rich_lines_fit_pixels_and_transport_without_losing_code() {
        let text = "# Results\n\nA **bold** and *emphasized* message with `code`.\n\n- [x] Done\n- [ ] Next\n\n```rust\n    let café = \"こんにちは\";\n```\n\n[Docs](https://example.com/a/long/path)\n\n| A | B |\n| - | - |\n| 1 | 2 |";
        for size in 0..=2 {
            let (f, lines) = wrap_rich_reply(text, size);
            for line in &lines {
                assert!(line.len() <= deck_proto::caps().doc_line, "{line:?}");
                assert!(rich::measure(crate::face(f), line) <= lane_width(DocKind::Reply));
            }
            assert!(lines.iter().any(|l| l.starts_with("h0\u{e000}B")));
            let mut code = String::new();
            for line in lines.iter().filter(|l| l.starts_with('c')) {
                rich::runs(&line[2..], |_, s| code.push_str(s));
            }
            assert_eq!(code, "    let café = \"こんにちは\";");
            assert!(lines.iter().any(|l| l.contains("[x]")));
        }
    }
    #[test]
    fn untrusted_control_bytes_cannot_inject_style() {
        let (_, lines) = wrap_rich_reply("Hello\u{1f}Pworld", 1);
        assert!(lines.iter().all(|l| !l.contains(rich::ESC)));
    }

    #[test]
    fn colored_code_and_activity_rows_fit_every_size_and_keep_token_styles() {
        for size in 0..=2 {
            let (font, lines) = wrap_rich_reply("```rust\nlet value = \"こんにちは\"; // comment\nprintln!(\"{}\", 42);\n```", size);
            let mut styles = Vec::new();
            for line in &lines {
                assert!(line.len() <= deck_proto::caps().doc_line);
                assert!(rich::measure(crate::face(font), line) <= lane_width(DocKind::Reply));
                if line.starts_with('c') { rich::runs(&line[2..], |flags, _| styles.push(flags & rich::SYNTAX_MASK)); }
            }
            for style in [rich::KEYWORD, rich::STRING, rich::NUMBER, rich::COMMENT] { assert!(styles.contains(&style)); }
            for line in [activity_row("Read", &"日本語.rs".repeat(50), size), tool_disclosure(size, 63, false, 64)] {
                assert!(line.len() <= deck_proto::caps().doc_line);
                assert!(rich::measure(crate::face(font), &line) <= lane_width(DocKind::Reply));
            }
        }
    }
}
