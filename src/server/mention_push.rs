//! mention-pushes-say-what: a team lead that hands work on with an @mention in its team-channel
//! reply pushes who got the task ("Lead handed “Find caterers” to Researcher"), as a hand-off with
//! the `delegate` tool does (pushes-say-what), not "Lead replied", which read as "done already?".
//!
//! The lead's `TurnEnded` is sent when its turn ends, and only then does `delegate_mentions` create
//! the mention hand-offs, so the notifier would find none. The one who runs the lead's turn holds a
//! [`Gate`]; the turn, when it ends with a reply that @mentions someone, registers under its turn id
//! ([`Slot::turn_ended`]); the notifier waits for the gate (at most [`WAIT`]) before that turn's push
//! and reads the hand-offs it was given ([`take`]). Mention hand-offs keep no `lead_turn_id`: that
//! would hold their results while the turn runs and batch them, which changes when the lead hears them.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long a lead's push waits for its mention hand-offs. Creating them is a few database writes;
/// past this the push goes out with what exists (none: "replied", as before).
pub(crate) const WAIT: Duration = Duration::from_secs(20);

/// A registered turn whose push never read it (a quiet turn, a lagged notifier) is dropped after this.
const KEEP: Duration = Duration::from_secs(600);

struct Shared {
    /// The hand-offs the lead's mentions started (delegation ids), once they exist.
    ids: Mutex<Vec<String>>,
    /// True once the hand-offs are made (or there are none).
    done: tokio::sync::watch::Sender<bool>,
}

/// Turn id → when it was registered and its gate.
type Pending = HashMap<String, (Instant, Arc<Shared>)>;

/// Turns whose push waits for their mention hand-offs.
static PENDING: LazyLock<Mutex<Pending>> = LazyLock::new(Default::default);

/// Held by whoever runs a team lead's turn and then its hand-offs: [`Gate::handed`] (or dropping it,
/// however the turn and its hand-offs ended) lets the turn's push go out.
pub(crate) struct Gate(Arc<Shared>);

/// The [`Gate`]'s side a turn carries (`TurnOpts::mention_push`).
#[derive(Clone)]
pub(crate) struct Slot(Arc<Shared>);

impl Gate {
    pub(crate) fn new() -> Self {
        Gate(Arc::new(Shared { ids: Mutex::new(Vec::new()), done: tokio::sync::watch::channel(false).0 }))
    }

    pub(crate) fn slot(&self) -> Slot {
        Slot(self.0.clone())
    }

    /// The lead's mentions started the hand-offs `ids` (none: it handed nothing on).
    pub(crate) fn handed(self, ids: Vec<String>) {
        if let Ok(mut held) = self.0.ids.lock() {
            *held = ids;
        }
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.0.done.send_replace(true);
    }
}

impl Slot {
    /// The lead's turn `turn_id` ended with `reply`. Only a reply that @mentions someone can hand
    /// work on, so only its push waits. Called before the turn's `TurnEnded` is sent.
    pub(crate) fn turn_ended(&self, turn_id: &str, reply: &str) {
        if !reply.contains('@') {
            return;
        }
        if let Ok(mut pending) = PENDING.lock() {
            pending.retain(|_, (at, _)| at.elapsed() < KEEP);
            pending.insert(turn_id.to_string(), (Instant::now(), self.0.clone()));
        }
    }
}

/// Whether turn `turn_id`'s push waits for the hand-offs its mentions start.
pub(crate) fn waits(turn_id: &str) -> bool {
    PENDING.lock().is_ok_and(|p| p.contains_key(turn_id))
}

/// Waits until turn `turn_id`'s mention hand-offs exist (or it has none), at most [`WAIT`].
pub(crate) async fn settled(turn_id: &str) {
    settled_within(turn_id, WAIT).await
}

async fn settled_within(turn_id: &str, wait: Duration) {
    let Some(shared) = PENDING.lock().ok().and_then(|p| p.get(turn_id).map(|(_, s)| s.clone())) else {
        return;
    };
    let mut done = shared.done.subscribe();
    let _ = tokio::time::timeout(wait, done.wait_for(|d| *d)).await;
}

/// The hand-offs turn `turn_id`'s mentions started, read once (the turn is forgotten).
pub(crate) fn take(turn_id: &str) -> Vec<String> {
    let shared = PENDING.lock().ok().and_then(|mut p| p.remove(turn_id)).map(|(_, s)| s);
    shared.and_then(|s| s.ids.lock().ok().map(|ids| ids.clone())).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_push_waits_for_the_hand_offs_and_reads_them_once() {
        let gate = Gate::new();
        gate.slot().turn_ended("mp-turn-1", "On it. @Researcher, find caterers");
        assert!(waits("mp-turn-1"));
        let waiting = tokio::spawn(async {
            settled("mp-turn-1").await;
            take("mp-turn-1")
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "the push waits until the hand-offs exist");
        gate.handed(vec!["d-1".into()]);
        assert_eq!(waiting.await.unwrap(), vec!["d-1".to_string()]);
        assert!(!waits("mp-turn-1") && take("mp-turn-1").is_empty(), "read once");
    }

    #[tokio::test]
    async fn a_reply_without_a_mention_never_waits() {
        let gate = Gate::new();
        gate.slot().turn_ended("mp-turn-2", "Sure, here is the plan.");
        assert!(!waits("mp-turn-2"));
        // settled returns at once for a turn that isn't waiting
        tokio::time::timeout(Duration::from_secs(1), settled("mp-turn-2")).await.unwrap();
    }

    #[tokio::test]
    async fn a_dropped_gate_lets_the_push_go_with_nothing_handed() {
        let gate = Gate::new();
        gate.slot().turn_ended("mp-turn-3", "Hi! @Researcher is here too");
        drop(gate);
        tokio::time::timeout(Duration::from_secs(1), settled("mp-turn-3")).await.unwrap();
        assert!(take("mp-turn-3").is_empty());
    }

    #[tokio::test]
    async fn a_gate_never_opened_holds_the_push_at_most_the_wait() {
        let gate = Gate::new();
        gate.slot().turn_ended("mp-turn-4", "@Researcher go");
        let wait = Duration::from_millis(200);
        let started = Instant::now();
        settled_within("mp-turn-4", wait).await;
        assert!(started.elapsed() >= wait && started.elapsed() < wait * 10, "{:?}", started.elapsed());
        assert!(take("mp-turn-4").is_empty());
        drop(gate);
    }
}
