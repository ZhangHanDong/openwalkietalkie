//! Bounded lexical coloring for common fenced-code languages. Unknown
//! languages stay plain; every source character still goes through `push`.
use crate::ui::rich::{CODE, COMMENT, FUNCTION, KEYWORD, NUMBER, STRING, TYPE};

pub(super) fn highlight(source: &str, language: Option<&str>) -> Vec<(char, u8)> {
    let lang = language
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let hash = matches!(
        lang.as_str(),
        "python" | "py" | "bash" | "sh" | "shell" | "zsh" | "yaml" | "yml" | "toml" | "ruby" | "rb"
    );
    let slash = matches!(
        lang.as_str(),
        "rust"
            | "rs"
            | "javascript"
            | "js"
            | "jsx"
            | "typescript"
            | "ts"
            | "tsx"
            | "c"
            | "cpp"
            | "c++"
            | "csharp"
            | "cs"
            | "go"
            | "java"
            | "kotlin"
            | "swift"
            | "jsonc"
            | "css"
    );
    let supported = hash || slash || lang == "json" || lang == "diff";
    let mut out = Vec::new();
    if !supported {
        super::rich::push(&mut out, source, CODE);
        return out;
    }
    if lang == "diff" {
        for line in source.split_inclusive('\n') {
            let style = if line.starts_with('+') {
                STRING
            } else if line.starts_with('-') {
                KEYWORD
            } else if line.starts_with("@@") {
                FUNCTION
            } else {
                0
            };
            super::rich::push(&mut out, line, CODE | style);
        }
        return out;
    }
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let style = if (hash && c == '#') || (slash && c == '/' && next == Some('/')) {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            COMMENT
        } else if slash && c == '/' && next == Some('*') {
            i += 2;
            let mut depth = 1;
            while i < chars.len() && depth > 0 {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else if lang == "rust" && chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            COMMENT
        } else if matches!(c, '\'' | '"' | '`') && !rust_lifetime(&chars, i, &lang) {
            let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
            i += if triple { 3 } else { 1 };
            while i < chars.len() {
                if chars[i] == '\\' {
                    i = (i + 2).min(chars.len());
                } else if chars[i] == c
                    && (!triple || (chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c)))
                {
                    i += if triple { 3 } else { 1 };
                    break;
                } else {
                    i += 1;
                }
            }
            STRING
        } else if c.is_ascii_digit() {
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '_'
                    || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                i += 1;
            }
            NUMBER
        } else if c.is_alphabetic() || c == '_' {
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let keywords = "as async await break case catch class const continue def default del do dyn elif else enum except export extends false False finally fn for from function if impl import in interface is let loop match mod move mut new nil None null nullptr of package pass private protected pub public raise ref return self Self static struct super switch this throw trait true True try type typeof unsafe use val var virtual void volatile where while with yield";
            if keywords.split_whitespace().any(|k| k == word) {
                KEYWORD
            } else if chars[i..].iter().find(|c| !c.is_whitespace()) == Some(&'(') {
                FUNCTION
            } else if c.is_uppercase() {
                TYPE
            } else {
                0
            }
        } else {
            i += 1;
            0
        };
        let token: String = chars[start..i].iter().collect();
        super::rich::push(&mut out, &token, CODE | style);
    }
    out
}

fn rust_lifetime(chars: &[char], at: usize, lang: &str) -> bool {
    if !matches!(lang, "rust" | "rs") || chars[at] != '\'' {
        return false;
    }
    let mut end = at + 1;
    while chars
        .get(end)
        .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        end += 1;
    }
    end > at + 1 && chars.get(end) != Some(&'\'')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colors_tokens_without_changing_code_or_leaking_styles_between_lines() {
        for (lang, code) in [("rust", "fn café<'a>() {\n /* a\n nested /* b */ end */\n let x = \"hello\\\"世界\"; // text\n x + 42\n}"), ("python", "def hello():\n    # comment\n    return \"世界\", 42"), ("json", "{\"value\":42,\"ok\":true}")] {
            let tokens = highlight(code, Some(lang));
            assert_eq!(tokens.iter().map(|(c, _)| c).collect::<String>(), code);
            for style in [KEYWORD, STRING, NUMBER] { assert!(tokens.iter().any(|(_, f)| *f == CODE | style)); }
        }
        let plain = highlight("hello 42 # text", Some("text"));
        assert!(plain.iter().all(|(_, flags)| *flags == CODE));
    }
}
