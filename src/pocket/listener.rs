//! The guest listener's supervisor. Every 5 s it reads the `pocket_apps` switch: on starts the
//! guest port (main port + 1, or `KWNT_POCKET_PORT`), off stops it. While on it also sweeps
//! expired pockets every minute (after pa7's reminders and results, `schedule::run_once`) and runs
//! watched pockets' kit timers every second. While off it binds nothing and touches no pocket. It
//! also stops a quick tunnel when the switch goes off, and on macOS keeps the computer awake while
//! a tunnel or an open pocket runs (pa6).

use crate::server::AppState;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const RECONCILE_EVERY: Duration = Duration::from_secs(5);
const SWEEP_EVERY: Duration = Duration::from_secs(60);
const TICK_EVERY: Duration = Duration::from_secs(1);

struct Running {
    addr: SocketAddr,
    cancel: CancellationToken,
}

/// The running guest listener, if any.
#[derive(Default)]
pub struct ListenerSlot {
    running: Mutex<Option<Running>>,
}

impl ListenerSlot {
    /// Where the guest listener is bound right now (`None`: not running).
    pub fn addr(&self) -> Option<SocketAddr> {
        self.running.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|r| r.addr)
    }

    fn stop(&self) {
        if let Some(r) = self.running.lock().unwrap_or_else(|e| e.into_inner()).take() {
            r.cancel.cancel();
            tracing::info!("pocket apps: guest listener stopped");
        }
    }
}

/// Brings the listener (and the quick tunnel, and the Mac's keep-awake) in line with the
/// switch. Returns whether pocket apps are on.
pub async fn reconcile(state: &AppState) -> bool {
    let on = super::enabled(&state.db).await;
    state.pocket.on.store(on, Ordering::SeqCst);
    if on {
        open(state).await;
    } else {
        state.pocket.tunnel.stop_all();
        state.pocket.listener.stop();
    }
    super::tunnel::keep_awake(state, on).await;
    on
}

/// Opens the guest port if it isn't open yet.
async fn open(state: &AppState) {
    let slot = &state.pocket.listener;
    if slot.addr().is_some() {
        return;
    }
    let Some(port) = super::pocket_port(&state.config) else {
        tracing::warn!("pocket apps: no guest port (the main port is 65535); set KWNT_POCKET_PORT");
        return;
    };
    let listener = match tokio::net::TcpListener::bind((state.config.host.as_str(), port)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(port, "pocket apps: couldn't open the guest port, trying again shortly: {e}");
            return;
        }
    };
    let addr = listener.local_addr().unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)));
    let cancel = CancellationToken::new();
    {
        let mut running = slot.running.lock().unwrap_or_else(|e| e.into_inner());
        if running.is_some() {
            return;
        }
        *running = Some(Running { addr, cancel: cancel.clone() });
    }
    let app = super::guest::router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move {
        let stop = cancel.clone();
        if let Err(e) = axum::serve(listener, app).with_graceful_shutdown(async move { stop.cancelled().await }).await {
            tracing::warn!("pocket apps: guest listener ended: {e}");
        }
    });
    tracing::info!(%addr, "pocket apps: guest listener started");
}

/// Starts the supervisor; it stops with kwant (and takes the guest listener with it).
pub fn spawn(state: AppState, cancel: CancellationToken) {
    tokio::spawn(async move {
        let mut reconcile_tick = tokio::time::interval(RECONCILE_EVERY);
        let mut sweep_tick = tokio::time::interval(SWEEP_EVERY);
        let mut timer_tick = tokio::time::interval(TICK_EVERY);
        for t in [&mut reconcile_tick, &mut sweep_tick, &mut timer_tick] {
            t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        }
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = reconcile_tick.tick() => { reconcile(&state).await; }
                _ = sweep_tick.tick() => {
                    if state.pocket.on.load(Ordering::SeqCst) {
                        let now = chrono::Utc::now();
                        // pa7 first: reminders, and results posted before the sweep deletes
                        super::schedule::run_once(&state, now).await;
                        super::sweep(&state, now).await;
                    }
                }
                _ = timer_tick.tick() => {
                    if state.pocket.on.load(Ordering::SeqCst) {
                        super::tick(&state).await;
                    }
                }
            }
        }
        state.pocket.tunnel.stop_all();
        state.pocket.awake.set(false);
        state.pocket.listener.stop();
    });
}
