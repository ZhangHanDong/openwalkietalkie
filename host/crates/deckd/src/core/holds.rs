//! Held cards inside the core (design §4.8 `hold`, contract §3.2): the
//! parked hook response of a `SetCardHeld`, keyed `(agent, card)` because
//! card ids are per slot. `Effect::Deliver{Hold}` resolves it with the
//! answer, `Effect::ReleaseHold` with the reason; a dropped `HoldGuard`
//! arrives as `HoldGone`. Week 2 (B1) drives these from the effects.
//!
//! A key is **re-used**: §5's degraded re-push writes the same card id, and
//! an adapter that re-pushes a still-held card parks a second time on the
//! same `(agent, card)`. So a park hands back a [`HoldToken`] and a guard
//! reports with it: whoever says "my hold is gone" has to say *which* hold
//! (B6).
use super::handle::HoldResult;
use deck_model::{AgentId, Answer, CardId, Release};
use std::collections::HashMap;
use tokio::sync::oneshot;

/// Which park a hold is. `(agent, card)` is not enough: a same-id re-push
/// parks again under the same key, and the first handler's guard must not
/// be able to release the hold that replaced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HoldToken(u64);

#[derive(Debug, Default)]
pub struct Holds {
    parked: HashMap<(AgentId, CardId), (HoldToken, oneshot::Sender<HoldResult>)>,
    seq: u64,
}

impl Holds {
    pub fn new() -> Self {
        Holds::default()
    }

    /// Park a hold; the receiver goes to the hook handler, the token to its
    /// guard.
    ///
    /// A second park on the same `(agent, card)` **releases** the handler it
    /// displaces with `HoldGone` rather than dropping its sender: a bare
    /// `RecvError` is indistinguishable from the core going away, so that
    /// handler would answer `{}` with an armed guard and its `HoldGone`
    /// would then take the new hold down with it — both hooks unanswered
    /// and the card degraded underneath a live one.
    pub fn park(
        &mut self,
        agent: AgentId,
        card: CardId,
    ) -> (HoldToken, oneshot::Receiver<HoldResult>) {
        self.seq += 1;
        let token = HoldToken(self.seq);
        let (tx, rx) = oneshot::channel();
        if let Some((displaced, sender)) = self.parked.insert((agent, card), (token, tx)) {
            tracing::debug!(
                ?displaced,
                ?token,
                "a second park on one card: releasing the first"
            );
            let _ = sender.send(HoldResult::Released {
                reason: Release::HoldGone,
            });
        }
        (token, rx)
    }

    pub fn contains(&self, agent: &AgentId, card: CardId) -> bool {
        self.parked.contains_key(&(agent.clone(), card))
    }

    /// The answer reaches the parked handler; false when nobody is parked.
    pub fn answer(&mut self, agent: &AgentId, card: CardId, answer: Answer) -> bool {
        match self.parked.remove(&(agent.clone(), card)) {
            Some((_, tx)) => tx.send(HoldResult::Answer(answer)).is_ok(),
            None => false,
        }
    }

    /// The hold is released without a decision; false when nobody is parked.
    pub fn release(&mut self, agent: &AgentId, card: CardId, reason: Release) -> bool {
        match self.parked.remove(&(agent.clone(), card)) {
            Some((_, tx)) => tx.send(HoldResult::Released { reason }).is_ok(),
            None => false,
        }
    }

    /// The handler went away: forget **its** hold. False when the key now
    /// holds somebody else's park (or none) — a stale report the core must
    /// not act on.
    pub fn forget(&mut self, agent: &AgentId, card: CardId, token: HoldToken) -> bool {
        let key = (agent.clone(), card);
        match self.parked.get(&key) {
            Some((parked, _)) if *parked == token => {
                self.parked.remove(&key);
                true
            }
            _ => false,
        }
    }

    /// A caller may cancel while the SetCardHeld reply is still in flight,
    /// before CoreHandle can construct its HoldGuard. The receiver closing
    /// is authoritative even in that gap. Only inspect the current park, so
    /// a cancelled receiver from a replaced hold cannot release its successor.
    pub fn take_abandoned(&mut self) -> Vec<(AgentId, CardId)> {
        let mut abandoned = Vec::new();
        self.parked.retain(|key, (_, sender)| {
            if sender.is_closed() {
                abandoned.push(key.clone());
                false
            } else {
                true
            }
        });
        abandoned
    }

