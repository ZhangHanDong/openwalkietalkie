//! The HID writer thread (contract §3.6): `hid_write` blocks for the BLE
//! round trip (≈60–360 ms), so the `deckd-hid` thread hands each output
//! report to a `deckd-hid-write` thread over a capacity-1 channel and never
//! waits for it; the writer reports `Written{took, ok}` back on a second
//! channel the hid loop drains each iteration. A write that never returns
//! is a stall after `WRITE_STALL` (`link_down("write_stall")`, the writer
//! abandoned, a new one spawned on the next open); a thread that ends
//! without answering is `lost()` at once. Closing is bounded the same way
//! (`CLOSE_BUDGET`): shutdown never joins a thread that is still inside a
//! write, so SIGTERM completes whatever the BLE link is doing.
//!
//! That `done` channel is also in the hid loop's wait (`hid::park`): the
//! outbox reports the next report due as soon as the pace elapses, which is
//! long before a write returns, so without it the loop would spin for the
//! whole round trip.
//!
//! The thread owns **its own** `HidDevice`: hidapi 2.6.7 declares
//! `unsafe impl Send for HidDevice` (`hidapi.rs:160`) and no `Sync` impl, so
//! `Arc<HidDevice>` is not `Send` and the reader and the writer cannot share
//! one handle. The deck is opened non-exclusively anyway (§6.6), so the hid
//! thread takes a second open of the same device and gives it to the writer,
//! used only for writes. macOS delivers every input report to that handle
//! too, so the writer drains and discards its own input queue after each
//! write — nothing reads it and an unbounded queue behind it would be a
//! slow leak.
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use deck_proto::REPORT_LEN;
use std::time::{Duration, Instant};

/// Input reports discarded from the writer's own handle per write.
const DISCARD_MAX: usize = 32;

/// A hung write is detected after this.
pub const WRITE_STALL: Duration = Duration::from_secs(5);

/// How long `close` waits for the thread to end before abandoning it.
///
/// `hid_write` is synchronous, so a thread inside a write cannot see
/// `WriterCmd::Close` and a plain `join()` is unbounded — a hung BLE write
/// would wedge `deckd-hid` and with it the whole SIGTERM path. A real write
/// is 60–360 ms (§3.6; 419 ms is the worst measured on hardware), so a
/// thread still writing after this is one that will not return, and it is
/// left to the process exit instead of waited for.
pub const CLOSE_BUDGET: Duration = Duration::from_millis(1000);

/// What the hid loop sends the writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriterCmd {
    /// One 64-byte output report.
    Write([u8; REPORT_LEN]),
    /// Drop the device handle; the thread ends.
    Close,
}

/// What the writer reports after each `Write`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    pub took: Duration,
    pub ok: bool,
}

/// The capacity-1 command channel (hid loop → writer).
pub type CmdChannel = (Sender<WriterCmd>, Receiver<WriterCmd>);
/// The capacity-1 result channel (writer → hid loop).
pub type DoneChannel = (Sender<Written>, Receiver<Written>);

/// The device refused the write (any hidapi error): the link is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("HID write failed")]
pub struct WriteFailed;

/// Where the writer thread puts a report. The device handle in production;
/// a fake in the tests, which must be able to block a write for as long as
/// they like without a real `hid_write`.
pub trait ReportSink: Send + 'static {
    /// Bytes written, or the error that ends the link.
    fn write(&mut self, report: &[u8; REPORT_LEN]) -> Result<usize, WriteFailed>;

    /// Whatever this handle received meanwhile; discarded.
    fn drain_input(&mut self) {}
}

/// The production sink: the writer's own non-exclusive open of the deck.
pub struct DeviceSink(pub hidapi::HidDevice);

impl ReportSink for DeviceSink {
    fn write(&mut self, report: &[u8; REPORT_LEN]) -> Result<usize, WriteFailed> {
        self.0.write(report).map_err(|e| {
            tracing::warn!("hid write failed: {e}");
            WriteFailed
        })
    }

