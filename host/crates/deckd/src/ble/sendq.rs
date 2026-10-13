//! The speaker half's outbox: a bounded FIFO between whoever produces
//! frames and CoreBluetooth's flow control.
//!
//! A write without response has no back-pressure of its own. CoreBluetooth
//! takes a few in flight, then `canSendWriteWithoutResponse` turns false and
//! a write issued anyway is dropped without a word; it turns true again with
//! `peripheralIsReadyToSendWriteWithoutResponse:`. So frames wait here, and
//! [`SendQueue::drain`] writes them only while the peripheral says it can
//! take one — once when a frame is queued and again on every "ready".
//!
//! The queue is bounded at [`SendQueue::CAPACITY`] frames (1.28 s of 20 ms
//! audio) and refuses the newest frame when full rather than growing: audio
//! that is a second late is already a gap on the deck, and latency that grows
//! without bound never recovers. Every refused or discarded frame is counted.
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct SendQueue {
    frames: VecDeque<Box<[u8]>>,
    capacity: usize,
    sent: u64,
    dropped: u64,
}

impl Default for SendQueue {
    fn default() -> Self {
        SendQueue::new(SendQueue::CAPACITY)
    }
}

impl SendQueue {
    /// 64 frames: 1.28 s of 20 ms frames.
    pub const CAPACITY: usize = 64;

    #[must_use]
    pub fn new(capacity: usize) -> SendQueue {
        SendQueue {
            frames: VecDeque::with_capacity(capacity),
            capacity,
            sent: 0,
            dropped: 0,
        }
    }

    /// Queue one frame. `false` (and one more dropped) when full.
    pub fn push(&mut self, frame: &[u8]) -> bool {
        if self.frames.len() >= self.capacity {
            self.dropped += 1;
            return false;
        }
        self.frames.push_back(frame.into());
        true
    }

    /// Write frames, oldest first, for as long as `can_send` says the link
    /// takes one (it is asked before every write). Returns how many went.
    pub fn drain(
        &mut self,
        mut can_send: impl FnMut() -> bool,
        mut write: impl FnMut(&[u8]),
    ) -> usize {
        let mut written = 0;
        while !self.frames.is_empty() && can_send() {
            if let Some(frame) = self.frames.pop_front() {
                write(&frame);
                written += 1;
            }
        }
        self.sent += written as u64;
        written
    }

    /// Discard everything queued (the link went away); the frames count as
    /// dropped. Returns how many there were.
    pub fn clear(&mut self) -> usize {
        let n = self.frames.len();
        self.frames.clear();
        self.dropped += n as u64;
        n
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Frames handed to the link so far.
    #[must_use]
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Frames refused because the queue was full, or discarded by `clear`.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_go_out_in_order_only_while_the_link_takes_them() {
        let mut queue = SendQueue::new(8);
        for i in 0..5u8 {
            assert!(queue.push(&[i, i]));
        }
        let mut out = Vec::new();
        // The link takes two, then says it is busy.
        let mut budget = 2;
        let n = queue.drain(
            || {
                let ok = budget > 0;
                budget -= i32::from(ok);
                ok
            },
            |f| out.push(f.to_vec()),
        );
        assert_eq!(n, 2);
        assert_eq!(out, [vec![0, 0], vec![1, 1]]);
        assert_eq!(queue.len(), 3);
        // "Ready to send" again: the rest, still in order.
        let n = queue.drain(|| true, |f| out.push(f.to_vec()));
        assert_eq!(n, 3);
        assert_eq!(
            out.iter().map(|f| f[0]).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        assert!(queue.is_empty());
        assert_eq!((queue.sent(), queue.dropped()), (5, 0));
        // Nothing queued: `can_send` is not even asked.
        let n = queue.drain(|| panic!("asked with nothing to send"), |_| {});
        assert_eq!(n, 0);
    }

    #[test]
    fn a_full_queue_refuses_the_newest_frame_and_counts_it() {
        let mut queue = SendQueue::default();
        assert_eq!(queue.capacity(), 64);
        for i in 0..64u8 {
            assert!(queue.push(&[i]));
        }
        assert!(!queue.push(&[64]));
        assert!(!queue.push(&[65]));
        assert_eq!((queue.len(), queue.dropped()), (64, 2));
        let mut first = None;
        queue.drain(
            || true,
            |f| {
                first.get_or_insert(f[0]);
            },
        );
        assert_eq!(first, Some(0), "the oldest frames were kept");
        assert!(queue.push(&[66]), "room again once drained");
    }

    #[test]
    fn clearing_counts_what_was_discarded() {
        let mut queue = SendQueue::new(4);
        queue.push(&[1]);
        queue.push(&[2, 3]);
        assert_eq!(queue.clear(), 2);
        assert!(queue.is_empty());
        assert_eq!((queue.sent(), queue.dropped()), (0, 2));
        assert_eq!(queue.clear(), 0);
    }
}
