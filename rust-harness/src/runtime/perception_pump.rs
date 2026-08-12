use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use ractor::ActorRef;
use thiserror::Error;
use tokio::{
    sync::oneshot,
    task::{JoinError, JoinHandle},
    time::{Instant, MissedTickBehavior},
};
use uuid::Uuid;

use crate::{
    mcp::{
        ArenaGateway,
        client::GatewayError,
        observation::Observation,
        types::{InventoryResult, MapObservation},
    },
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel},
    runtime::{blackboard::HotBlackboard, messages::PerceptionMsg},
    world::perception::PerceptionInput,
};

/// The read-only world boundary used by the perception pump.
///
/// Production uses a character-bound [`ArenaGateway`]. The seam exists so the
/// concurrency and cancellation contract can be tested without a live MCP
/// session. It deliberately exposes no mutation operation.
#[async_trait]
pub trait PerceptionSource: Send + Sync {
    async fn observe(&self) -> Result<Observation, GatewayError>;
    async fn render_map(&self, radius: u32) -> Result<MapObservation, GatewayError>;
    async fn inventory(&self) -> Result<InventoryResult, GatewayError>;
}

#[async_trait]
impl PerceptionSource for ArenaGateway {
    async fn observe(&self) -> Result<Observation, GatewayError> {
        ArenaGateway::observe(self).await
    }

    async fn render_map(&self, radius: u32) -> Result<MapObservation, GatewayError> {
        ArenaGateway::render_map(self, radius).await
    }

    async fn inventory(&self) -> Result<InventoryResult, GatewayError> {
        ArenaGateway::inventory(self).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerceptionPumpConfig {
    pub interval: Duration,
    pub map_radius: u32,
    pub inventory_every_cycles: u64,
}

impl Default for PerceptionPumpConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            map_radius: 12,
            inventory_every_cycles: 10,
        }
    }
}

