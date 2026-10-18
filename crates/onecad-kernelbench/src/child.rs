use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::prepared::PreparedCase;
use crate::result::{synthetic_result, Backend};
use crate::result_validation;

#[derive(Debug)]
struct Captured {
    bytes: Vec<u8>,
    overflow: bool,
}

pub fn run(
    runner: &std::path::Path,
    case: &PreparedCase,
    backend: Backend,
    request: &Value,
) -> Result<Value, Value> {
    let limits = &case.limits.resources;
    let mut command = Command::new(runner);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_memory_limit(&mut command, limits.address_space_bytes);
    let mut child = command.spawn().map_err(|e| {
        failure(
            case,
            backend,
            "spawn-error",
            json!({"state":"spawn-error"}),
            &e.to_string(),
        )
    })?;
    write_request(&mut child, request).map_err(|e| {
        failure(
            case,
            backend,
            "stdin-error",
            json!({"state":"stdin-error"}),
            &e,
        )
    })?;
    let stdout = spawn_reader(child.stdout.take(), limits.stdout_bytes);
    let stderr = spawn_reader(child.stderr.take(), limits.stderr_bytes);
    let outcome = wait_child(
        &mut child,
        Duration::from_millis(limits.wall_time_ms),
        limits.address_space_bytes,
    );
    let captured_out = join_reader(stdout);
    let captured_err = join_reader(stderr);
    child_result(case, backend, outcome, captured_out, captured_err)
}

fn write_request(child: &mut std::process::Child, request: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("request exceeds 1 MiB".into());
    }
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "runner stdin unavailable".to_string())?;
    stdin
        .write_all(&bytes)
        .and_then(|_| stdin.write_all(b"\n"))
        .map_err(|e| e.to_string())
}

fn spawn_reader<R: Read + Send + 'static>(
    reader: Option<R>,
    cap: usize,
) -> thread::JoinHandle<Captured> {
    thread::spawn(move || match reader {
        Some(reader) => read_bounded(reader, cap),
        None => Captured {
            bytes: Vec::new(),
            overflow: false,
        },
    })
}

fn read_bounded(mut reader: impl Read, cap: usize) -> Captured {
    let mut bytes = Vec::with_capacity(cap.min(65_536));
    let mut chunk = [0_u8; 8192];
    let mut overflow = false;
    while let Ok(count) = reader.read(&mut chunk) {
        if count == 0 {
            break;
        }
        let remaining = cap.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..count.min(remaining)]);
        overflow |= count > remaining;
    }
    Captured { bytes, overflow }
}

fn join_reader(handle: thread::JoinHandle<Captured>) -> Captured {
    handle.join().unwrap_or(Captured {
        bytes: Vec::new(),
        overflow: true,
    })
}

#[derive(Debug)]
struct ProcessOutcome {
    status: Option<std::process::ExitStatus>,
    timeout: bool,
    memory_limit: bool,
    peak_rss: Option<u64>,
}

fn wait_child(
    child: &mut std::process::Child,
    timeout: Duration,
    memory_limit: u64,
) -> ProcessOutcome {
    let deadline = Instant::now() + timeout;
    let mut peak_rss = None;
    loop {
        if let Some((_virtual_size, resident_size)) = process_memory(child.id()) {
            peak_rss = Some(peak_rss.unwrap_or(0).max(resident_size));
            if resident_size > memory_limit {
                let _ = child.kill();
                return ProcessOutcome {
                    status: child.wait().ok(),
                    timeout: false,
                    memory_limit: true,
                    peak_rss,
                };
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return ProcessOutcome {
                    status: Some(status),
                    timeout: false,
                    memory_limit: false,
                    peak_rss,
                }
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => break,
        }
    }
    let _ = child.kill();
    ProcessOutcome {
        status: child.wait().ok(),
        timeout: true,
        memory_limit: false,
        peak_rss,
    }
}

fn child_result(
    case: &PreparedCase,
    backend: Backend,
    outcome: ProcessOutcome,
    stdout: Captured,
    stderr: Captured,
) -> Result<Value, Value> {
    let stderr_text = String::from_utf8_lossy(&stderr.bytes);
    if outcome.memory_limit {
        return Err(failure(
            case,
            backend,
            "memory-limit",
            execution(&outcome),
            &stderr_text,
        ));
    }
    if outcome.timeout {
        return Err(failure(
            case,
            backend,
            "timeout",
            execution(&outcome),
            &stderr_text,
        ));
    }
    if stdout.overflow {
        return Err(failure(
            case,
            backend,
            "output-overflow",
            execution(&outcome),
            &stderr_text,
        ));
    }
    let Some(status) = outcome.status else {
        return Err(failure(
            case,
            backend,
            "wait-error",
            execution(&outcome),
            &stderr_text,
        ));
    };
    if !status.success() {
        return Err(failure(
            case,
            backend,
            "crash",
            execution(&outcome),
            &stderr_text,
        ));
    }
    let mut value = parse_stdout(case, backend, &stdout.bytes)
        .map_err(|class| failure(case, backend, class, execution(&outcome), &stderr_text))?;
    value["resource"] = json!({
        "peakRssBytes": outcome.peak_rss,
        "exitCode": status.code(),
        "signal": signal(Some(status)),
        "stderrTruncated": stderr.overflow,
    });
    Ok(value)
}

fn parse_stdout(
    case: &PreparedCase,
    backend: Backend,
    bytes: &[u8],
) -> Result<Value, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "invalid-json")?;
    let lines: Vec<_> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() != 1 {
        return Err("invalid-json");
    }
    let value: Value = serde_json::from_str(lines[0]).map_err(|_| "invalid-json")?;
    result_validation::validate(&value, case, backend)
        .map(|_| value)
        .map_err(|_| "invalid-json")
}

