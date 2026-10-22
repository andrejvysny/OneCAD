//! [`SidecarHost`]: the macOS production backend — the `onecad-worker` child
//! process over piped stdio. The code below moved here unchanged from
//! `manager.rs` (spawn, stderr forwarding, SIGKILL reaping); only the shared
//! handshake stayed behind.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;

use super::host::{KillOutcome, Launched, WorkerHost, WorkerInstance};
use super::manifest as bundled_manifest;

/// Spawns `binary` with `envs` (chaos drills) for every launch.
#[derive(Debug, Clone)]
pub(crate) struct SidecarHost {
    binary: PathBuf,
    envs: Vec<(String, String)>,
}

impl SidecarHost {
    pub(crate) fn new(binary: PathBuf, envs: Vec<(String, String)>) -> Self {
        Self { binary, envs }
    }
}

#[async_trait]
impl WorkerHost for SidecarHost {
    async fn launch(&self, epoch: u64) -> Result<Launched, String> {
        let expected = bundled_manifest::embedded_release_manifest()?;
        if let Some(expected) = &expected {
            bundled_manifest::verify_binary(&self.binary, expected)?;
        }

        let mut cmd = tokio::process::Command::new(&self.binary);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &self.envs {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn {:?}: {e}", self.binary))?;
        // HARD INVARIANT: the stderr pipe is now OURS, and `Log.h` flushes every line —
        // an unread pipe fills its buffer and BLOCKS the worker mid-op. The forwarder is
        // therefore spawned unconditionally, before anything that can fail or await
        // (`?` below drops `child`, and `kill_on_drop` then ends the forwarder at EOF).
        // The epoch is read at spawn time by the caller: the supervisor's `fetch_add`
        // happens on the death path, before the respawn, so this is the epoch the
        // worker runs under. (The failed-*start* branch does not bump, so N
        // consecutive failed spawns share one epoch — cosmetic, and there is no
        // worker output to attribute anyway.)
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(forward_worker_stderr(stderr, epoch));
        }
        let stdout = child.stdout.take().ok_or("child stdout missing")?;
        let stdin = child.stdin.take().ok_or("child stdin missing")?;
        Ok(Launched {
            reader: Box::new(stdout),
            writer: Box::new(stdin),
            instance: Box::new(SidecarInstance { child }),
            expected: expected
                .as_ref()
                .map(bundled_manifest::WorkerManifest::axes),
        })
    }
}

/// The running child. Dropping it kills the process (`kill_on_drop`), which is
/// what reaps a child whose handshake failed or was cancelled by retirement.
struct SidecarInstance {
    child: tokio::process::Child,
}

#[async_trait]
impl WorkerInstance for SidecarInstance {
    async fn exited(&mut self) {
        let _ = self.child.wait().await;
    }

    /// Waits up to `grace` for the self-exit the graceful `Shutdown` should
    /// produce, then `SIGKILL`s. Always returns with the child reaped, so
    /// `kill_on_drop` stays a backstop rather than the mechanism.
    async fn terminate(&mut self, grace: Duration) {
        if tokio::time::timeout(grace, self.child.wait()).await.is_ok() {
            return;
        }
        tracing::warn!("retired worker ignored Shutdown within the grace window — SIGKILL");
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }

    async fn force_kill(&mut self) -> KillOutcome {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        KillOutcome::Reaped
    }
}

/// Largest single worker stderr line forwarded verbatim; the remainder of an
/// over-long line is dropped with a marker (a runaway `WLOG` must not blow up the
/// JSONL sink).
const MAX_WORKER_LINE: usize = 8 * 1024;

/// Forwards the C++ worker's stderr into the `tracing` stream under target
/// `worker`, one event per line, at the level its `WLOG` prefix declares.
///
/// This task is detached (it outlives nothing but the pipe), so worker events carry
/// **no span context** — join them to a regen through the frame trace's `reqId`, or
/// through `epoch`. It self-terminates at EOF: it is the sole owner of the pipe, so
/// when the child dies the read resolves `Ok(0)`.
async fn forward_worker_stderr(stderr: tokio::process::ChildStderr, epoch: u64) {
    use tokio::io::AsyncBufReadExt;

    let mut reader = tokio::io::BufReader::new(stderr);
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) => {
                tracing::debug!(target: "worker", epoch, "worker stderr closed (EOF)");
                return;
            }
            Ok(_) => {
                let truncated = buf.len() > MAX_WORKER_LINE;
                if truncated {
                    buf.truncate(MAX_WORKER_LINE);
                }
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\n', '\r']);
                if line.is_empty() {
                    continue;
                }
                let line = if truncated {
                    format!("{line} …[truncated]")
                } else {
                    line.to_string()
                };
                match sniff_worker_level(&line) {
                    WorkerLine::Error => tracing::error!(target: "worker", epoch, "{line}"),
                    WorkerLine::Warn => tracing::warn!(target: "worker", epoch, "{line}"),
                    WorkerLine::Info => tracing::info!(target: "worker", epoch, "{line}"),
                    WorkerLine::Debug => tracing::debug!(target: "worker", epoch, "{line}"),
                }
            }
            Err(e) => {
                tracing::debug!(target: "worker", epoch, "worker stderr read error: {e}");
                return;
            }
        }
    }
}

/// The level a forwarded worker line claims.
enum WorkerLine {
    Error,
    Warn,
    Info,
    Debug,
}

/// Sniffs the `WLOG` prefix `[<timestamp>] <LEVEL>  <message>` (`Log.h`). Anything
/// else (a raw `fprintf(stderr,…)`, an OCCT message, the test stub's own lines)
/// is INFO — never silently dropped.
fn sniff_worker_level(line: &str) -> WorkerLine {
    let rest = line.split_once("] ").map_or(line, |(_, rest)| rest);
    match rest.split_whitespace().next().unwrap_or_default() {
        "ERROR" => WorkerLine::Error,
        "WARN" => WorkerLine::Warn,
        "DEBUG" => WorkerLine::Debug,
        _ => WorkerLine::Info,
    }
}
