//! Dev-observability subscriber wiring (DEV-OBSERVABILITY wave R).
//!
//! One `tracing` registry feeding two layers behind ONE shared [`EnvFilter`]:
//!
//! * a **stderr** `fmt` layer (human, unbuffered — the panic/crash lane never
//!   loses a line), and
//! * a **JSONL** layer over `<logs>/dev.jsonl`, written through
//!   [`tracing_appender::non_blocking`].
//!
//! ## The file lane is app-only
//!
//! [`init`] is called from [`crate::run`] and NOWHERE else. `debug_assertions` is
//! true under `cargo test` too, so a `log_dir()`-driven file layer wired into a
//! test/CI subscriber would have every `cargo test` scribbling into the repo's
//! `logs/`. Unit tests use [`init_test_tracing`]; integration tests build their own
//! `tracing_subscriber` stack (the crate is a normal dependency, so `tests/` can
//! use it directly — see `tests/worker_stderr_capture.rs`).
//!
//! ## Session scoping
//!
//! `dev.jsonl` is **truncated at every app start** and its first line is a
//! `session.start` event carrying `{pid, version, filter}` — so "this run" is just
//! "the whole file", and a grep never mixes two sessions. The non-blocking writer's
//! [`WorkerGuard`] is dropped on `RunEvent::Exit`; a SIGINT (Ctrl-C on
//! `tauri dev`) never reaches that arm, so at most one buffer of JSONL tail is lost
//! — accepted, because the stderr layer is unbuffered.
//!
//! ## Schema
//!
//! `{"timestamp":"<RFC3339 µs UTC>","level":…,"target":…,"message":…, <event
//! fields>, "span":{"name":…, <span fields>}}`. Span-close lines carry
//! `"message":"close"` plus `time.busy`/`time.idle` as unit-suffixed **strings**
//! (human hint only) — machine timing is always an explicit numeric `elapsed_ms`
//! field on a terminal event. `timestamp`/`level`/`target`/`span`/`message` are
//! RESERVED event-field names.

use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// The default `RUST_LOG` when the env var is unset.
///
/// `fe` and `worker` are REQUIRED directives, not decoration: those two lanes are
/// synthetic targets (forwarded frontend events / forwarded C++ stderr) whose
/// events are emitted at `debug`, so a bare global `info` would silently drop them.
pub const DEFAULT_FILTER: &str = "info,onecad_lib=debug,onecad=debug,fe=debug,worker=debug";

/// The shared filter: `RUST_LOG` when set (and parseable), else [`DEFAULT_FILTER`].
#[must_use]
pub fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))
}

/// Where `dev.jsonl` goes, or `None` for "no file lane".
///
/// * `ONECAD_LOG_DIR=off` (any case) — off, even in a debug build.
/// * `ONECAD_LOG_DIR=<path>` — that directory (works in a packaged build too).
/// * unset — `<repo>/logs` in a debug build; `None` in a release build.
#[must_use]
pub fn log_dir() -> Option<PathBuf> {
    match std::env::var("ONECAD_LOG_DIR") {
        Ok(v) if v.trim().eq_ignore_ascii_case("off") => None,
        Ok(v) if !v.trim().is_empty() => Some(PathBuf::from(v)),
        _ if cfg!(debug_assertions) => {
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../logs"))
        }
        _ => None,
    }
}

/// The JSONL layer configuration — shared by [`init`] and the schema unit test so
/// the test pins the SHIPPING shape, not a copy of it.
fn json_layer<S, W>(writer: W) -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> MakeWriter<'w> + 'static,
{
    tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(false)
        .with_span_events(FmtSpan::CLOSE)
        .with_writer(writer)
}

/// Truncates + opens `<dir>/dev.jsonl`, returning the non-blocking writer and its
/// flush guard. `Err` carries a human reason (the caller warns and runs without the
/// file lane — a missing log file must never stop the app).
fn open_sink(
    dir: &PathBuf,
) -> Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create_dir_all {}: {e}", dir.display()))?;
    let path = dir.join("dev.jsonl");
    // TRUNCATE (not append): the file IS the current session (see module docs).
    let file =
        std::fs::File::create(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
    Ok(tracing_appender::non_blocking(file))
}

/// Installs the process subscriber. Returns the JSONL flush guard, which the caller
/// must hold for the process lifetime (dropping it flushes the buffer).
///
/// Idempotent-ish: uses `try_init`, so a second call (or a test that already
/// installed one) is a no-op rather than a panic.
#[must_use]
pub fn init() -> Option<WorkerGuard> {
    let sink = log_dir().map(|dir| (dir.clone(), open_sink(&dir)));
    let (dir, layer, guard, error) = match sink {
        Some((dir, Ok((writer, guard)))) => {
            (Some(dir), Some(json_layer(writer)), Some(guard), None)
        }
        Some((dir, Err(e))) => (Some(dir), None, None, Some(e)),
        None => (None, None, None, None),
    };

    let installed = tracing_subscriber::registry()
        .with(env_filter())
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(layer)
        .try_init()
        .is_ok();
    if !installed {
        return guard;
    }
    // Emitted only AFTER the subscriber is live, or the diagnostics vanish.
    if let Some(error) = error {
        tracing::warn!(error = %error, "dev log file unavailable; stderr only");
    }
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| DEFAULT_FILTER.to_string());
    tracing::info!(
        pid = std::process::id(),
        version = env!("CARGO_PKG_VERSION"),
        filter = %filter,
        dir = %dir.map(|d| d.display().to_string()).unwrap_or_default(),
        "session.start"
    );
    guard
}

