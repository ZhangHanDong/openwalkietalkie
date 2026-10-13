//! Chunker/Assembler glue for the hid thread: typed lines out, typed lines in.
use deck_proto::{Assembler, Chunker, DecodeError, DeviceLine, EncodeError, HostLine};

#[derive(Debug, Default)]
pub struct Framer {
    chunker: Chunker,
    assembler: Assembler,
    /// The last 16 encoded lines, for `err` diagnostics (redacted in logs).
    history: std::collections::VecDeque<Vec<u8>>,
}

pub const HISTORY: usize = 16;

impl Framer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode and queue `line`; returns the number of reports it takes.
    pub fn push_line(&mut self, line: &HostLine) -> Result<usize, EncodeError> {
        let bytes = deck_proto::encode(line)?;
        let reports = bytes.len().div_ceil(deck_proto::PAYLOAD_MAX);
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(bytes.clone());
        self.chunker.push_line(&bytes);
        Ok(reports)
    }

    pub fn next_report(&mut self) -> Option<[u8; 64]> {
        self.chunker.next_report()
    }

    pub fn has_pending(&self) -> bool {
        !self.chunker.is_empty()
    }

    /// True while a line is partially written (no other ch-5 line may start).
    pub fn in_flight(&self) -> bool {
        self.chunker.in_flight()
    }

    pub fn pending_reports(&self) -> usize {
        self.chunker.pending_reports()
    }

    /// Feed one input report; `f` sees each parsed device line.
    pub fn push_report(
        &mut self,
        report: &[u8],
        f: &mut dyn FnMut(Result<DeviceLine, DecodeError>),
    ) {
        self.assembler.push_report(report, f);
    }

    /// Drop the queue and any partial input line (link drop, new sid).
    pub fn clear(&mut self) {
        self.chunker.clear();
        self.assembler.clear();
    }

    /// The `m` and byte length of the last `n` lines written, oldest first —
    /// what an `err` log line may say without `[privacy] log_content`.
    pub fn recent(&self, n: usize) -> Vec<(String, usize)> {
        self.history
            .iter()
            .rev()
            .take(n)
            .map(|bytes| (m_of(bytes), bytes.len()))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

fn m_of(bytes: &[u8]) -> String {
    deck_proto::parse_host_line(bytes.strip_suffix(b"\n").unwrap_or(bytes))
        .map(|l| l.m().to_string())
        .unwrap_or_else(|_| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_go_out_as_reports_and_reports_come_in_as_lines() {
        let mut f = Framer::new();
        assert_eq!(f.push_line(&HostLine::Synced).unwrap(), 1);
        assert_eq!(
            f.push_line(&HostLine::Reply {
                slot: 1,
                text: "x".repeat(150),
                rev: 1
            })
            .unwrap(),
            4
        );
        assert_eq!(f.pending_reports(), 5);
        assert!(f.has_pending());
        let first = f.next_report().unwrap();
        assert_eq!(&first[3..17], b"{\"m\":\"synced\"}");
        assert!(!f.in_flight());
        f.next_report();
        assert!(f.in_flight());
        let mut got = Vec::new();
        let mut report = vec![6, 5, 11];
        report.extend_from_slice(b"{\"m\":\"up\"}\n");
        report.resize(64, 0);
        f.push_report(&report, &mut |l| got.push(l));
        assert_eq!(got, vec![Ok(DeviceLine::Up)]);
        assert_eq!(
            f.recent(5),
            vec![("synced".into(), 15), ("reply".into(), 192)]
        );
        f.clear();
        assert!(!f.has_pending());
        assert!(f.push_line(&HostLine::AgentGone { slot: 0 }).is_err());
    }
}
