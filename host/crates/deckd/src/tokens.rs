//! `tokens.toml` (design §4.2, §9, §11; contract §3.4 "Tokens"): the bearer
//! tokens of the TCP door. A plaintext is `dk_` + base64url-nopad(32 random
//! bytes from `getrandom`), shown once and never stored; the file holds only
//! `[[token]] name, hash (blake3 hex), scopes, created, expires?, client?,
//! embedded_in`. It is 0600 and written atomically (temp file + rename), so
//! `deckd token` edits it while the daemon runs and the daemon reloads it
//! whole on change.
use deck_model::{Scope, Ts};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const PREFIX: &str = "dk_";
pub const RANDOM_BYTES: usize = 32;
/// Names are short identifiers: `hooks`, `build-bot`, `gate`.
pub const NAME_MAX: usize = 64;

/// One `[[token]]` entry. `hash` is the blake3 of the plaintext, hex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub name: String,
    pub hash: String,
    pub scopes: Vec<Scope>,
    pub created: Ts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<Ts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    #[serde(default)]
    pub embedded_in: Vec<String>,
}

impl Token {
    /// Past its expiry at `now`.
    pub fn expired(&self, now: Ts) -> bool {
        self.expires.is_some_and(|at| now >= at)
    }

    /// The `token.list` row: everything but the hash.
    pub fn public(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "scopes": self.scopes,
            "created": self.created,
            "expires": self.expires,
            "embedded_in": self.embedded_in,
            "client": self.client,
        })
    }
}

/// The whole file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(default, rename = "token")]
    pub tokens: Vec<Token>,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid tokens.toml: {0}")]
    Parse(String),
    #[error("invalid token name {0:?}: 1..64 printable characters, no spaces")]
    Name(String),
    #[error("a token named {0:?} already exists")]
    Duplicate(String),
    #[error("no token named {0:?}")]
    NotFound(String),
    #[error("invalid expiry {0:?}: expected 30d, 12h, 15m, YYYY-MM-DD or an RFC 3339 timestamp")]
    Expiry(String),
    #[error("unknown scope {0:?}")]
    Scope(String),
    #[error("at least one scope is needed")]
    NoScopes,
    #[error("no random source: {0}")]
    Random(String),
}