pub struct PerceptionPumpArgs {
    pub character_id: String,
    pub source: Arc<dyn PerceptionSource>,
    pub blackboard: Arc<HotBlackboard>,
    pub perception: ActorRef<PerceptionMsg>,
    pub analytics: Arc<dyn AnalyticsSink>,
    pub config: PerceptionPumpConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerceptionPumpExit {
    Shutdown,
    PerceptionActorUnavailable,
}

impl PerceptionPumpExit {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Shutdown => "shutdown",
            Self::PerceptionActorUnavailable => "perception_actor_unavailable",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PerceptionPumpStartError {
    #[error("perception interval must be greater than zero")]
    ZeroInterval,
    #[error("inventory refresh cadence must be greater than zero")]
    ZeroInventoryCadence,
}

/// A cancellable handle for one single-flight perception loop.
pub struct PerceptionPumpHandle {
    shutdown: oneshot::Sender<()>,
    join: JoinHandle<PerceptionPumpExit>,
}

impl PerceptionPumpHandle {
    /// Ask the pump to stop, cancel any in-flight read cycle, and wait for it.
    ///
    /// # Errors
    ///
    /// Returns a join error if the pump task panicked or was externally aborted.
    pub async fn shutdown(self) -> Result<PerceptionPumpExit, JoinError> {
        let _ = self.shutdown.send(());
        self.join.await
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.join.is_finished()
    }
}

/// Start one periodic perception task.
///
/// The first cycle starts immediately. Each cycle waits for the prior cycle to
/// finish, and missed interval ticks are skipped. This gives the pump
/// latest-value behavior without overlapping MCP reads or building a backlog.
///
/// # Errors
///
/// Returns [`PerceptionPumpStartError::ZeroInterval`] for an invalid interval.
pub fn start_perception_pump(
    args: PerceptionPumpArgs,
) -> Result<PerceptionPumpHandle, PerceptionPumpStartError> {
    if args.config.interval.is_zero() {
        return Err(PerceptionPumpStartError::ZeroInterval);
    }
    if args.config.inventory_every_cycles == 0 {
        return Err(PerceptionPumpStartError::ZeroInventoryCadence);
    }
    let (shutdown, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(run_pump(args, shutdown_rx));
    Ok(PerceptionPumpHandle { shutdown, join })
}

async fn run_pump(
    args: PerceptionPumpArgs,
    mut shutdown: oneshot::Receiver<()>,
) -> PerceptionPumpExit {
    let pump_id = Uuid::new_v4();
    record_pump_started(&args, pump_id);

    let mut ticker = tokio::time::interval_at(Instant::now(), args.config.interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut cycle_sequence = 0_u64;
    let exit = loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break PerceptionPumpExit::Shutdown,
            _ = ticker.tick() => {}
        }

        cycle_sequence = cycle_sequence.saturating_add(1);
        let cycle_id = Uuid::new_v4();
        let started = std::time::Instant::now();
        args.analytics.record(
            cycle_event(
                "perception.cycle_started",
                EventLevel::Debug,
                &args,
                pump_id,
                cycle_id,
                cycle_sequence,
                started,
            )
            .attribute("map_radius", args.config.map_radius),
        );

        let inventory_requested = inventory_due(cycle_sequence, args.config.inventory_every_cycles);
        let reads = read_cycle(
            args.source.as_ref(),
            args.config.map_radius,
            inventory_requested,
        );
        tokio::pin!(reads);
        let read_result = tokio::select! {
            biased;
            _ = &mut shutdown => {
                args.analytics.record(
                    cycle_event(
                        "perception.cycle_cancelled",
                        EventLevel::Info,
                        &args,
                        pump_id,
                        cycle_id,
                        cycle_sequence,
                        started,
                    )
                    .attribute("reason", "shutdown"),
                );
                break PerceptionPumpExit::Shutdown;
            }
            result = &mut reads => result,
        };

        let Some(input) = finish_reads(
            read_result,
            &args,
            pump_id,
            cycle_id,
            cycle_sequence,
            started,
        ) else {
            continue;
        };

        if args
            .perception
            .send_message(PerceptionMsg::Observation(Box::new(input)))
            .is_err()
        {
            args.analytics.record(
                cycle_event(
                    "perception.cycle_failed",
                    EventLevel::Error,
                    &args,
                    pump_id,
                    cycle_id,
                    cycle_sequence,
                    started,
                )
                .attribute("failure_stage", "delivery")
                .attribute("error_class", "perception_actor_unavailable"),
            );
            break PerceptionPumpExit::PerceptionActorUnavailable;
        }
    };

    args.analytics.record(
        AnalyticsEvent::new("perception.pump_stopped", EventLevel::Info)
            .character(&args.character_id)
            .correlation(pump_id)
            .attribute("pump_id", pump_id.to_string())
            .attribute("reason", exit.as_str())
            .attribute("cycles_started", cycle_sequence),
    );
    exit
}

fn record_pump_started(args: &PerceptionPumpArgs, pump_id: Uuid) {
    args.analytics.record(
        AnalyticsEvent::new("perception.pump_started", EventLevel::Info)
            .character(&args.character_id)
            .correlation(pump_id)
            .attribute("pump_id", pump_id.to_string())
            .attribute("interval_ms", duration_ms(args.config.interval))
            .attribute("map_radius", args.config.map_radius)
            .attribute("inventory_every_cycles", args.config.inventory_every_cycles),
    );
}

const fn inventory_due(cycle_sequence: u64, every_cycles: u64) -> bool {
    cycle_sequence > 0 && (cycle_sequence - 1).is_multiple_of(every_cycles)
}

struct CycleReads {
    observation: Result<Observation, GatewayError>,
    map: Result<MapObservation, GatewayError>,
    inventory: Option<Result<InventoryResult, GatewayError>>,
}

async fn read_cycle(
    source: &dyn PerceptionSource,
    map_radius: u32,
    inventory_requested: bool,
) -> CycleReads {
    let inventory = async {
        if inventory_requested {
            Some(source.inventory().await)
        } else {
            None
        }
    };
    let (observation, map, inventory) =
        tokio::join!(source.observe(), source.render_map(map_radius), inventory);
    CycleReads {
        observation,
        map,
        inventory,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "cycle identity stays explicit at the observability boundary"
)]
fn finish_reads(
    reads: CycleReads,
    args: &PerceptionPumpArgs,
    pump_id: Uuid,
    cycle_id: Uuid,
    cycle_sequence: u64,
    started: std::time::Instant,
) -> Option<PerceptionInput> {
    let observation_error = reads.observation.as_ref().err();
    let map_error = reads.map.as_ref().err();
    let inventory_requested = reads.inventory.is_some();
    let inventory_error_class = reads
        .inventory
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .map(gateway_error_class);
    if observation_error.is_some() || map_error.is_some() {
        let mut event = cycle_event(
            "perception.cycle_failed",
            EventLevel::Warn,
            args,
            pump_id,
            cycle_id,
            cycle_sequence,
            started,
        )
        .attribute("failure_stage", "read")
        .attribute("observation_succeeded", observation_error.is_none())
        .attribute("map_succeeded", map_error.is_none())
        .attribute("inventory_requested", inventory_requested)
        .attribute(
            "inventory_succeeded",
            inventory_requested && inventory_error_class.is_none(),
        );
        if let Some(error) = observation_error {
            event = event.attribute("observation_error_class", gateway_error_class(error));
        }
        if let Some(error) = map_error {
            event = event.attribute("map_error_class", gateway_error_class(error));
        }
        if let Some(error_class) = inventory_error_class {
            event = event.attribute("inventory_error_class", error_class);
        }
        args.analytics.record(event);
        return None;
    }

    let inventory = reads.inventory.and_then(Result::ok);
    let strategic_intent = args.blackboard.strategy().as_ref().clone();
    let strategic_revision = strategic_intent.revision;
    let mut event = cycle_event(
        "perception.cycle_completed",
        if inventory_error_class.is_none() {
            EventLevel::Debug
        } else {
            EventLevel::Warn
        },
        args,
        pump_id,
        cycle_id,
        cycle_sequence,
        started,
    )
    .attribute(
        "status",
        if inventory_error_class.is_none() {
            "complete"
        } else {
            "degraded"
        },
    )
    .attribute("inventory_requested", inventory_requested)
    .attribute("inventory_available", inventory.is_some())
    .attribute("strategic_revision", strategic_revision);
    if let Some(error_class) = inventory_error_class {
        event = event.attribute("inventory_error_class", error_class);
    }
    args.analytics.record(event);

    Some(PerceptionInput {
        observation_cycle_id: cycle_id,
        observation_cycle_sequence: cycle_sequence,
        observation: reads
            .observation
            .expect("mandatory observation was checked above"),
        map: reads.map.expect("mandatory map was checked above"),
        inventory,
        strategic_intent,
        observed_at: Utc::now(),
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "cycle identity stays explicit at the observability boundary"
)]
fn cycle_event(
    name: &'static str,
    level: EventLevel,
    args: &PerceptionPumpArgs,
    pump_id: Uuid,
    cycle_id: Uuid,
    cycle_sequence: u64,
    started: std::time::Instant,
) -> AnalyticsEvent {
    AnalyticsEvent::new(name, level)
        .character(&args.character_id)
        .correlation(cycle_id)
        .attribute("pump_id", pump_id.to_string())
        .attribute("observation_cycle_id", cycle_id.to_string())
        .attribute("observation_cycle_sequence", cycle_sequence)
        .attribute("duration_ms", duration_ms(started.elapsed()))
}

fn gateway_error_class(error: &GatewayError) -> &'static str {
    match error {
        GatewayError::MissingCapability { .. } => "missing_capability",
        GatewayError::Mcp(error) => error.class(),
        GatewayError::Decode { .. } => "decode",
        GatewayError::InvalidArguments { .. } => "invalid_arguments",
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{
        future::pending,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use ractor::{Actor, ActorProcessingErr};
    use tokio::sync::{Barrier, mpsc};

    use super::*;
    use crate::{
        brain::strategic_intent::StrategicIntent, mcp::transport::McpError,
        observability::RecordingAnalyticsSink,
    };

    struct InputCollector;

    impl Actor for InputCollector {
        type Msg = PerceptionMsg;
        type State = mpsc::UnboundedSender<PerceptionInput>;
        type Arguments = mpsc::UnboundedSender<PerceptionInput>;

        async fn pre_start(
            &self,
            _myself: ActorRef<Self::Msg>,
            args: Self::Arguments,
        ) -> Result<Self::State, ActorProcessingErr> {
            Ok(args)
        }

        async fn handle(
            &self,
            myself: ActorRef<Self::Msg>,
            message: Self::Msg,
            state: &mut Self::State,
        ) -> Result<(), ActorProcessingErr> {
            match message {
                PerceptionMsg::Observation(input) => {
                    let _ = state.send(*input);
                }
                PerceptionMsg::Shutdown => myself.stop(None),
                _ => {}
            }
            Ok(())
        }
    }

    struct ConcurrentSource {
        barrier: Barrier,
        calls: AtomicUsize,
    }

    impl ConcurrentSource {
        fn new() -> Self {
            Self {
                barrier: Barrier::new(3),
                calls: AtomicUsize::new(0),
            }
        }

        async fn arrive(&self) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.barrier.wait().await;
        }
    }

    #[async_trait]
    impl PerceptionSource for ConcurrentSource {
        async fn observe(&self) -> Result<Observation, GatewayError> {
            self.arrive().await;
            Ok(Observation::default())
        }

        async fn render_map(&self, radius: u32) -> Result<MapObservation, GatewayError> {
            self.arrive().await;
            Ok(MapObservation {
                requested_radius: Some(radius),
                ..MapObservation::default()
            })
        }

        async fn inventory(&self) -> Result<InventoryResult, GatewayError> {
            self.arrive().await;
            Ok(InventoryResult::default())
        }
    }

    async fn collector() -> (
        ActorRef<PerceptionMsg>,
        mpsc::UnboundedReceiver<PerceptionInput>,
    ) {
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (actor, _join) = Actor::spawn(None, InputCollector, input_tx)
            .await
            .expect("collector starts");
        (actor, input_rx)
    }

    fn blackboard(revision: u64) -> Arc<HotBlackboard> {
        Arc::new(HotBlackboard::new(StrategicIntent {
            revision,
            objective: "test the live world".to_owned(),
            ..StrategicIntent::default()
        }))
    }

    fn args(
        source: Arc<dyn PerceptionSource>,
        perception: ActorRef<PerceptionMsg>,
        analytics: Arc<dyn AnalyticsSink>,
        interval: Duration,
    ) -> PerceptionPumpArgs {
        PerceptionPumpArgs {
            character_id: "cassian".to_owned(),
            source,
            blackboard: blackboard(17),
            perception,
            analytics,
            config: PerceptionPumpConfig {
                interval,
                map_radius: 9,
                inventory_every_cycles: 10,
            },
        }
    }

    #[tokio::test]
    async fn reads_concurrently_and_sends_one_revisioned_input() {
        let source = Arc::new(ConcurrentSource::new());
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let (collector, mut inputs) = collector().await;
        let handle = start_perception_pump(args(
            source.clone(),
            collector.clone(),
            analytics.clone(),
            Duration::from_mins(1),
        ))
        .expect("valid pump");

        let input = tokio::time::timeout(Duration::from_secs(1), inputs.recv())
            .await
            .expect("three concurrent reads do not deadlock")
            .expect("input delivered");
        assert_eq!(source.calls.load(Ordering::SeqCst), 3);
        assert_eq!(input.map.requested_radius, Some(9));
        assert_eq!(input.strategic_intent.revision, 17);
        assert!(input.inventory.is_some());

        assert_eq!(
            handle.shutdown().await.expect("pump task"),
            PerceptionPumpExit::Shutdown
        );
        collector.stop(None);
        let events = analytics.events();
        let completed = events
            .iter()
            .find(|event| event.name == "perception.cycle_completed")
            .expect("completion is observable");
        assert_eq!(completed.character_id.as_deref(), Some("cassian"));
        assert_eq!(completed.correlation_id, Some(input.observation_cycle_id));
        assert_eq!(
            completed.attributes["observation_cycle_id"],
            input.observation_cycle_id.to_string()
        );
        assert_eq!(
            completed.attributes["observation_cycle_sequence"],
            input.observation_cycle_sequence
        );
        assert!(completed.attributes.contains_key("duration_ms"));
        assert_eq!(completed.attributes["status"], "complete");
    }

    struct InventoryFailureSource;

    #[async_trait]
    impl PerceptionSource for InventoryFailureSource {
        async fn observe(&self) -> Result<Observation, GatewayError> {
            Ok(Observation::default())
        }

        async fn render_map(&self, _radius: u32) -> Result<MapObservation, GatewayError> {
            Ok(MapObservation::default())
        }

        async fn inventory(&self) -> Result<InventoryResult, GatewayError> {
            Err(GatewayError::Mcp(McpError::Timeout { timeout_ms: 10 }))
        }
    }

    #[tokio::test]
    async fn inventory_failure_is_safe_degradation() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let (collector, mut inputs) = collector().await;
        let handle = start_perception_pump(args(
            Arc::new(InventoryFailureSource),
            collector.clone(),
            analytics.clone(),
            Duration::from_mins(1),
        ))
        .expect("valid pump");

        let input = tokio::time::timeout(Duration::from_secs(1), inputs.recv())
            .await
            .expect("cycle completes")
            .expect("input delivered");
        assert!(input.inventory.is_none());
        handle.shutdown().await.expect("pump task");
        collector.stop(None);

        let completed = analytics
            .events()
            .into_iter()
            .find(|event| event.name == "perception.cycle_completed")
            .expect("completion is observable");
        assert_eq!(completed.attributes["status"], "degraded");
        assert_eq!(completed.attributes["inventory_error_class"], "timeout");
        assert!(
            completed
                .attributes
                .values()
                .all(|value| !value.to_string().contains("10 ms")),
            "analytics must not contain raw error messages"
        );
    }

    struct CountingSource {
        inventory_calls: AtomicUsize,
    }

    #[async_trait]
    impl PerceptionSource for CountingSource {
        async fn observe(&self) -> Result<Observation, GatewayError> {
            Ok(Observation::default())
        }

        async fn render_map(&self, _radius: u32) -> Result<MapObservation, GatewayError> {
            Ok(MapObservation::default())
        }

        async fn inventory(&self) -> Result<InventoryResult, GatewayError> {
            self.inventory_calls.fetch_add(1, Ordering::SeqCst);
            Ok(InventoryResult::default())
        }
    }

    #[tokio::test]
    async fn inventory_refresh_cadence_avoids_redundant_mcp_calls() {
        let source = CountingSource {
            inventory_calls: AtomicUsize::new(0),
        };

        for sequence in 1..=12 {
            let reads = read_cycle(&source, 9, inventory_due(sequence, 10)).await;
            assert_eq!(reads.inventory.is_some(), sequence == 1 || sequence == 11);
        }

        assert_eq!(source.inventory_calls.load(Ordering::SeqCst), 2);
    }

    struct HangingSource {
        calls: AtomicUsize,
    }

    impl HangingSource {
        fn started(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        async fn hang<T>(&self) -> Result<T, GatewayError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            pending().await
        }
    }

    #[async_trait]
    impl PerceptionSource for HangingSource {
        async fn observe(&self) -> Result<Observation, GatewayError> {
            self.hang().await
        }

        async fn render_map(&self, _radius: u32) -> Result<MapObservation, GatewayError> {
            self.hang().await
        }

        async fn inventory(&self) -> Result<InventoryResult, GatewayError> {
            self.hang().await
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_an_in_flight_cycle() {
        let source = Arc::new(HangingSource {
            calls: AtomicUsize::new(0),
        });
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let (collector, _inputs) = collector().await;
        let handle = start_perception_pump(args(
            source.clone(),
            collector.clone(),
            analytics.clone(),
            Duration::from_millis(5),
        ))
        .expect("valid pump");

        tokio::time::timeout(Duration::from_secs(1), async {
            while source.started() < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all reads started");
        let exit = tokio::time::timeout(Duration::from_millis(100), handle.shutdown())
            .await
            .expect("shutdown is prompt")
            .expect("pump task");
        assert_eq!(exit, PerceptionPumpExit::Shutdown);
        collector.stop(None);

        let names = analytics
            .events()
            .into_iter()
            .map(|event| event.name)
            .collect::<Vec<_>>();
        assert!(
            names
                .iter()
                .any(|name| name == "perception.cycle_cancelled")
        );
        assert!(names.iter().any(|name| name == "perception.pump_stopped"));
    }

    struct SlowSource {
        active_observes: AtomicUsize,
        maximum_observes: AtomicUsize,
        observations: AtomicUsize,
        delay: Duration,
        inventory: Mutex<InventoryResult>,
    }

    impl SlowSource {
        async fn wait(&self) {
            tokio::time::sleep(self.delay).await;
        }
    }

    #[async_trait]
    impl PerceptionSource for SlowSource {
        async fn observe(&self) -> Result<Observation, GatewayError> {
            let active = self.active_observes.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum_observes.fetch_max(active, Ordering::SeqCst);
            self.observations.fetch_add(1, Ordering::SeqCst);
            self.wait().await;
            self.active_observes.fetch_sub(1, Ordering::SeqCst);
            Ok(Observation::default())
        }

        async fn render_map(&self, _radius: u32) -> Result<MapObservation, GatewayError> {
            self.wait().await;
            Ok(MapObservation::default())
        }

        async fn inventory(&self) -> Result<InventoryResult, GatewayError> {
            self.wait().await;
            Ok(self
                .inventory
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone())
        }
    }

    #[tokio::test]
    async fn slow_cycles_do_not_overlap_or_build_a_tick_backlog() {
        let source = Arc::new(SlowSource {
            active_observes: AtomicUsize::new(0),
            maximum_observes: AtomicUsize::new(0),
            observations: AtomicUsize::new(0),
            delay: Duration::from_millis(40),
            inventory: Mutex::new(InventoryResult::default()),
        });
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let (collector, _inputs) = collector().await;
        let handle = start_perception_pump(args(
            source.clone(),
            collector.clone(),
            analytics,
            Duration::from_millis(5),
        ))
        .expect("valid pump");

        tokio::time::sleep(Duration::from_millis(115)).await;
        handle.shutdown().await.expect("pump task");
        collector.stop(None);
        assert_eq!(source.maximum_observes.load(Ordering::SeqCst), 1);
        assert!(source.observations.load(Ordering::SeqCst) <= 3);
    }

    #[tokio::test]
    async fn rejects_zero_interval_without_starting_a_task() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let (collector, _inputs) = collector().await;
        let result = start_perception_pump(args(
            Arc::new(InventoryFailureSource),
            collector.clone(),
            analytics,
            Duration::ZERO,
        ));
        assert!(matches!(
            result,
            Err(PerceptionPumpStartError::ZeroInterval)
        ));
        collector.stop(None);
    }

    #[tokio::test]
    async fn rejects_zero_inventory_cadence_without_starting_a_task() {
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let (collector, _inputs) = collector().await;
        let mut args = args(
            Arc::new(InventoryFailureSource),
            collector.clone(),
            analytics,
            Duration::from_secs(1),
        );
        args.config.inventory_every_cycles = 0;
        let result = start_perception_pump(args);
        assert!(matches!(
            result,
            Err(PerceptionPumpStartError::ZeroInventoryCadence)
        ));
        collector.stop(None);
    }
}
