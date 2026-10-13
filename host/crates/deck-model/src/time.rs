//! Wall-clock stamps for the API (`"2026-09-18T19:42:03.120Z"`), derived
//! inside the reducer from a base pair `(Instant, Ts)` so nothing here reads
//! a clock.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{Duration, Instant};

/// Milliseconds since the Unix epoch, UTC; on the wire an RFC 3339 string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Ts(pub u64);

impl JsonSchema for Ts {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Ts".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "format": "date-time" })
    }
}

impl Ts {
    pub const fn from_millis(ms: u64) -> Self {
        Ts(ms)
    }

    pub const fn millis(self) -> u64 {
        self.0
    }

    /// `base` observed at `base_at`; `now` moved forward from there.
    pub fn at(base: Ts, base_at: Instant, now: Instant) -> Ts {
        Ts(base
            .0
            .saturating_add(now.saturating_duration_since(base_at).as_millis() as u64))
    }

    pub fn plus(self, d: Duration) -> Ts {
        Ts(self.0.saturating_add(d.as_millis() as u64))
    }

    /// Calendar fields (year, month, day, hour, minute, second, millisecond).
    fn fields(self) -> (i64, u32, u32, u32, u32, u32, u32) {
        let ms = self.0;
        let secs = ms / 1000;
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        // Howard Hinnant's civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = if m <= 2 { y + 1 } else { y };
        (
            y,
            m,
            d,
            (rem / 3600) as u32,
            (rem % 3600 / 60) as u32,
            (rem % 60) as u32,
            (ms % 1000) as u32,
        )
    }

    fn parse(s: &str) -> Option<Ts> {
        let b = s.as_bytes();
        if b.len() < 20
            || b[4] != b'-'
            || b[7] != b'-'
            || b[10] != b'T'
            || b[13] != b':'
            || b[16] != b':'
        {
            return None;
        }
        let num = |range: std::ops::Range<usize>| -> Option<u64> {
            let part = s.get(range)?;
            if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let (y, m, d) = (num(0..4)? as i64, num(5..7)?, num(8..10)?);
        let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
            return None;
        }
        let rest = &s[19..];
        let (millis, tz) = match rest.strip_prefix('.') {
            Some(frac) => {
                let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
                if digits.is_empty() {
                    return None;
                }
                let scaled = format!("{:0<3}", &digits[..digits.len().min(3)]);
                (scaled.parse::<u64>().ok()?, &frac[digits.len()..])
            }
            None => (0, rest),
        };
        if tz != "Z" {
            return None;
        }
        // days_from_civil.
        let (y, m) = if m <= 2 {
            (y - 1, m as i64)
        } else {
            (y, m as i64)
        };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        if days < 0 {
            return None;
        }
        Some(Ts(
            (days as u64 * 86_400 + hh * 3600 + mm * 60 + ss) * 1000 + millis
        ))
    }
}

impl fmt::Display for Ts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d, hh, mm, ss, ms) = self.fields();
        write!(f, "{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{ms:03}Z")
    }
}

impl Serialize for Ts {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Ts {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ts::parse(&s).ok_or_else(|| serde::de::Error::custom("expected an RFC 3339 UTC timestamp"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_parses_rfc3339_utc() {
        let ts = Ts(1_789_760_523_120); // 2026-09-18T19:42:03.120Z
        assert_eq!(ts.to_string(), "2026-09-18T19:42:03.120Z");
        assert_eq!(Ts::parse("2026-09-18T19:42:03.120Z"), Some(ts));
        assert_eq!(
            Ts::parse("2026-09-18T19:42:03Z"),
            Some(Ts(1_789_760_523_000))
        );
        assert_eq!(
            Ts::parse("2026-09-18T19:42:03.1Z"),
            Some(Ts(1_789_760_523_100))
        );
        assert_eq!(Ts(0).to_string(), "1970-01-01T00:00:00.000Z");
        assert_eq!(Ts::parse("1970-01-01T00:00:00.000Z"), Some(Ts(0)));
        assert_eq!(Ts::parse("2026-13-18T19:42:03Z"), None);
        assert_eq!(Ts::parse("2026-09-18T19:42:03+02:00"), None);
        assert_eq!(Ts::parse("garbage"), None);
        let json = serde_json::to_string(&ts).unwrap();
        assert_eq!(json, "\"2026-09-18T19:42:03.120Z\"");
        assert_eq!(serde_json::from_str::<Ts>(&json).unwrap(), ts);
        assert!(serde_json::from_str::<Ts>("\"x\"").is_err());
    }

    #[test]
    fn leap_days_and_year_ends_round_trip() {
        for s in [
            "2024-02-29T23:59:59.999Z",
            "2023-12-31T00:00:00.000Z",
            "2100-03-01T12:00:00.000Z",
            "2000-02-29T00:00:00.000Z",
        ] {
            assert_eq!(Ts::parse(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn at_moves_forward_from_the_base() {
        let base = Instant::now();
        let later = base + Duration::from_millis(1500);
        assert_eq!(Ts::at(Ts(1000), base, later), Ts(2500));
        assert_eq!(Ts::at(Ts(1000), later, base), Ts(1000));
        assert_eq!(Ts(5).plus(Duration::from_secs(1)), Ts(1005));
    }
}