    fn drain_input(&mut self) {
        let mut report = [0u8; REPORT_LEN];
        for _ in 0..DISCARD_MAX {
            match self.0.read_timeout(&mut report, 0) {
                Ok(n) if n > 0 => {}
                _ => break,
            }
        }
    }
}

/// The hid loop's side of the writer thread.
#[derive(Debug)]
pub struct Writer {
    pub cmds: Sender<WriterCmd>,
    pub done: Receiver<Written>,
    /// When the in-flight write was handed over, if one is.
    in_flight: Option<Instant>,
    /// The `done` channel is closed: the thread will never answer again.
    gone: bool,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    /// A writer over its channels (the thread itself is B3's).
    pub fn new(
        cmds: Sender<WriterCmd>,
        done: Receiver<Written>,
        join: Option<std::thread::JoinHandle<()>>,
    ) -> Self {
        Writer {
            cmds,
            done,
            in_flight: None,
            gone: false,
            join,
        }
    }

    /// The channel pair a writer thread and its hid loop share.
    pub fn channels() -> (CmdChannel, DoneChannel) {
        (crossbeam_channel::bounded(1), crossbeam_channel::bounded(1))
    }

    /// No write in flight: `Link::next_report` may be asked for one.
    pub fn idle(&self) -> bool {
        self.in_flight.is_none()
    }

    /// Hand a report over; false when the writer is busy or gone.
    pub fn write(&mut self, report: [u8; REPORT_LEN], now: Instant) -> bool {
        if self.in_flight.is_some() {
            return false;
        }
        match self.cmds.try_send(WriterCmd::Write(report)) {
            Ok(()) => {
                self.in_flight = Some(now);
                true
            }
            Err(_) => false,
        }
    }

    /// Whatever the writer finished; clears the in-flight mark. A closed
    /// channel is noticed here (`lost`) rather than waited out: the thread
    /// ending without a `Written` is the one case where the hid loop would
    /// otherwise park on a `done` that can never become ready.
    pub fn drain(&mut self) -> Vec<Written> {
        let mut done = Vec::new();
        loop {
            match self.done.try_recv() {
                Ok(written) => done.push(written),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.gone = true;
                    break;
                }
            }
        }
        if !done.is_empty() {
            self.in_flight = None;
        }
        done
    }

    /// The thread ended without answering the write in flight (it panicked,
    /// or was never spawned): that report can never complete, so the link is
    /// down now instead of after `WRITE_STALL`.
    pub fn lost(&self) -> bool {
        self.gone && self.in_flight.is_some()
    }

    /// The in-flight write is older than `WRITE_STALL`.
    pub fn stalled(&self, now: Instant) -> bool {
        self.in_flight
            .is_some_and(|since| now.duration_since(since) >= WRITE_STALL)
    }

    /// Tell the writer to drop its handle and wait at most `budget` for the
    /// thread to end; `true` when it ended and was joined.
    ///
    /// Never `join()` blindly: the thread may be inside `hid_write`, which
    /// is synchronous and, on a hung BLE link, never returns — that join is
    /// what wedged `deckd-hid` on shutdown (SIGTERM never completed). The
    /// thread's end is watched through `done` disconnecting instead, which
    /// costs nothing on a healthy link (a real write is 60–360 ms) and is
    /// bounded on a hung one: at the budget the thread is abandoned, its
    /// `Written` goes nowhere and it exits when (if) the write returns.
    /// `Duration::ZERO` abandons a writer already known to be stalled.
    pub fn close(mut self, budget: Duration) -> bool {
        let _ = self.cmds.try_send(WriterCmd::Close);
        drop(self.cmds);
        let Some(join) = self.join.take() else {
            return true;
        };
        let deadline = Instant::now() + budget;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.done.recv_timeout(left) {
                // A write that returned on the way out; wait for the end.
                Ok(_) => continue,
                // The thread dropped its `done`: it is finished.
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    drop(join);
                    return false;
                }
            }
        }
        let _ = join.join();
        true
    }
}

