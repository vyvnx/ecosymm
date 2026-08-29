//! the current run, retained once and fanned out to every viewer.
//!
//! two things live here: an immutable bundle describing the run *right now*,
//! and a bounded broadcast of the changes to it. the bundle replaces fields in
//! place and never appends, so a server that runs forever holds the same
//! amount of memory on day 30 as on day 1.
//!
//! late joiners are the reason the two are locked together. a subscriber
//! subscribes first and clones the bundle second, under the same lock that
//! publishing takes, then discards everything it already received. without
//! that order a run could advance in the gap between the two and the viewer
//! would never learn about it.

use axum::extract::ws::Message;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use tokio::sync::broadcast;

/// finished messages in flight per viewer. a viewer that falls this far behind
/// is resynchronised from the retained bundle rather than waited for, so one
/// slow browser cannot slow the simulation.
const BROADCAST_DEPTH: usize = 256;

/// which retained field a message replaces
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    Config,
    World,
    Report,
    Snapshot,
    /// the whole bounded event ring, republished whenever it grows. it is the
    /// feed itself rather than a delta, so a reconnecting viewer merges by
    /// event id and never rebuilds a conflicting local history.
    Telemetry,
    Market,
    Result,
}

/// everything a viewer needs to be caught up, and nothing that grows
#[derive(Clone, Default)]
pub struct Bundle {
    pub run_id: i64,
    /// the sequence number of the newest change already folded in here
    pub seq: u64,
    pub config: Option<Message>,
    pub world: Option<Message>,
    pub report: Option<Message>,
    pub snapshot: Option<Message>,
    pub telemetry: Option<Message>,
    pub market: Option<Message>,
    pub result: Option<Message>,
}

impl Bundle {
    /// the bootstrap, in the order a browser can apply it: what the world is
    /// before what is in it, and the market last so it can be read against a
    /// run the client already knows about.
    pub fn bootstrap(&self) -> Vec<Message> {
        [
            &self.config,
            &self.world,
            &self.report,
            &self.snapshot,
            &self.telemetry,
            &self.market,
            &self.result,
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
    }
}

/// one published change
#[derive(Clone)]
pub struct Item {
    pub run_id: i64,
    pub seq: u64,
    pub message: Message,
}

/// a private invalidation. carries a revision and never a balance: account
/// state is refetched over the authenticated http api, never broadcast.
#[derive(Clone, Copy)]
pub struct AccountChanged {
    pub account_id: i64,
    pub revision: i64,
}

pub struct Hub {
    retained: RwLock<Bundle>,
    seq: AtomicU64,
    live: broadcast::Sender<Item>,
    accounts: broadcast::Sender<AccountChanged>,
}

impl Default for Hub {
    fn default() -> Self {
        Hub {
            retained: RwLock::new(Bundle::default()),
            seq: AtomicU64::new(0),
            live: broadcast::channel(BROADCAST_DEPTH).0,
            accounts: broadcast::channel(BROADCAST_DEPTH).0,
        }
    }
}

impl Hub {
    pub fn subscribe(&self) -> broadcast::Receiver<Item> {
        self.live.subscribe()
    }

    pub fn subscribe_accounts(&self) -> broadcast::Receiver<AccountChanged> {
        self.accounts.subscribe()
    }

    /// clone the retained bundle. always called *after* subscribing, never
    /// before: the seq it carries is what tells the caller which queued
    /// messages it has already been given.
    pub fn bundle(&self) -> Bundle {
        self.retained.read().expect("hub lock").clone()
    }

    /// a market opens before its run exists, so the last run's world and
    /// obituary stay retained through the betting window - that is what a
    /// viewer joining between runs has to look at. the new run clears them
    /// when its config lands, not here.
    pub fn open_run(&self, run_id: i64, market: Message) -> u64 {
        let mut retained = self.retained.write().expect("hub lock");
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        retained.run_id = run_id;
        retained.seq = seq;
        retained.market = Some(market.clone());
        let _ = self.live.send(Item { run_id, seq, message: market });
        seq
    }

    /// retain first, then publish, both under the write lock. that is what
    /// closes the gap between a viewer's snapshot and its subscription.
    pub fn publish(&self, slot: Slot, message: Message) -> u64 {
        let mut retained = self.retained.write().expect("hub lock");
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        retained.seq = seq;
        // a new run's config is where the last one ends: nothing it drew may
        // survive into the new world, or a viewer would see two at once
        if slot == Slot::Config {
            retained.world = None;
            retained.report = None;
            retained.snapshot = None;
            retained.telemetry = None;
            retained.result = None;
        }
        let field = match slot {
            Slot::Config => &mut retained.config,
            Slot::World => &mut retained.world,
            Slot::Report => &mut retained.report,
            Slot::Snapshot => &mut retained.snapshot,
            Slot::Telemetry => &mut retained.telemetry,
            Slot::Market => &mut retained.market,
            Slot::Result => &mut retained.result,
        };
        *field = Some(message.clone());
        let _ = self.live.send(Item { run_id: retained.run_id, seq, message });
        seq
    }

