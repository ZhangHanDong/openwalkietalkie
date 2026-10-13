//! The event bus (design §2.3, §4.3; contract §2, §3.2): every event the
//! core publishes goes to a `broadcast` channel (the doors' fan-out, 4096
//! deep — a lagged subscriber gets a `resync`), into a ring of the last
//! 4096 events (`state.events{since}` and the hello's `since` replay), and
//! the snapshot cell is swapped whenever `rev` changed. Events are stored
//! once, unredacted; every reader redacts at fan-out (`redact_event`).
use arc_swap::ArcSwap;
use deck_model::{Event, EventKind, SessionId, Snapshot};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// The broadcast channel's depth and the ring's default length.
pub const BROADCAST_CAPACITY: usize = 4096;
pub const RING_CAPACITY: usize = 4096;
/// `state.events` returns at most this many events per call by default.
pub const REPLAY_LIMIT: usize = 500;

/// One event on the bus and the session that owns the agent it is about
/// (`None` when it is about no agent, or the agent is gone): the doors
/// compare it with their own session to keep an owner's content.
#[derive(Clone, Debug)]
pub struct Published {
    pub event: Arc<Event>,
    pub owner: Option<SessionId>,
}

struct Ring {
    events: VecDeque<Arc<Event>>,
    capacity: usize,
    /// The rev of the newest event the ring had to drop, if any.
    evicted: Option<u64>,
}

pub struct Bus {
    tx: broadcast::Sender<Published>,
    ring: Mutex<Ring>,
    snapshot: ArcSwap<Snapshot>,
}

impl Default for Bus {
    fn default() -> Self {
        Bus::new(RING_CAPACITY)
    }
}

impl Bus {
    /// A bus whose ring keeps `ring` events (`[privacy] event_ring`).
    pub fn new(ring: usize) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Bus {
            tx,
            ring: Mutex::new(Ring {
                events: VecDeque::with_capacity(ring.min(RING_CAPACITY)),
                capacity: ring.max(1),
                evicted: None,
            }),
            snapshot: ArcSwap::from_pointee(Snapshot::default()),
        }
    }

    /// Ring first (so a subscriber that wakes up can replay it), then the
    /// broadcast. No receiver is not an error.
    pub fn publish(&self, published: Published) {
        {
            let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
            if ring.events.len() >= ring.capacity {
                if let Some(old) = ring.events.pop_front() {
                    ring.evicted = Some(old.rev);
                }
            }
            ring.events.push_back(published.event.clone());
        }
        let _ = self.tx.send(published);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Published> {
        self.tx.subscribe()
    }

    /// The events after `since` (at most `limit`, in order), filtered by the
    /// type globs when given, and whether the ring no longer holds
    /// everything after `since` (`gap`: the caller should `state.get`).
    pub fn replay(
        &self,
        since: u64,
        limit: usize,
        types: Option<&[String]>,
    ) -> (Vec<Arc<Event>>, bool) {
        let ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        let gap = ring.evicted.is_some_and(|evicted| since < evicted);
        let events = ring
            .events
            .iter()
            .filter(|e| e.rev > since)
            .filter(|e| types.is_none_or(|types| matches_any(types, e.kind)))
            .take(limit)
            .cloned()
            .collect();
        (events, gap)
    }

    /// How many events the ring holds.
    pub fn ring_len(&self) -> usize {
        self.ring
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .events
            .len()
    }

    /// The current snapshot (unredacted; the reader redacts).
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.load_full()
    }

    pub fn rev(&self) -> u64 {
        self.snapshot.load().rev
    }

    /// The core swaps the cell after a step that changed `rev`.
    pub fn set_snapshot(&self, snapshot: Snapshot) {
        self.snapshot.store(Arc::new(snapshot));
    }

    /// Live subscribers (for tests and `metrics.get`).
    pub fn receivers(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// A subscription glob against an event type: `"*"` matches every kind but
/// the high-volume ones, `"card.*"` a prefix, anything else the exact name
/// (§4.5: high-volume kinds must be named).
pub fn type_matches(pattern: &str, kind: EventKind) -> bool {
    let name = kind.as_str();
    if pattern == "*" {
        return !kind.is_high_volume();
    }
    match pattern.strip_suffix(".*") {
        Some(prefix) => {
            !kind.is_high_volume()
                && name
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('.'))
        }
        None => pattern == name,
    }
}