/// Spawn `deckd-hid-write` around a sink of its own. The hid thread keeps
/// the read handle; this one is only ever written to (and drained).
pub fn spawn(sink: impl ReportSink) -> Writer {
    let ((cmds_tx, cmds_rx), (done_tx, done_rx)) = Writer::channels();
    let join = std::thread::Builder::new()
        .name("deckd-hid-write".into())
        .spawn(move || writer_main(sink, cmds_rx, done_tx))
        .ok();
    if join.is_none() {
        tracing::error!("cannot spawn the hid writer thread");
    }
    Writer::new(cmds_tx, done_rx, join)
}

/// The production writer: a second, non-exclusive open of the deck.
pub fn spawn_device(device: hidapi::HidDevice) -> Writer {
    spawn(DeviceSink(device))
}

/// One report at a time, blocking on the write (the BLE round trip) and
/// reporting how long it took. Ends on `Close`, on either channel closing,
/// on a device error, or when the hid loop abandons it after a stall.
fn writer_main(mut sink: impl ReportSink, cmds: Receiver<WriterCmd>, done: Sender<Written>) {
    while let Ok(cmd) = cmds.recv() {
        let WriterCmd::Write(report) = cmd else {
            break;
        };
        let started = Instant::now();
        let result = sink.write(&report);
        let took = started.elapsed();
        let ok = match result {
            Ok(n) if n == report.len() => true,
            Ok(n) => {
                tracing::warn!(n, "short HID write");
                true
            }
            Err(WriteFailed) => false,
        };
        if done.send(Written { took, ok }).is_err() || !ok {
            break;
        }
        sink.drain_input();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A sink the test drives: each write waits for a `go` token, so a
    /// "hung BLE write" costs the test nothing but a parked thread it
    /// releases at the end.
    pub(crate) struct GatedSink {
        pub go: Receiver<Result<usize, WriteFailed>>,
        pub wrote: Sender<[u8; REPORT_LEN]>,
    }

    impl ReportSink for GatedSink {
        fn write(&mut self, report: &[u8; REPORT_LEN]) -> Result<usize, WriteFailed> {
            let _ = self.wrote.send(*report);
            self.go.recv().unwrap_or(Err(WriteFailed))
        }
    }

    /// `(writer, release, wrote)`: send one result per write to let it
    /// return, and read what the thread was handed.
    pub(crate) fn gated() -> (
        Writer,
        Sender<Result<usize, WriteFailed>>,
        Receiver<[u8; REPORT_LEN]>,
    ) {
        let (go_tx, go_rx) = crossbeam_channel::unbounded();
        let (wrote_tx, wrote_rx) = crossbeam_channel::unbounded();
        (
            spawn(GatedSink {
                go: go_rx,
                wrote: wrote_tx,
            }),
            go_tx,
            wrote_rx,
        )
    }

    #[test]
    fn a_writer_is_idle_until_a_write_is_in_flight_and_stalls_after_five_seconds() {
        let ((cmd_tx, cmd_rx), (done_tx, done_rx)) = Writer::channels();
        let mut w = Writer::new(cmd_tx, done_rx, None);
        let t0 = Instant::now();
        assert!(w.idle());
        assert!(!w.stalled(t0));
        assert!(w.write([6; 64], t0));
        assert!(!w.idle());
        assert!(!w.write([6; 64], t0), "one in flight at a time");
        assert_eq!(cmd_rx.try_recv(), Ok(WriterCmd::Write([6; 64])));
        assert!(!w.stalled(t0 + WRITE_STALL - Duration::from_millis(1)));
        assert!(w.stalled(t0 + WRITE_STALL));
        done_tx
            .send(Written {
                took: Duration::from_millis(80),
                ok: true,
            })
            .unwrap();
        let done = w.drain();
        assert_eq!(done.len(), 1);
        assert!(done[0].ok && done[0].took == Duration::from_millis(80));
        assert!(w.idle());
        assert!(w.drain().is_empty());
        assert!(w.close(CLOSE_BUDGET), "no thread to wait for");
        assert_eq!(cmd_rx.try_recv(), Ok(WriterCmd::Close));
        assert!(cmd_rx.try_recv().is_err(), "sender dropped");
    }

    /// The thread itself: one report at a time, `Written` per write, a
    /// device error ends it, and `Close` joins.
    #[test]
    fn the_thread_writes_one_report_at_a_time_and_ends_on_an_error() {
        let (mut w, go, wrote) = gated();
        let t0 = Instant::now();
        assert!(w.write([1; 64], t0));
        assert_eq!(
            wrote.recv_timeout(Duration::from_secs(5)).unwrap(),
            [1u8; 64]
        );
        assert!(w.drain().is_empty(), "still blocked in the sink");
        go.send(Ok(REPORT_LEN)).unwrap();
        let done = loop {
            let done = w.drain();
            if !done.is_empty() {
                break done;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(done.len(), 1);
        assert!(done[0].ok);
        assert!(w.idle());
        // A device error is reported once and ends the thread.
        assert!(w.write([2; 64], t0));
        assert_eq!(
            wrote.recv_timeout(Duration::from_secs(5)).unwrap(),
            [2u8; 64]
        );
        go.send(Err(WriteFailed)).unwrap();
        let done = loop {
            let done = w.drain();
            if !done.is_empty() {
                break done;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(!done[0].ok);
        assert!(
            w.close(CLOSE_BUDGET),
            "the thread ended on the failed write"
        );
    }

    /// Shutdown is bounded: `hid_write` is synchronous, so a thread inside a
    /// hung BLE write can neither see `Close` nor be joined, and the join
    /// that waited for it wedged `deckd-hid` — `deckd run` then never
    /// answered SIGTERM. `close` waits at most its budget and abandons the
    /// thread. The fake sink here never returns from `write` at all.
    #[test]
    fn closing_a_writer_whose_write_never_returns_is_bounded_and_abandons_it() {
        let (mut w, go, wrote) = gated();
        assert!(w.write([9; REPORT_LEN], Instant::now()));
        assert_eq!(
            wrote.recv_timeout(Duration::from_secs(5)).unwrap(),
            [9u8; REPORT_LEN],
            "the thread is inside the sink and will not come out"
        );
        // `close` runs on a thread of its own so that a close that does not
        // return fails this test instead of hanging the whole suite.
        let budget = Duration::from_millis(100);
        let (closed_tx, closed_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let started = Instant::now();
            let joined = w.close(budget);
            let _ = closed_tx.send((joined, started.elapsed()));
        });
        let (joined, took) = closed_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("close must return: a hung write cannot wedge the shutdown");
        assert!(!joined, "the thread is abandoned, not joined");
        assert!(took >= budget, "the budget is spent before abandoning");
        assert!(took < Duration::from_secs(5), "bounded, took {took:?}");
        // Release the abandoned thread so the test leaves nothing parked.
        let _ = go.send(Ok(REPORT_LEN));
    }

    /// The healthy case is unchanged: a write still in flight when the close
    /// comes is waited for inside the budget and the thread is joined.
    #[test]
    fn closing_a_writer_waits_for_a_write_that_does_return_and_joins() {
        let (mut w, go, wrote) = gated();
        assert!(w.write([4; REPORT_LEN], Instant::now()));
        assert!(wrote.recv_timeout(Duration::from_secs(5)).is_ok());
        // The write returns just as the close does: `done` carries a
        // `Written` before the disconnect, and the close must not stop there.
        go.send(Ok(REPORT_LEN)).unwrap();
        let started = Instant::now();
        assert!(w.close(CLOSE_BUDGET), "the thread ended and was joined");
        assert!(started.elapsed() < CLOSE_BUDGET, "no budget was spent");
    }
}
