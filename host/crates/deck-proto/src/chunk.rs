//! Lines → 64-byte output reports. One line becomes `ceil(len/61)` reports
//! on consecutive `next_report` calls and is drained before the next line
//! starts (fragment atomicity, P:33-35); lines are never packed together,
//! exactly like `deck-sim.py`'s `frames()`.
use crate::{CHANNEL, PAYLOAD_MAX, REPORT_ID, REPORT_LEN};
use std::collections::VecDeque;

#[derive(Debug, Default)]
pub struct Chunker {
    lines: VecDeque<Vec<u8>>,
    /// Bytes of the front line already emitted.
    offset: usize,
}

impl Chunker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue one complete line (its LF included; appended when missing so a
    /// caller bug cannot glue two lines together).
    pub fn push_line(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut line = bytes.to_vec();
        if line.last() != Some(&b'\n') {
            line.push(b'\n');
        }
        self.lines.push_back(line);
    }

    /// The next report, or `None` when nothing is queued.
    pub fn next_report(&mut self) -> Option<[u8; REPORT_LEN]> {
        let line = self.lines.front()?;
        let rest = &line[self.offset..];
        let chunk = rest.len().min(PAYLOAD_MAX);
        let mut report = [0u8; REPORT_LEN];
        report[0] = REPORT_ID;
        report[1] = CHANNEL;
        report[2] = chunk as u8;
        report[3..3 + chunk].copy_from_slice(&rest[..chunk]);
        self.offset += chunk;
        if self.offset >= line.len() {
            self.lines.pop_front();
            self.offset = 0;
        }
        Some(report)
    }

    /// True while a line has been started but not fully emitted: nothing of
    /// another channel-5 line may be written now.
    pub fn in_flight(&self) -> bool {
        self.offset != 0
    }

    /// Lines queued, counting a partially emitted one.
    pub fn pending_lines(&self) -> usize {
        self.lines.len()
    }

    /// Reports still to emit for everything queued.
    pub fn pending_reports(&self) -> usize {
        self.lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let rest = if i == 0 {
                    line.len() - self.offset
                } else {
                    line.len()
                };
                rest.div_ceil(PAYLOAD_MAX)
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Drop everything queued (a new sid, a link drop).
    pub fn clear(&mut self) {
        self.lines.clear();
        self.offset = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(c: &mut Chunker) -> Vec<[u8; 64]> {
        std::iter::from_fn(|| c.next_report()).collect()
    }

    #[test]
    fn a_short_line_is_one_report_with_zero_padding() {
        let mut c = Chunker::new();
        c.push_line(b"{\"m\":\"synced\"}\n");
        assert_eq!(c.pending_reports(), 1);
        let reports = drain(&mut c);
        assert_eq!(reports.len(), 1);
        let r = reports[0];
        assert_eq!(&r[..3], &[6, 5, 15]);
        assert_eq!(&r[3..18], b"{\"m\":\"synced\"}\n");
        assert!(r[18..].iter().all(|&b| b == 0));
        assert!(c.is_empty());
        assert!(!c.in_flight());
    }

    #[test]
    fn a_long_line_spans_reports_and_is_drained_before_the_next() {
        let mut c = Chunker::new();
        let long = vec![b'a'; 130];
        c.push_line(&long);
        c.push_line(b"b\n");
        assert_eq!(c.pending_reports(), 3 + 1);
        let first = c.next_report().unwrap();
        assert_eq!(first[2], 61);
        assert!(c.in_flight());
        assert_eq!(c.pending_lines(), 2);
        let second = c.next_report().unwrap();
        assert_eq!(second[2], 61);
        let third = c.next_report().unwrap();
        assert_eq!(third[2], 9); // 130 a's + LF = 131 = 61 + 61 + 9
        assert_eq!(third[3 + 8], b'\n');
        assert!(!c.in_flight());
        let fourth = c.next_report().unwrap();
        assert_eq!(&fourth[..5], &[6, 5, 2, b'b', b'\n']);
        assert!(c.next_report().is_none());
    }

    #[test]
    fn a_missing_lf_is_appended_and_empty_input_ignored() {
        let mut c = Chunker::new();
        c.push_line(b"");
        assert!(c.is_empty());
        c.push_line(b"{}");
        let r = c.next_report().unwrap();
        assert_eq!(&r[..6], &[6, 5, 3, b'{', b'}', b'\n']);
    }

    #[test]
    fn exactly_61_bytes_is_one_full_report() {
        let mut c = Chunker::new();
        let mut line = vec![b'x'; 60];
        line.push(b'\n');
        c.push_line(&line);
        assert_eq!(c.pending_reports(), 1);
        assert_eq!(drain(&mut c).len(), 1);
        let mut line = vec![b'x'; 61];
        line.push(b'\n');
        c.push_line(&line);
        assert_eq!(c.pending_reports(), 2);
        let reports = drain(&mut c);
        assert_eq!(reports[1][2], 1);
    }

    #[test]
    fn clear_drops_a_partial_line() {
        let mut c = Chunker::new();
        c.push_line(&[b'a'; 100]);
        c.next_report();
        c.clear();
        assert!(c.next_report().is_none());
        assert!(!c.in_flight());
    }
}