/// Any of the globs matches.
pub fn matches_any(patterns: &[String], kind: EventKind) -> bool {
    patterns.iter().any(|p| type_matches(p, kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_model::{Principal, Ts};

    fn event(rev: u64, kind: EventKind) -> Arc<Event> {
        Arc::new(Event {
            rev,
            ts: Ts(rev),
            kind,
            principal: Principal::Uid(0),
            data: serde_json::json!({}),
        })
    }

    fn publish(bus: &Bus, rev: u64, kind: EventKind) {
        bus.publish(Published {
            event: event(rev, kind),
            owner: None,
        });
    }

    #[test]
    fn the_ring_replays_after_a_rev_and_reports_a_gap_once_it_dropped_events() {
        let bus = Bus::new(4);
        assert_eq!(bus.replay(0, 10, None), (Vec::new(), false));
        for rev in 1..=4 {
            publish(&bus, rev, EventKind::CardSet);
        }
        let (events, gap) = bus.replay(0, 10, None);
        assert_eq!(events.len(), 4);
        assert!(!gap);
        let (events, gap) = bus.replay(2, 10, None);
        assert_eq!(events.iter().map(|e| e.rev).collect::<Vec<_>>(), vec![3, 4]);
        assert!(!gap);
        assert_eq!(bus.replay(1, 1, None).0.len(), 1, "limit");
        // The fifth event evicts rev 1: `since: 0` has a gap, `since: 1` has not.
        publish(&bus, 5, EventKind::AgentChanged);
        assert_eq!(bus.ring_len(), 4);
        let (events, gap) = bus.replay(0, 10, None);
        assert_eq!(events.len(), 4);
        assert!(gap);
        assert!(!bus.replay(1, 10, None).1);
        assert!(bus.replay(9, 10, None).0.is_empty());
        // Type filters.
        let (events, _) = bus.replay(0, 10, Some(&["agent.*".to_string()]));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::AgentChanged);
    }

    #[test]
    fn subscribers_see_the_owner_and_the_snapshot_cell_swaps() {
        let bus = Bus::default();
        let mut rx = bus.subscribe();
        assert_eq!(bus.receivers(), 1);
        bus.publish(Published {
            event: event(1, EventKind::CardSet),
            owner: Some(SessionId(3)),
        });
        let got = rx.try_recv().unwrap();
        assert_eq!(got.event.rev, 1);
        assert_eq!(got.owner, Some(SessionId(3)));
        assert_eq!(bus.rev(), 0);
        bus.set_snapshot(Snapshot {
            rev: 7,
            ..Default::default()
        });
        assert_eq!(bus.rev(), 7);
        assert_eq!(bus.snapshot().rev, 7);
    }

    #[test]
    fn globs_match_prefixes_and_never_high_volume_kinds_by_wildcard() {
        assert!(type_matches("*", EventKind::CardSet));
        assert!(!type_matches("*", EventKind::TranscriptPartial));
        assert!(type_matches("card.*", EventKind::CardAnswered));
        assert!(!type_matches("card.*", EventKind::AgentAdded));
        assert!(!type_matches("card", EventKind::CardSet));
        assert!(type_matches(
            "transcript.partial",
            EventKind::TranscriptPartial
        ));
        assert!(!type_matches("transcript.*", EventKind::TranscriptPartial));
        assert!(type_matches("transcript.*", EventKind::TranscriptFinal));
        assert!(type_matches(
            "op.confirm_requested",
            EventKind::OpConfirmRequested
        ));
        assert!(matches_any(
            &["device.*".into(), "error".into()],
            EventKind::Error
        ));
        assert!(!matches_any(&[], EventKind::Error));
    }
}
