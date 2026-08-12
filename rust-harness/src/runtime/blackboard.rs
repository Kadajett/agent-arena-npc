use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use arc_swap::{ArcSwap, ArcSwapOption};
use tokio::sync::watch;

use crate::{
    brain::{strategic_intent::StrategicIntent, tactical_frame::TacticalFrame},
    execution::packet::ActionPacket,
};

pub struct HotBlackboard {
    frame: ArcSwap<TacticalFrame>,
    strategy: ArcSwap<StrategicIntent>,
    current_packet: ArcSwapOption<ActionPacket>,
    perception_revision: AtomicU64,
    strategic_revision: AtomicU64,
    minimum_valid_frame_revision: AtomicU64,
    frame_revision_tx: watch::Sender<u64>,
}

impl HotBlackboard {
    pub fn new(initial_strategy: StrategicIntent) -> Self {
        let initial_strategic_revision = initial_strategy.revision;
        let frame = TacticalFrame::empty(initial_strategy.clone());
        let (frame_revision_tx, _frame_revision_rx) = watch::channel(0);
        Self {
            frame: ArcSwap::from_pointee(frame),
            strategy: ArcSwap::from_pointee(initial_strategy),
            current_packet: ArcSwapOption::empty(),
            perception_revision: AtomicU64::new(0),
            strategic_revision: AtomicU64::new(initial_strategic_revision),
            minimum_valid_frame_revision: AtomicU64::new(0),
            frame_revision_tx,
        }
    }

    pub fn frame(&self) -> Arc<TacticalFrame> {
        self.frame.load_full()
    }

    pub fn strategy(&self) -> Arc<StrategicIntent> {
        self.strategy.load_full()
    }

    pub fn current_packet(&self) -> Option<Arc<ActionPacket>> {
        self.current_packet.load_full()
    }

    pub fn publish_frame(&self, frame: Arc<TacticalFrame>) -> bool {
        if frame.perception_revision <= self.perception_revision() {
            return false;
        }
        self.perception_revision
            .store(frame.perception_revision, Ordering::Release);
        let revision = frame.revision;
        self.frame.store(frame);
        self.frame_revision_tx.send_replace(revision);
        true
    }

    pub fn publish_strategy(&self, strategy: Arc<StrategicIntent>) -> bool {
        if strategy.revision <= self.strategic_revision() {
            return false;
        }
        self.strategic_revision
            .store(strategy.revision, Ordering::Release);
        self.strategy.store(strategy);
        true
    }

    pub fn set_current_packet(&self, packet: Option<Arc<ActionPacket>>) {
        self.current_packet.store(packet);
    }

    pub fn invalidate_before(&self, revision: u64) {
        self.minimum_valid_frame_revision
            .fetch_max(revision, Ordering::AcqRel);
    }

    pub fn perception_revision(&self) -> u64 {
        self.perception_revision.load(Ordering::Acquire)
    }

    pub fn strategic_revision(&self) -> u64 {
        self.strategic_revision.load(Ordering::Acquire)
    }

    pub fn minimum_valid_frame_revision(&self) -> u64 {
        self.minimum_valid_frame_revision.load(Ordering::Acquire)
    }

    /// Subscribe to accepted tactical-frame publications.
    #[must_use]
    pub fn subscribe_frames(&self) -> watch::Receiver<u64> {
        self.frame_revision_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_out_of_order_frames() {
        let blackboard = HotBlackboard::new(StrategicIntent::default());
        let mut newer = TacticalFrame::empty(StrategicIntent::default());
        newer.revision = 10;
        newer.perception_revision = 2;
        assert!(blackboard.publish_frame(Arc::new(newer)));

        let mut older = TacticalFrame::empty(StrategicIntent::default());
        older.revision = 9;
        older.perception_revision = 1;
        assert!(!blackboard.publish_frame(Arc::new(older)));
        assert_eq!(blackboard.frame().revision, 10);
    }

    #[tokio::test]
    async fn accepted_frames_notify_subscribers() {
        let blackboard = HotBlackboard::new(StrategicIntent::default());
        let mut revisions = blackboard.subscribe_frames();
        let mut frame = TacticalFrame::empty(StrategicIntent::default());
        frame.revision = 7;
        frame.perception_revision = 1;

        assert!(blackboard.publish_frame(Arc::new(frame)));
        revisions.changed().await.expect("frame notification");
        assert_eq!(*revisions.borrow(), 7);
    }
}
