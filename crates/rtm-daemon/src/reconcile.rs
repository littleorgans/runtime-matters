use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use chrono::Utc;
use lilo_rm_core::{IsolationPolicy, Lifecycle, LostEvidence, RuntimeEvent};
use rtm_platform::process::ProcessStartTime;
use tokio::sync::broadcast;
use tokio::time::{Instant, sleep_until};

use crate::server::ServerState;

pub const PROBE_SWEEP_INTERVAL: Duration = Duration::from_secs(30);
const RESUME_POLL_INTERVAL: Duration = Duration::from_secs(1);
const RESUME_GAP_THRESHOLD: chrono::Duration = chrono::Duration::seconds(3);

#[derive(Clone, Copy, Debug)]
pub struct ReconcileConfig {
    pub sweep_interval: Duration,
    pub resume_poll_interval: Duration,
    pub resume_gap_threshold: chrono::Duration,
}

impl ReconcileConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            sweep_interval: duration_env("RTM_PROBE_SWEEP_INTERVAL_MS", PROBE_SWEEP_INTERVAL)?,
            resume_poll_interval: duration_env(
                "RTM_RESUME_POLL_INTERVAL_MS",
                RESUME_POLL_INTERVAL,
            )?,
            resume_gap_threshold: chrono_duration_env(
                "RTM_RESUME_GAP_THRESHOLD_MS",
                RESUME_GAP_THRESHOLD,
            )?,
        })
    }
}

impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            sweep_interval: PROBE_SWEEP_INTERVAL,
            resume_poll_interval: RESUME_POLL_INTERVAL,
            resume_gap_threshold: RESUME_GAP_THRESHOLD,
        }
    }
}

pub trait ProcessProbe {
    fn pid_alive(&self, pid: u32) -> bool;
    fn start_time_for_pid(&self, pid: u32) -> Result<ProcessStartTime>;
}

trait RuntimeLiveness {
    fn lost_evidence(&self, lifecycle: &Lifecycle) -> Result<Option<LostEvidence>>;
}

pub struct SystemProcessProbe;

struct ReconcileLiveness<'a, P, D> {
    process: &'a P,
    docker: &'a D,
}

struct UnavailableDockerLiveness;

impl ProcessProbe for SystemProcessProbe {
    fn pid_alive(&self, pid: u32) -> bool {
        rtm_platform::process::pid_alive(pid)
    }

    fn start_time_for_pid(&self, pid: u32) -> Result<ProcessStartTime> {
        rtm_platform::process::start_time_probe_for_pid(pid)
    }
}

trait DockerLiveness {
    fn lost_evidence(&self, lifecycle: &Lifecycle) -> Result<Option<LostEvidence>>;
}

impl DockerLiveness for UnavailableDockerLiveness {
    fn lost_evidence(&self, lifecycle: &Lifecycle) -> Result<Option<LostEvidence>> {
        Err(anyhow!(
            "docker lifecycle probe is not configured for session {}",
            lifecycle.session_id
        ))
    }
}

impl<P, D> RuntimeLiveness for ReconcileLiveness<'_, P, D>
where
    P: ProcessProbe,
    D: DockerLiveness,
{
    fn lost_evidence(&self, lifecycle: &Lifecycle) -> Result<Option<LostEvidence>> {
        match &lifecycle.isolation {
            IsolationPolicy::Host => host_lost_evidence(lifecycle, self.process),
            IsolationPolicy::Docker(_) => self.docker.lost_evidence(lifecycle),
        }
    }
}

pub async fn reconcile_startup(
    state: Arc<ServerState>,
    probe: &impl ProcessProbe,
) -> Result<Vec<RuntimeEvent>> {
    reconcile_once(
        state,
        &ReconcileLiveness {
            process: probe,
            docker: &UnavailableDockerLiveness,
        },
    )
    .await
}

pub async fn run_periodic<P>(
    state: Arc<ServerState>,
    probe: P,
    shutdown_rx: broadcast::Receiver<()>,
    config: ReconcileConfig,
) where
    P: ProcessProbe + Send + Sync + 'static,
{
    run_periodic_with_config(state, probe, shutdown_rx, config).await;
}