impl Tokens {
    /// Empty when the file is absent; an error when it is unreadable or invalid.
    pub fn load(path: &Path) -> Result<Tokens, TokenError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Tokens::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Tokens::default()),
            Err(source) => Err(TokenError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    pub fn parse(text: &str) -> Result<Tokens, TokenError> {
        toml::from_str(text).map_err(|e| TokenError::Parse(e.to_string()))
    }

    pub fn to_toml(&self) -> String {
        // A file of hashes and scopes always serializes; the fallback is an
        // empty file rather than a panic.
        toml::to_string(self).unwrap_or_default()
    }

    /// Atomic 0600 write: a temp file next to `path`, synced, then renamed.
    pub fn save(&self, path: &Path) -> Result<(), TokenError> {
        let write_err = |source| TokenError::Write {
            path: path.to_path_buf(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(write_err)?;
        }
        let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
        {
            use std::io::Write;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut file = opts.open(&tmp).map_err(write_err)?;
            file.write_all(self.to_toml().as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(write_err)?;
        }
        std::fs::rename(&tmp, path).map_err(write_err)?;
        #[cfg(unix)]
        {
            // A pre-existing file keeps its own mode across the rename only
            // if it was the temp file's; make 0600 the truth either way.
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(write_err)?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Token> {
        self.tokens.iter().find(|t| t.name == name)
    }

    pub fn list(&self) -> &[Token] {
        &self.tokens
    }

    /// A new token; returns the plaintext, the only time it exists. A
    /// duplicate name is refused (revoke or rotate instead).
    pub fn create(
        &mut self,
        name: &str,
        scopes: Vec<Scope>,
        expires: Option<Ts>,
        client: Option<String>,
        now: Ts,
    ) -> Result<String, TokenError> {
        let name = valid_name(name)?;
        if scopes.is_empty() {
            return Err(TokenError::NoScopes);
        }
        if self.get(&name).is_some() {
            return Err(TokenError::Duplicate(name));
        }
        let plaintext = new_plaintext()?;
        let mut scopes = scopes;
        scopes.sort();
        scopes.dedup();
        self.tokens.push(Token {
            name,
            hash: hash_hex(&plaintext),
            scopes,
            created: now,
            expires,
            client,
            embedded_in: Vec::new(),
        });
        Ok(plaintext)
    }

    pub fn revoke(&mut self, name: &str) -> Result<Token, TokenError> {
        let at = self
            .tokens
            .iter()
            .position(|t| t.name == name)
            .ok_or_else(|| TokenError::NotFound(name.to_string()))?;
        Ok(self.tokens.remove(at))
    }

    /// A new secret under the same name: scopes, expiry, client and
    /// `embedded_in` kept, `created` set to `now`, the old plaintext
    /// invalid at once. Returns the new plaintext.
    pub fn rotate(&mut self, name: &str, now: Ts) -> Result<String, TokenError> {
        let token = self
            .tokens
            .iter_mut()
            .find(|t| t.name == name)
            .ok_or_else(|| TokenError::NotFound(name.to_string()))?;
        let plaintext = new_plaintext()?;
        token.hash = hash_hex(&plaintext);
        token.created = now;
        Ok(plaintext)
    }

    /// The token a presented plaintext belongs to, if any and not expired.
    /// The hash comparison is constant time (`blake3::Hash` equality).
    pub fn verify(&self, plaintext: &str, now: Ts) -> Option<&Token> {
        let presented = blake3::hash(plaintext.as_bytes());
        self.tokens
            .iter()
            .find(|t| blake3::Hash::from_hex(&t.hash).is_ok_and(|stored| stored == presented))
            .filter(|t| !t.expired(now))
    }
}

fn valid_name(name: &str) -> Result<String, TokenError> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > NAME_MAX
        || name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"')
    {
        return Err(TokenError::Name(name.to_string()));
    }
    Ok(name.to_string())
}

/// `dk_` + base64url-nopad(32 random bytes) — 46 characters.
pub fn new_plaintext() -> Result<String, TokenError> {
    let mut bytes = [0u8; RANDOM_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| TokenError::Random(e.to_string()))?;
    Ok(format!("{PREFIX}{}", base64url_nopad(&bytes)))
}

/// blake3 of the plaintext, hex (what the file stores).
pub fn hash_hex(plaintext: &str) -> String {
    blake3::hash(plaintext.as_bytes()).to_hex().to_string()
}

/// RFC 4648 §5 without padding.
pub fn base64url_nopad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        let chars = chunk.len() + 1;
        for i in 0..chars {
            let idx = (n >> (18 - 6 * i)) & 63;
            out.push(ALPHABET[idx as usize] as char);
        }
    }
    out
}

/// `--scopes a,b` → scopes; unknown names are refused.
pub fn parse_scopes(spec: &str) -> Result<Vec<Scope>, TokenError> {
    let mut scopes = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let scope: Scope = serde_json::from_value(serde_json::Value::String(part.to_string()))
            .map_err(|_| TokenError::Scope(part.to_string()))?;
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    if scopes.is_empty() {
        return Err(TokenError::NoScopes);
    }
    Ok(scopes)
}

/// `30d` / `12h` / `15m` from `now`, `YYYY-MM-DD` (that day, 00:00 UTC), or
/// an RFC 3339 timestamp (`Z` or a `±HH:MM` offset).
pub fn parse_expiry(spec: &str, now: Ts) -> Result<Ts, TokenError> {
    let spec = spec.trim();
    let bad = || TokenError::Expiry(spec.to_string());
    if let Some((digits, unit)) = spec
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| spec.split_at(i))
        .filter(|(digits, _)| !digits.is_empty())
    {
        let n: u64 = digits.parse().map_err(|_| bad())?;
        let per = match unit {
            "d" => 86_400,
            "h" => 3_600,
            "m" => 60,
            _ => return parse_timestamp(spec).ok_or_else(bad),
        };
        let secs = n.checked_mul(per).ok_or_else(bad)?;
        return Ok(now.plus(Duration::from_secs(secs)));
    }
    Err(bad())
}

