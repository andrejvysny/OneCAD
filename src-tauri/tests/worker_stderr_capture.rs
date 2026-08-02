//! DEV-OBSERVABILITY wave R — the worker's stderr is **captured**, not inherited.
//!
//! `spawn_and_connect` switched the child's stderr from `Stdio::inherit()` to
//! `Stdio::piped()` plus an unconditional forwarder task. That makes two things
//! testable, and both are regression-prone:
//!
//! * a line the child writes to stderr reaches `tracing` under target `worker`
//!   with the spawn `epoch` attached — if the forwarder were ever dropped, the
//!   worker would instead BLOCK on a full pipe, which no other test would catch;
//! * the [`WorkerLifecycle`] transitions are logged (Ready = info, Restarting =
//!   warn) even with nobody subscribed to the broadcast.
//!
//! The subscriber is the crate's own [`DEFAULT_FILTER`], so this also pins that the
//! shipping filter does not silently drop the `worker` lane (it is emitted at
//! info/debug under a synthetic target — a bare global `info` would eat half of it).
//!
//! Gated on the stub binary existing (built by `cargo test --workspace`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{EnvFilter, Layer};

use onecad_lib::logging::DEFAULT_FILTER;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::WorkerManager;

/// One captured `tracing` event, flattened to what this test asserts on.
#[derive(Debug, Clone)]
struct Line {
    target: String,
    level: String,
    message: String,
    epoch: Option<u64>,
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<Line>>>);

impl Captured {
    fn snapshot(&self) -> Vec<Line> {
        self.0.lock().unwrap().clone()
    }
}

impl<S: tracing::Subscriber> Layer<S> for Captured {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0.lock().unwrap().push(Line {
            target: event.metadata().target().to_string(),
            level: event.metadata().level().to_string(),
            message: fields.message,
            epoch: fields.epoch,
        });
    }
}

#[derive(Default)]
struct Fields {
    message: String,
    epoch: Option<u64>,
}

impl Visit for Fields {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "epoch" {
            self.epoch = Some(value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // The event message is a `fmt::Arguments`; its Debug == its Display.
            self.message = format!("{value:?}");
        }
    }
}

/// The built stub binary. `None` ⇒ the workspace was not built ⇒ skip.
fn stub_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?; // <target>/debug/deps/<test>
    let bin = exe.parent()?.parent()?.join("onecad-worker-stub");
    bin.exists().then_some(bin)
}

/// A fast-supervision config whose stub dies right after the handshake, so ONE
/// short run exercises stderr forwarding + Ready + Restarting.
fn dying_stub_config(binary: PathBuf) -> SupervisorConfig {
    SupervisorConfig {
        binary,
        envs: vec![("ONECAD_STUB_EXIT_AFTER_HELLO".into(), "1".into())],
        ping_interval: Duration::from_millis(100),
        ping_timeout: Duration::from_millis(500),
        max_missed_pings: 2,
        backoff: vec![Duration::from_millis(10)],
        // One strike: the flap budget ends the supervisor quickly instead of
        // respawning the dying stub for the whole test timeout.
        max_rapid_deaths: 1,
        healthy_threshold: Duration::from_millis(300),
        poison_threshold: 3,
        auto_open_session: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_stderr_reaches_the_worker_lane_and_lifecycle_is_logged() {
    let Some(bin) = stub_binary() else {
        return;
    };
    let captured = Captured::default();
    // A GLOBAL default: the forwarder is a detached task on another thread, so a
    // thread-local (`with_default`) subscriber would never see it.
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(EnvFilter::new(DEFAULT_FILTER))
            .with(captured.clone()),
    )
    .expect("this test binary installs the only subscriber");

    let wm = WorkerManager::spawn(dying_stub_config(bin));
    // The stub exits right after `hello`, so Ready → Restarting happens on its own.
    let _ = wm.wait_ready(Duration::from_millis(500)).await;

    let mut lines = Vec::new();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        lines = captured.snapshot();
        let has_stderr = lines.iter().any(|l| l.target == "worker");
        let has_restart = lines
            .iter()
            .any(|l| l.message.contains("worker restarting"));
        if has_stderr && has_restart {
            break;
        }
    }

    let forwarded: Vec<&Line> = lines.iter().filter(|l| l.target == "worker").collect();
    assert!(
        !forwarded.is_empty(),
        "the child's stderr must reach target `worker` (pipe + forwarder alive); got {lines:#?}"
    );
    assert!(
        forwarded
            .iter()
            .any(|l| l.message.contains("stub: EXIT_AFTER_HELLO")),
        "the stub's own stderr line is forwarded verbatim; got {forwarded:#?}"
    );
    assert!(
        forwarded.iter().all(|l| l.epoch.is_some()),
        "every forwarded worker line carries its spawn epoch; got {forwarded:#?}"
    );

    let ready = lines
        .iter()
        .find(|l| l.message.contains("worker ready"))
        .expect("the Ready transition is logged");
    assert_eq!(ready.level, "INFO");
    assert_eq!(ready.epoch, Some(1), "Ready carries the live epoch");

    let restarting = lines
        .iter()
        .find(|l| l.message.contains("worker restarting"))
        .expect("the Restarting transition is logged");
    assert_eq!(
        restarting.level, "WARN",
        "a restart is a warning, not an info"
    );
}