fn failure(
    case: &PreparedCase,
    backend: Backend,
    class: &str,
    execution: Value,
    stderr: &str,
) -> Value {
    synthetic_result(case, backend, class, execution, &bounded_text(stderr, 4096))
}

fn bounded_text(value: &str, cap: usize) -> String {
    value.chars().take(cap).collect()
}

fn execution(outcome: &ProcessOutcome) -> Value {
    let state = if outcome.memory_limit {
        "memoryLimit"
    } else if outcome.timeout {
        "timeout"
    } else {
        "exited"
    };
    json!({"state":state,"exitCode":outcome.status.and_then(|status| status.code()),"signal":signal(outcome.status),"peakRssBytes":outcome.peak_rss})
}

#[cfg(unix)]
fn signal(status: Option<std::process::ExitStatus>) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.and_then(|value| value.signal())
}

#[cfg(not(unix))]
fn signal(_status: Option<std::process::ExitStatus>) -> Option<i32> {
    None
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct ProcTaskInfo {
    virtual_size: u64,
    resident_size: u64,
    times: [u64; 4],
    counters: [i32; 12],
}

#[cfg(target_os = "macos")]
fn process_memory(pid: u32) -> Option<(u64, u64)> {
    const PROC_PIDTASKINFO: i32 = 4;
    let mut info = ProcTaskInfo::default();
    // SAFETY: proc_pidinfo writes at most supplied repr(C) buffer size.
    let written = unsafe {
        proc_pidinfo(
            pid as i32,
            PROC_PIDTASKINFO,
            0,
            (&mut info as *mut ProcTaskInfo).cast(),
            std::mem::size_of::<ProcTaskInfo>() as i32,
        )
    };
    (written == std::mem::size_of::<ProcTaskInfo>() as i32)
        .then_some((info.virtual_size, info.resident_size))
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn proc_pidinfo(
        pid: i32,
        flavor: i32,
        arg: u64,
        buffer: *mut libc::c_void,
        buffer_size: i32,
    ) -> i32;
}

#[cfg(not(target_os = "macos"))]
fn process_memory(_pid: u32) -> Option<(u64, u64)> {
    None
}

// RLIMIT_AS is unreliable on macOS: the OS reserves large virtual address
// space (dyld, frameworks) well before user code runs, independent of actual
// heap use, so a modest limit kills the child immediately at startup. macOS
// enforces the memory limit via the RSS polling loop in `wait_child` instead
// (see `process_memory`); Linux/other Unix keep the pre-exec RLIMIT_AS.
#[cfg(all(unix, not(target_os = "macos")))]
fn configure_memory_limit(command: &mut Command, bytes: u64) {
    use std::os::unix::process::CommandExt;
    // SAFETY: pre_exec calls only async-signal-safe setrlimit and returns its OS error.
    unsafe {
        command.pre_exec(move || {
            let limit = libc::rlimit {
                rlim_cur: bytes as libc::rlim_t,
                rlim_max: bytes as libc::rlim_t,
            };
            if libc::setrlimit(libc::RLIMIT_AS, &limit) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
}

#[cfg(any(not(unix), target_os = "macos"))]
fn configure_memory_limit(_command: &mut Command, _bytes: u64) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reader_drains_and_truncates() {
        let captured = read_bounded(&b"abcdefghij"[..], 4);
        assert_eq!(captured.bytes, b"abcd");
        assert!(captured.overflow);
    }
}