/// `YYYY-MM-DD`, or `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`.
fn parse_timestamp(spec: &str) -> Option<Ts> {
    let utc = |s: &str| serde_json::from_value::<Ts>(serde_json::Value::String(s.into())).ok();
    if spec.len() == 10 {
        return utc(&format!("{spec}T00:00:00Z"));
    }
    if let Some(ts) = utc(spec) {
        return Some(ts);
    }
    // A numeric offset: parse the local part as UTC and shift it.
    let (local, sign, offset) = spec
        .rfind(['+', '-'])
        .filter(|&at| at >= 19)
        .map(|at| (&spec[..at], spec.as_bytes()[at], &spec[at + 1..]))?;
    let (hh, mm) = offset.split_once(':')?;
    let (hh, mm): (u64, u64) = (hh.parse().ok()?, mm.parse().ok()?);
    if hh > 23 || mm > 59 {
        return None;
    }
    let shift = (hh * 3600 + mm * 60) * 1000;
    let local = utc(&format!("{local}Z"))?;
    Some(if sign == b'+' {
        Ts(local.millis().checked_sub(shift)?)
    } else {
        Ts(local.millis().checked_add(shift)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Ts = Ts(1_789_760_523_000); // 2026-09-18T19:42:03Z

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("deckd-tokens-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn plaintexts_have_the_prefix_and_46_random_characters() {
        let a = new_plaintext().unwrap();
        let b = new_plaintext().unwrap();
        assert_ne!(a, b);
        for t in [&a, &b] {
            assert!(t.starts_with("dk_"));
            assert_eq!(t.len(), 3 + 43);
            assert!(t[3..]
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        }
        assert_eq!(hash_hex("x").len(), 64);
        assert_eq!(hash_hex("x"), blake3::hash(b"x").to_hex().to_string());
    }

    #[test]
    fn base64url_matches_the_rfc_vectors() {
        assert_eq!(base64url_nopad(b""), "");
        assert_eq!(base64url_nopad(b"f"), "Zg");
        assert_eq!(base64url_nopad(b"fo"), "Zm8");
        assert_eq!(base64url_nopad(b"foo"), "Zm9v");
        assert_eq!(base64url_nopad(b"foob"), "Zm9vYg");
        assert_eq!(base64url_nopad(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url_nopad(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64url_nopad(&[0xfb, 0xff, 0xbf]), "-_-_");
        assert_eq!(base64url_nopad(&[0u8; 32]).len(), 43);
    }

    #[test]
    fn expiries_parse_relative_dates_and_timestamps() {
        assert_eq!(
            parse_expiry("30d", NOW).unwrap(),
            NOW.plus(Duration::from_secs(30 * 86_400))
        );
        assert_eq!(
            parse_expiry(" 12h ", NOW).unwrap(),
            NOW.plus(Duration::from_secs(12 * 3600))
        );
        assert_eq!(
            parse_expiry("15m", NOW).unwrap(),
            NOW.plus(Duration::from_secs(900))
        );
        assert_eq!(
            parse_expiry("2026-12-31", NOW).unwrap().to_string(),
            "2026-12-31T00:00:00.000Z"
        );
        assert_eq!(
            parse_expiry("2026-12-31T23:59:59Z", NOW)
                .unwrap()
                .to_string(),
            "2026-12-31T23:59:59.000Z"
        );
        assert_eq!(
            parse_expiry("2026-12-31T23:59:59.5Z", NOW)
                .unwrap()
                .to_string(),
            "2026-12-31T23:59:59.500Z"
        );
        assert_eq!(parse_expiry("2026-09-18T21:42:03+02:00", NOW).unwrap(), NOW);
        assert_eq!(parse_expiry("2026-09-18T14:12:03-05:30", NOW).unwrap(), NOW);
        for bad in [
            "",
            "d",
            "30",
            "30x",
            "-1d",
            "2026-13-01",
            "2026-12-31T25:00:00Z",
            "2026-12-31T23:59:59+25:00",
            "next week",
            "99999999999999999999d",
        ] {
            assert!(
                matches!(parse_expiry(bad, NOW), Err(TokenError::Expiry(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn scopes_parse_from_comma_lists() {
        assert_eq!(
            parse_scopes("observe,agent, operate,observe").unwrap(),
            vec![Scope::Observe, Scope::Agent, Scope::Operate]
        );
        assert_eq!(
            parse_scopes("hooks:ingest").unwrap(),
            vec![Scope::HooksIngest]
        );
        assert!(matches!(parse_scopes("root"), Err(TokenError::Scope(s)) if s == "root"));
        assert!(matches!(parse_scopes(""), Err(TokenError::NoScopes)));
        assert!(matches!(parse_scopes(","), Err(TokenError::NoScopes)));
    }

    #[test]
    fn create_verify_revoke_rotate() {
        let d = dir("crud");
        let path = d.join("tokens.toml");
        let mut tokens = Tokens::load(&path).unwrap();
        assert!(tokens.list().is_empty(), "absent file: empty");
        let plain = tokens
            .create(
                "gate",
                vec![Scope::Operate, Scope::Observe],
                None,
                Some("codex".into()),
                NOW,
            )
            .unwrap();
        assert!(plain.starts_with("dk_"));
        tokens.save(&path).unwrap();
        // The file: hash only, 0600, the design's keys.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(&plain), "no plaintext on disk: {text}");
        assert!(text.contains(&hash_hex(&plain)));
        assert!(text.contains("[[token]]"));
        assert!(text.contains("name = \"gate\""));
        assert!(
            text.contains("scopes = [\"observe\", \"operate\"]"),
            "{text}"
        );
        assert!(text.contains("created = \"2026-09-18T19:42:03.000Z\""));
        assert!(text.contains("client = \"codex\""));
        assert!(text.contains("embedded_in = []"));
        assert!(!text.contains("expires"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert!(
                std::fs::read_dir(&d).unwrap().count() == 1,
                "no temp file left"
            );
        }
        // Verify: the plaintext, not its hash, not a lookalike.
        let loaded = Tokens::load(&path).unwrap();
        assert_eq!(loaded, tokens);
        let t = loaded.verify(&plain, NOW).unwrap();
        assert_eq!(t.name, "gate");
        assert_eq!(t.scopes, vec![Scope::Observe, Scope::Operate]);
        assert_eq!(t.client.as_deref(), Some("codex"));
        assert!(loaded.verify(&hash_hex(&plain), NOW).is_none());
        assert!(loaded.verify(&plain[..plain.len() - 1], NOW).is_none());
        assert!(loaded.verify("", NOW).is_none());
        // Duplicate names are refused; bad names too.
        assert!(matches!(
            tokens.create("gate", vec![Scope::Observe], None, None, NOW),
            Err(TokenError::Duplicate(n)) if n == "gate"
        ));
        assert!(matches!(
            tokens.create("", vec![Scope::Observe], None, None, NOW),
            Err(TokenError::Name(_))
        ));
        assert!(matches!(
            tokens.create("two words", vec![Scope::Observe], None, None, NOW),
            Err(TokenError::Name(_))
        ));
        assert!(matches!(
            tokens.create("noscope", vec![], None, None, NOW),
            Err(TokenError::NoScopes)
        ));
        // Expiry: valid before, invalid at and after.
        let soon = NOW.plus(Duration::from_secs(60));
        let short = tokens
            .create("short", vec![Scope::Observe], Some(soon), None, NOW)
            .unwrap();
        assert_eq!(tokens.verify(&short, NOW).unwrap().name, "short");
        assert_eq!(
            tokens
                .verify(&short, NOW.plus(Duration::from_secs(59)))
                .unwrap()
                .name,
            "short"
        );
        assert!(tokens.verify(&short, soon).is_none());
        assert!(tokens
            .verify(&short, soon.plus(Duration::from_secs(1)))
            .is_none());
        assert!(tokens.get("short").unwrap().expired(soon));
        tokens.save(&path).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("expires = \"2026-09-18T19:43:03.000Z\""));
        // Rotate: same name and scopes, new secret, the old one dead.
        let later = NOW.plus(Duration::from_secs(3600));
        let rotated = tokens.rotate("gate", later).unwrap();
        assert_ne!(rotated, plain);
        assert!(
            tokens.verify(&plain, NOW).is_none(),
            "old plaintext invalid"
        );
        let t = tokens.verify(&rotated, NOW).unwrap();
        assert_eq!(t.name, "gate");
        assert_eq!(t.scopes, vec![Scope::Observe, Scope::Operate]);
        assert_eq!(t.client.as_deref(), Some("codex"));
        assert_eq!(t.created, later);
        assert!(matches!(
            tokens.rotate("nope", NOW),
            Err(TokenError::NotFound(n)) if n == "nope"
        ));
        // Revoke.
        let gone = tokens.revoke("short").unwrap();
        assert_eq!(gone.name, "short");
        assert!(tokens.verify(&short, NOW).is_none());
        assert!(matches!(
            tokens.revoke("short"),
            Err(TokenError::NotFound(_))
        ));
        tokens.save(&path).unwrap();
        let loaded = Tokens::load(&path).unwrap();
        assert_eq!(loaded.list().len(), 1);
        assert_eq!(loaded.verify(&rotated, NOW).unwrap().name, "gate");
        assert_eq!(
            loaded.list()[0].public(),
            serde_json::json!({
                "name": "gate", "scopes": ["observe", "operate"],
                "created": "2026-09-18T20:42:03.000Z", "expires": null,
                "embedded_in": [], "client": "codex",
            })
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn the_file_format_is_the_design_table_and_rejects_garbage() {
        let text = r#"
[[token]]
name = "hooks"
hash = "0000000000000000000000000000000000000000000000000000000000000000"
scopes = ["hooks:ingest"]
created = "2026-09-18T19:42:03.000Z"
embedded_in = ["claude"]

[[token]]
name = "curl"
hash = "ffff"
scopes = ["observe", "voice"]
created = "2026-09-18T19:42:03.000Z"
expires = "2026-12-31T00:00:00.000Z"
client = "other"
"#;
        let t = Tokens::parse(text).unwrap();
        assert_eq!(t.list().len(), 2);
        assert_eq!(t.get("hooks").unwrap().embedded_in, vec!["claude"]);
        assert_eq!(
            t.get("curl").unwrap().expires.unwrap().to_string(),
            "2026-12-31T00:00:00.000Z"
        );
        assert!(
            t.verify("dk_anything", NOW).is_none(),
            "a bad hex hash never matches"
        );
        assert_eq!(Tokens::parse("").unwrap(), Tokens::default());
        assert!(matches!(
            Tokens::parse("[[token]]\nname = 1"),
            Err(TokenError::Parse(_))
        ));
        assert!(matches!(
            Tokens::parse("[[token]]\nname = \"x\"\nhash = \"\"\nscopes = [\"root\"]\ncreated = \"2026-09-18T19:42:03.000Z\""),
            Err(TokenError::Parse(_))
        ));
        let d = dir("unreadable");
        std::fs::create_dir_all(d.join("tokens.toml")).unwrap();
        assert!(matches!(
            Tokens::load(&d.join("tokens.toml")),
            Err(TokenError::Read { .. })
        ));
        std::fs::remove_dir_all(d).unwrap();
    }
}