/// Routes panics through `tracing` (target `panic`) BEFORE the previous hook runs,
/// so a crash lands in both lanes with its payload + backtrace.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());
        let location = info
            .location()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<unknown>".to_string());
        tracing::error!(
            target: "panic",
            payload = %payload,
            location = %location,
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "panic"
        );
        previous(info);
    }));
}

/// Opt-in stderr tracing for a **unit** test (`logging::init_test_tracing()` at the
/// top of the test). Never touches the file lane — no test may write `logs/`.
#[cfg(test)]
pub fn init_test_tracing() {
    let _ = tracing_subscriber::registry()
        .with(env_filter())
        .with(tracing_subscriber::fmt::layer().with_test_writer())
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// An in-memory `MakeWriter` so the schema test reads exactly what the file
    /// lane would have written.
    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedBuf {
        type Writer = SharedBuf;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    impl SharedBuf {
        fn lines(&self) -> Vec<serde_json::Value> {
            let bytes = self.0.lock().unwrap().clone();
            String::from_utf8(bytes)
                .unwrap()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).expect("every JSONL line parses"))
                .collect()
        }
    }

    /// The default filter is a CONTRACT with `docs/DEBUGGING.md` + the `fe`/`worker`
    /// lanes: both are emitted at debug, so dropping either directive silently
    /// blanks a whole lane.
    #[test]
    fn default_filter_keeps_the_synthetic_lanes_at_debug() {
        assert_eq!(
            DEFAULT_FILTER,
            "info,onecad_lib=debug,onecad=debug,fe=debug,worker=debug"
        );
        for directive in ["fe=debug", "worker=debug", "onecad_lib=debug"] {
            assert!(
                DEFAULT_FILTER.contains(directive),
                "default filter must carry {directive}"
            );
        }
    }

    #[test]
    fn log_dir_policy_honours_off_and_explicit_paths() {
        // `ONECAD_LOG_DIR` is process-global: this test owns it end-to-end and
        // restores it, and no other test in this module reads it.
        let previous = std::env::var("ONECAD_LOG_DIR").ok();

        std::env::set_var("ONECAD_LOG_DIR", "off");
        assert_eq!(log_dir(), None, "`off` kills the file lane");
        std::env::set_var("ONECAD_LOG_DIR", "OFF");
        assert_eq!(log_dir(), None, "`off` is case-insensitive");

        std::env::set_var("ONECAD_LOG_DIR", "/tmp/onecad-logs");
        assert_eq!(log_dir(), Some(PathBuf::from("/tmp/onecad-logs")));

        std::env::remove_var("ONECAD_LOG_DIR");
        // The test profile always has debug_assertions, so unset ⇒ the repo `logs/`.
        assert!(log_dir().is_some_and(|p| p.ends_with("logs")));

        match previous {
            Some(v) => std::env::set_var("ONECAD_LOG_DIR", v),
            None => std::env::remove_var("ONECAD_LOG_DIR"),
        }
    }

    /// The sink is TRUNCATE-on-open: `dev.jsonl` must describe exactly one session,
    /// or every grep silently spans runs. Also proves the directory is created.
    #[test]
    fn opening_the_sink_creates_the_dir_and_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested/logs");
        let path = nested.join("dev.jsonl");

        let (writer, guard) = open_sink(&nested).expect("sink opens");
        drop((writer, guard));
        std::fs::write(&path, b"stale line from a previous session\n").unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);

        let (writer, guard) = open_sink(&nested).expect("sink reopens");
        drop((writer, guard));
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            0,
            "a new session truncates the previous one"
        );
    }

    #[test]
    fn jsonl_schema_carries_the_documented_fields() {
        let buf = SharedBuf::default();
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new(DEFAULT_FILTER))
            .with(json_layer(buf.clone()));
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("regen.drive", job = "j-1", rev = 7u64);
            let _entered = span.enter();
            tracing::info!(target: "worker", epoch = 3u64, "stub: hello");
        });

        let lines = buf.lines();
        let event = lines
            .iter()
            .find(|l| l["message"] == "stub: hello")
            .expect("the event line is written");
        assert_eq!(event["level"], "INFO");
        assert_eq!(event["target"], "worker");
        // Flattened event fields sit at the top level, span fields under `span`.
        assert_eq!(event["epoch"], 3);
        assert_eq!(event["span"]["name"], "regen.drive");
        assert_eq!(event["span"]["job"], "j-1");
        assert_eq!(event["span"]["rev"], 7);
        // RFC3339 µs UTC.
        let ts = event["timestamp"].as_str().unwrap();
        assert!(ts.contains('T') && ts.ends_with('Z'), "timestamp: {ts}");
        // No span LIST (only the current span) — the file stays greppable.
        assert!(event.get("spans").is_none());

        // FmtSpan::CLOSE gives one close line whose timings are STRINGS (human
        // hint); machine timing is the explicit numeric `elapsed_ms` on terminals.
        let close = lines
            .iter()
            .find(|l| l["message"] == "close")
            .expect("span close is written");
        assert!(close["time.busy"].is_string(), "time.busy must be a string");
        assert!(close["time.idle"].is_string(), "time.idle must be a string");
    }
}