    pub fn len(&self) -> usize {
        self.parked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parked.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_model::{AnswerSource, Principal};

    fn answer() -> Answer {
        Answer {
            choice: Some(2),
            label: Some("Deny".into()),
            text: None,
            source: AnswerSource::Deck,
            principal: Principal::Device,
            session: None,
            utterance: None,
        }
    }

    #[test]
    fn holds_resolve_with_an_answer_or_a_release_and_are_keyed_per_agent() {
        let mut holds = Holds::new();
        let a = AgentId("ag_a".into());
        let b = AgentId("ag_b".into());
        let (_, rx_a) = holds.park(a.clone(), CardId(1));
        let (_, rx_b) = holds.park(b.clone(), CardId(1));
        assert_eq!(holds.len(), 2);
        assert!(holds.contains(&a, CardId(1)) && !holds.contains(&a, CardId(2)));
        assert!(holds.answer(&a, CardId(1), answer()));
        assert!(!holds.answer(&a, CardId(1), answer()), "resolved once");
        assert!(matches!(
            rx_a.blocking_recv().unwrap(),
            HoldResult::Answer(Answer {
                choice: Some(2),
                ..
            })
        ));
        assert!(holds.release(&b, CardId(1), Release::Snooze));
        assert!(matches!(
            rx_b.blocking_recv().unwrap(),
            HoldResult::Released {
                reason: Release::Snooze
            }
        ));
        assert!(holds.is_empty());
        let (_, rx) = holds.park(a.clone(), CardId(2));
        drop(rx);
        assert!(
            !holds.answer(&a, CardId(2), answer()),
            "the handler is gone"
        );
        let (token, kept) = holds.park(a.clone(), CardId(3));
        assert!(holds.forget(&a, CardId(3), token));
        drop(kept);
        assert!(!holds.forget(&a, CardId(3), token));
    }

    /// B6: `park` was `HashMap::insert`, so a second park on one
    /// `(agent, card)` — the degraded same-id re-push of §5 is exactly that
    /// shape — silently dropped the first sender. Handler A saw a bare
    /// `RecvError`, answered `{}` with its guard still armed, and the
    /// `HoldGone` that guard sent removed **handler B's** hold: two hooks
    /// unanswered and a card degraded under a live one.
    #[test]
    fn a_second_park_releases_the_handler_it_displaces_and_keeps_its_own() {
        let mut holds = Holds::new();
        let a = AgentId("ag_a".into());
        let (first_token, first) = holds.park(a.clone(), CardId(1));
        let (second_token, second) = holds.park(a.clone(), CardId(1));
        assert_ne!(first_token, second_token);
        assert_eq!(holds.len(), 1, "one card, one hold");

        // The displaced handler is told, not abandoned: it writes `{}` and
        // stops waiting, like any other release.
        assert!(matches!(
            first.blocking_recv().unwrap(),
            HoldResult::Released {
                reason: Release::HoldGone
            }
        ));
        // …and its guard cannot take the second handler's hold with it.
        assert!(!holds.forget(&a, CardId(1), first_token), "stale");
        assert!(holds.answer(&a, CardId(1), answer()));
        assert!(matches!(
            second.blocking_recv().unwrap(),
            HoldResult::Answer(_)
        ));
        assert!(holds.is_empty());
    }

    #[test]
    fn abandoned_receivers_do_not_release_live_replacements() {
        let mut holds = Holds::new();
        let agent = AgentId("ag_a".into());
        let (_, abandoned) = holds.park(agent.clone(), CardId(1));
        let (_, replaced) = holds.park(agent.clone(), CardId(2));
        let (_, live) = holds.park(agent.clone(), CardId(2));
        drop(abandoned);
        drop(replaced);
        assert_eq!(holds.take_abandoned(), vec![(agent.clone(), CardId(1))]);
        assert!(holds.contains(&agent, CardId(2)));
        assert!(holds.take_abandoned().is_empty());
        drop(live);
        assert_eq!(holds.take_abandoned(), vec![(agent, CardId(2))]);
        assert!(holds.is_empty());
    }
}