async fn run_periodic_with_config<P>(
    state: Arc<ServerState>,
    probe: P,
    mut shutdown_rx: broadcast::Receiver<()>,
    config: ReconcileConfig,
) where
    P: ProcessProbe + Send + Sync + 'static,
{
    let mut next_deadline = Instant::now() + config.sweep_interval;
    let mut last_wall_tick = Utc::now();

    loop {
        let poll_deadline =
            std::cmp::min(next_deadline, Instant::now() + config.resume_poll_interval);
        tokio::select! {
            _ = shutdown_rx.recv() => break,
            _ = sleep_until(poll_deadline) => {
                let now = Instant::now();
                let wall_now = Utc::now();
                let resumed = wall_now - last_wall_tick > config.resume_gap_threshold;
                last_wall_tick = wall_now;

                if now >= next_deadline || resumed {
                    let liveness = ReconcileLiveness {
                        process: &probe,
                        docker: &UnavailableDockerLiveness,
                    };
                    if let Err(error) = reconcile_once(Arc::clone(&state), &liveness).await {
                        tracing::warn!(%error, "periodic reconciliation failed");
                    }
                    next_deadline = if now >= next_deadline {
                        advance_deadline(next_deadline, config.sweep_interval, now)
                    } else {
                        now + config.sweep_interval
                    };
                }
            }
        }
    }
}

fn advance_deadline(mut deadline: Instant, interval: Duration, now: Instant) -> Instant {
    deadline += interval;
    while deadline <= now {
        deadline += interval;
    }
    deadline
}

async fn reconcile_once(
    state: Arc<ServerState>,
    liveness: &impl RuntimeLiveness,
) -> Result<Vec<RuntimeEvent>> {
    let mut events = Vec::new();
    for lifecycle in state.store().running().await? {
        if let Some(evidence) = liveness.lost_evidence(&lifecycle)? {
            if let Some(event) = state.record_lost(lifecycle.session_id, evidence).await? {
                events.push(event);
            }
            continue;
        }
        let runtime_pid = lifecycle
            .runtime_pid
            .ok_or_else(|| anyhow!("running lifecycle missing runtime pid"))?;
        state
            .start_exit_watcher(lifecycle.session_id, runtime_pid)
            .await?;
    }
    state.store().record_probe_sweep(Utc::now()).await?;
    Ok(events)
}

fn host_lost_evidence(
    lifecycle: &Lifecycle,
    probe: &impl ProcessProbe,
) -> Result<Option<LostEvidence>> {
    let runtime_pid = lifecycle
        .runtime_pid
        .ok_or_else(|| anyhow!("running lifecycle missing runtime pid"))?;
    if !probe.pid_alive(runtime_pid) {
        return Ok(Some(LostEvidence::PidNotAlive));
    }

    let Some(stored_start_time) = lifecycle.start_time else {
        return Ok(None);
    };
    let current_start_time = match probe.start_time_for_pid(runtime_pid) {
        Ok(ProcessStartTime::Known(start_time)) => start_time,
        Ok(ProcessStartTime::Gone) => return Ok(Some(LostEvidence::PidNotAlive)),
        Ok(ProcessStartTime::Unsupported) => {
            if !probe.pid_alive(runtime_pid) {
                return Ok(Some(LostEvidence::PidNotAlive));
            }
            return Ok(None);
        }
        Err(error) => {
            if !probe.pid_alive(runtime_pid) {
                return Ok(Some(LostEvidence::PidNotAlive));
            }
            return Err(error);
        }
    };
    if current_start_time != stored_start_time {
        return Ok(Some(LostEvidence::PidReuseDetected));
    }
    Ok(None)
}

fn duration_env(name: &'static str, default: Duration) -> Result<Duration> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(default);
    };
    let millis = value
        .to_string_lossy()
        .parse::<u64>()
        .map_err(|error| anyhow!("{name} must be milliseconds: {error}"))?;
    Ok(Duration::from_millis(millis))
}