    /// tell every live socket for one account that its state moved. the
    /// devices then refetch it themselves over the authenticated api.
    pub fn account_changed(&self, account_id: i64, revision: i64) {
        let _ = self.accounts.send(AccountChanged { account_id, revision });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast::error::TryRecvError;

    fn message(text: &str) -> Message {
        Message::Text(text.into())
    }

    fn texts(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .map(|m| match m {
                Message::Text(t) => t.to_string(),
                other => panic!("expected text, got {other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn the_retained_bundle_replaces_fields_and_never_grows() {
        let hub = Hub::default();
        hub.open_run(1, message("market"));
        for epoch in 0..1_000 {
            hub.publish(Slot::Report, message(&format!("epoch {epoch}")));
            hub.publish(Slot::Snapshot, message(&format!("snapshot {epoch}")));
        }
        let bundle = hub.bundle();
        assert_eq!(
            texts(&bundle.bootstrap()),
            vec!["epoch 999", "snapshot 999", "market"],
            "the bundle kept history"
        );
        assert_eq!(bundle.run_id, 1);
    }

    /// the last run stays on screen through the next betting window, and is
    /// wiped by the new run's config rather than by the new market
    #[tokio::test]
    async fn the_last_run_survives_the_betting_window_and_no_longer() {
        let hub = Hub::default();
        hub.open_run(1, message("market 1"));
        hub.publish(Slot::Config, message("config 1"));
        hub.publish(Slot::World, message("world 1"));
        hub.publish(Slot::Result, message("result 1"));

        hub.open_run(2, message("market 2"));
        let between = hub.bundle();
        assert_eq!(
            texts(&between.bootstrap()),
            vec!["config 1", "world 1", "market 2", "result 1"],
            "a viewer joining between runs has nothing to look at"
        );
        assert_eq!(between.run_id, 2);

        hub.publish(Slot::Config, message("config 2"));
        assert_eq!(texts(&hub.bundle().bootstrap()), vec!["config 2", "market 2"]);
    }

    /// the join race: subscribe, then clone, then drop what the clone already
    /// contains. a change published in the gap has to survive that.
    #[tokio::test]
    async fn a_viewer_joining_mid_run_sees_every_change_exactly_once() {
        let hub = Hub::default();
        hub.open_run(1, message("market"));
        hub.publish(Slot::World, message("world"));

        let mut rx = hub.subscribe();
        // a change lands between the subscription and the snapshot
        hub.publish(Slot::Report, message("epoch 1"));
        let bundle = hub.bundle();
        hub.publish(Slot::Report, message("epoch 2"));

        assert!(texts(&bundle.bootstrap()).contains(&"epoch 1".to_string()));
        let mut live = Vec::new();
        while let Ok(item) = rx.try_recv() {
            if item.seq > bundle.seq {
                live.push(item.message);
            }
        }
        assert_eq!(texts(&live), vec!["epoch 2"], "epoch 1 was delivered twice or lost");
    }

    #[tokio::test]
    async fn a_viewer_that_falls_far_enough_behind_is_told_rather_than_waited_for() {
        let hub = Hub::default();
        let mut rx = hub.subscribe();
        hub.open_run(1, message("market"));
        for epoch in 0..BROADCAST_DEPTH + 10 {
            hub.publish(Slot::Report, message(&format!("epoch {epoch}")));
        }
        assert!(
            matches!(rx.try_recv(), Err(TryRecvError::Lagged(_))),
            "the queue grew instead of dropping the slow viewer's history"
        );
        // and the newest state is still there to resynchronise from
        assert_eq!(
            texts(&hub.bundle().bootstrap()),
            vec![format!("epoch {}", BROADCAST_DEPTH + 9), "market".into()]
        );
    }

    #[tokio::test]
    async fn an_account_invalidation_carries_a_revision_and_nothing_else() {
        let hub = Hub::default();
        let mut rx = hub.subscribe_accounts();
        hub.account_changed(7, 42);
        let changed = rx.try_recv().unwrap();
        assert_eq!((changed.account_id, changed.revision), (7, 42));
    }
}