fn chrono_duration_env(name: &'static str, default: chrono::Duration) -> Result<chrono::Duration> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(default);
    };
    let millis = value
        .to_string_lossy()
        .parse::<i64>()
        .map_err(|error| anyhow!("{name} must be milliseconds: {error}"))?;
    Ok(chrono::Duration::milliseconds(millis))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::{DateTime, TimeZone};
    use lilo_rm_core::{IsolationPolicy, IsolationProfile, LifecycleState, RuntimeKind, ShimReady};
    use rtm_store::{LifecycleStore, StoreConfig};
    use uuid::Uuid;

    use super::*;
    use crate::server::DaemonConfig;

    struct FakeProbe {
        alive: HashSet<u32>,
        start_times: HashMap<u32, DateTime<Utc>>,
    }

    impl ProcessProbe for FakeProbe {
        fn pid_alive(&self, pid: u32) -> bool {
            self.alive.contains(&pid)
        }

        fn start_time_for_pid(&self, pid: u32) -> Result<ProcessStartTime> {
            Ok(self
                .start_times
                .get(&pid)
                .copied()
                .map(ProcessStartTime::Known)
                .unwrap_or(ProcessStartTime::Unsupported))
        }
    }

    struct VanishingProbe {
        alive_checks: AtomicUsize,
    }

    impl ProcessProbe for VanishingProbe {
        fn pid_alive(&self, _pid: u32) -> bool {
            self.alive_checks.fetch_add(1, Ordering::SeqCst) == 0
        }

        fn start_time_for_pid(&self, pid: u32) -> Result<ProcessStartTime> {
            Err(anyhow!("failed to read start time for pid {pid}"))
        }
    }

    struct GoneProbe;

    impl ProcessProbe for GoneProbe {
        fn pid_alive(&self, _pid: u32) -> bool {
            true
        }

        fn start_time_for_pid(&self, _pid: u32) -> Result<ProcessStartTime> {
            Ok(ProcessStartTime::Gone)
        }
    }

    struct FakeDockerLiveness {
        checks: AtomicUsize,
        evidence: Option<LostEvidence>,
    }

    impl DockerLiveness for FakeDockerLiveness {
        fn lost_evidence(&self, _lifecycle: &Lifecycle) -> Result<Option<LostEvidence>> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            Ok(self.evidence)
        }
    }

    #[tokio::test]
    async fn startup_reconciliation_marks_dead_and_reused_pids_lost_once() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let store = LifecycleStore::open(StoreConfig {
            db_path: temp.path().join("rtm.sqlite"),
        })
        .await
        .expect("store");
        let dead = persist_running(&store, 101, Utc.timestamp_opt(1_000, 0).unwrap()).await;
        let reused = persist_running(&store, 202, Utc.timestamp_opt(2_000, 0).unwrap()).await;
        let mut already_lost =
            persist_running(&store, 303, Utc.timestamp_opt(3_000, 0).unwrap()).await;
        already_lost.mark_lost(LostEvidence::PidNotAlive);
        store
            .update_lifecycle(&already_lost)
            .await
            .expect("persist lost");

        let state =
            Arc::new(ServerState::new(test_config(temp.path()), store.clone()).expect("state"));
        let probe = FakeProbe {
            alive: HashSet::from([202]),
            start_times: HashMap::from([(202, Utc.timestamp_opt(2_001, 0).unwrap())]),
        };

        let events = reconcile_startup(Arc::clone(&state), &probe)
            .await
            .expect("reconcile");
        let replay = reconcile_startup(state, &probe).await.expect("replay");

        assert_eq!(events.len(), 2);
        assert!(replay.is_empty(), "{replay:?}");
        assert_lost(&store, dead.session_id, LostEvidence::PidNotAlive).await;
        assert_lost(&store, reused.session_id, LostEvidence::PidReuseDetected).await;
    }

    #[tokio::test]
    async fn startup_reconciliation_marks_pid_lost_when_start_time_races_exit() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let store = LifecycleStore::open(StoreConfig {
            db_path: temp.path().join("rtm.sqlite"),
        })
        .await
        .expect("store");
        let lifecycle = persist_running(&store, 404, Utc.timestamp_opt(4_000, 0).unwrap()).await;
        let state =
            Arc::new(ServerState::new(test_config(temp.path()), store.clone()).expect("state"));
        let probe = VanishingProbe {
            alive_checks: AtomicUsize::new(0),
        };

        let events = reconcile_startup(state, &probe).await.expect("reconcile");

        assert_eq!(events.len(), 1);
        assert_lost(&store, lifecycle.session_id, LostEvidence::PidNotAlive).await;
    }

    #[tokio::test]
    async fn startup_reconciliation_marks_pid_lost_when_probe_reports_gone() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let store = LifecycleStore::open(StoreConfig {
            db_path: temp.path().join("rtm.sqlite"),
        })
        .await
        .expect("store");
        let lifecycle = persist_running(&store, 505, Utc.timestamp_opt(5_000, 0).unwrap()).await;
        let state =
            Arc::new(ServerState::new(test_config(temp.path()), store.clone()).expect("state"));

        let events = reconcile_startup(state, &GoneProbe)
            .await
            .expect("reconcile");

        assert_eq!(events.len(), 1);
        assert_lost(&store, lifecycle.session_id, LostEvidence::PidNotAlive).await;
    }

    #[tokio::test]
    async fn periodic_reconciliation_marks_dead_and_reused_pids_lost_once() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let store = LifecycleStore::open(StoreConfig {
            db_path: temp.path().join("rtm.sqlite"),
        })
        .await
        .expect("store");
        let dead = persist_running(&store, 101, Utc.timestamp_opt(1_000, 0).unwrap()).await;
        let reused = persist_running(&store, 202, Utc.timestamp_opt(2_000, 0).unwrap()).await;
        let state =
            Arc::new(ServerState::new(test_config(temp.path()), store.clone()).expect("state"));
        let probe = FakeProbe {
            alive: HashSet::from([202]),
            start_times: HashMap::from([(202, Utc.timestamp_opt(2_001, 0).unwrap())]),
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
        let task = tokio::spawn(run_periodic_with_config(
            Arc::clone(&state),
            probe,
            shutdown_rx,
            ReconcileConfig {
                sweep_interval: Duration::from_millis(20),
                ..ReconcileConfig::default()
            },
        ));

        wait_for_events(&state, 2).await;
        let _ = shutdown_tx.send(());
        task.await.expect("periodic task");

        assert_lost(&store, dead.session_id, LostEvidence::PidNotAlive).await;
        assert_lost(&store, reused.session_id, LostEvidence::PidReuseDetected).await;
        assert_eq!(
            state
                .events(lilo_rm_core::EventsRequest {
                    since: Some(0),
                    wait_ms: None
                })
                .await
                .expect("events")
                .events
                .len(),
            2
        );
        assert_eq!(store.running().await.expect("running").len(), 0);
    }

    #[tokio::test]
    async fn docker_reconciliation_uses_docker_liveness() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let store = LifecycleStore::open(StoreConfig {
            db_path: temp.path().join("rtm.sqlite"),
        })
        .await
        .expect("store");
        let lifecycle = persist_docker_running(&store, 606).await;
        let state =
            Arc::new(ServerState::new(test_config(temp.path()), store.clone()).expect("state"));
        let process = FakeProbe {
            alive: HashSet::new(),
            start_times: HashMap::new(),
        };
        let docker = FakeDockerLiveness {
            checks: AtomicUsize::new(0),
            evidence: Some(LostEvidence::PidNotAlive),
        };
        let liveness = ReconcileLiveness {
            process: &process,
            docker: &docker,
        };

        let events = reconcile_once(state, &liveness).await.expect("reconcile");

        assert_eq!(docker.checks.load(Ordering::SeqCst), 1);
        assert_eq!(events.len(), 1);
        assert_lost(&store, lifecycle.session_id, LostEvidence::PidNotAlive).await;
    }

    #[test]
    fn resume_gap_threshold_detects_wall_clock_jump() {
        let last_wall_tick = Utc.timestamp_opt(1_000, 0).unwrap();
        let wall_now = Utc.timestamp_opt(1_004, 0).unwrap();

        assert!(wall_now - last_wall_tick > RESUME_GAP_THRESHOLD);
    }

    async fn assert_lost(store: &LifecycleStore, session_id: Uuid, evidence: LostEvidence) {
        let lifecycle = store
            .get(session_id)
            .await
            .expect("get")
            .expect("lifecycle");
        assert_eq!(lifecycle.state, LifecycleState::Lost(evidence));
    }

    async fn persist_running(
        store: &LifecycleStore,
        pid: u32,
        start_time: DateTime<Utc>,
    ) -> Lifecycle {
        let mut lifecycle = forking_lifecycle();
        store.insert_forking(&lifecycle).await.expect("insert");
        lifecycle.mark_running(ShimReady {
            session_id: lifecycle.session_id,
            shim_pid: pid + 10_000,
            runtime_pid: pid,
            start_time,
            tmux_pane: None,
        });
        store.update_lifecycle(&lifecycle).await.expect("running");
        lifecycle
    }

    async fn persist_docker_running(store: &LifecycleStore, pid: u32) -> Lifecycle {
        let mut lifecycle = forking_lifecycle();
        lifecycle.isolation = IsolationPolicy::Docker(IsolationProfile {
            name: Some("locked".to_owned()),
        });
        store.insert_forking(&lifecycle).await.expect("insert");
        lifecycle.mark_running(ShimReady {
            session_id: lifecycle.session_id,
            shim_pid: pid + 10_000,
            runtime_pid: pid,
            start_time: Utc.timestamp_opt(6_000, 0).unwrap(),
            tmux_pane: None,
        });
        store.update_lifecycle(&lifecycle).await.expect("running");
        lifecycle
    }

    fn forking_lifecycle() -> Lifecycle {
        let session_id = Uuid::now_v7();
        Lifecycle::forking(session_id, RuntimeKind::Claude)
    }

    async fn wait_for_events(state: &ServerState, expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if state
                    .events(lilo_rm_core::EventsRequest {
                        since: Some(0),
                        wait_ms: None,
                    })
                    .await
                    .expect("events")
                    .events
                    .len()
                    == expected
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("events reached expected count");
    }

    fn test_config(root: &Path) -> DaemonConfig {
        DaemonConfig {
            endpoint: rtm_paths::RuntimeEndpoint::unix_socket(root.join("rtm-test.sock")),
            shim_path: PathBuf::from("rtm"),
            log_root: root.join("logs"),
            store: StoreConfig {
                db_path: root.join("rtm-test.sqlite"),
            },
            reconcile: ReconcileConfig::default(),
            docker_preflight: Default::default(),
        }
    }
}
