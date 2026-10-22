use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use super::*;

/// Records what the worker sent through the callbacks.
#[derive(Default)]
struct Recorder {
    lines: Mutex<Vec<(LogLevel, String)>>,
    panic_on_log: bool,
}

impl WorkerCallbacks for Recorder {
    fn log(&self, level: LogLevel, message: &str) {
        self.lines
            .lock()
            .unwrap()
            .push((level, message.to_string()));
        assert!(
            !self.panic_on_log,
            "callback panic must stay on this side of the ABI"
        );
    }
}

/// A socketpair: (host end, worker input, worker output = dup of the input).
fn pair() -> (UnixStream, RunConfig) {
    let (host, worker) = UnixStream::pair().unwrap();
    let input: OwnedFd = worker.into();
    let output = input.try_clone().unwrap();
    let config = RunConfig {
        input,
        output,
        log_level: LogLevel::Info,
        occt_lock_wait_ms: 0,
    };
    (host, config)
}

#[test]
fn log_levels_round_trip_and_unknown_is_info() {
    for level in [
        LogLevel::Error,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
    ] {
        assert_eq!(LogLevel::from_abi(level.to_abi()), level);
    }
    assert_eq!(LogLevel::from_abi(42), LogLevel::Info);
    assert_eq!(JournalKind::from_abi(7), None);
}

#[cfg(not(onecad_embedded_linked))]
#[test]
fn stub_build_reports_not_linked_and_releases_the_fds() {
    assert!(!is_linked());
    assert_eq!(abi_version(), Err(NotLinked));
    assert!(identity_json().is_none());
    let (mut host, config) = pair();
    let recorder: Arc<dyn WorkerCallbacks> = Arc::new(Recorder::default());
    assert_eq!(run(config, recorder), Err(NotLinked));
    let mut buf = [0u8; 1];
    assert_eq!(
        host.read(&mut buf).unwrap(),
        0,
        "the worker end was closed: EOF"
    );
}

/// Reads one OCW1 frame's JSON (12-byte header: magic, json len, bin len).
#[cfg(onecad_embedded_linked)]
fn read_frame_json(host: &mut UnixStream) -> serde_json::Value {
    let mut header = [0u8; 12];
    host.read_exact(&mut header).unwrap();
    assert_eq!(&header[..4], b"OCW1");
    let json_len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let bin_len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let mut body = vec![0u8; json_len + bin_len];
    host.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body[..json_len]).unwrap()
}

#[cfg(onecad_embedded_linked)]
#[test]
fn linked_worker_says_hello_matching_its_identity_and_exits_cleanly_on_eof() {
    assert_eq!(abi_version(), Ok(ABI_VERSION));
    let identity: serde_json::Value = serde_json::from_str(identity_json().unwrap()).unwrap();
    let (mut host, config) = pair();
    let recorder = Arc::new(Recorder::default());
    let callbacks: Arc<dyn WorkerCallbacks> = recorder.clone();
    let worker = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || run(config, callbacks))
        .unwrap();
    let hello = read_frame_json(&mut host);
    let result = &hello["result"];
    assert_eq!(
        result["occt"]["fingerprint"],
        identity["occt"]["fingerprint"]
    );
    assert_eq!(result["protocolVersion"], identity["protocolVersion"]);
    host.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(worker.join().unwrap(), Ok(0));
    let lines = recorder.lines.lock().unwrap();
    assert!(
        lines.iter().any(|(_, line)| line.contains("starting")),
        "{lines:?}"
    );
}

#[cfg(onecad_embedded_linked)]
#[test]
fn a_panicking_callback_is_contained_and_the_run_completes() {
    let (mut host, config) = pair();
    let callbacks: Arc<dyn WorkerCallbacks> = Arc::new(Recorder {
        panic_on_log: true,
        ..Recorder::default()
    });
    let worker = std::thread::spawn(move || run(config, callbacks));
    let _ = read_frame_json(&mut host);
    host.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(worker.join().unwrap(), Ok(0));
}

#[cfg(onecad_embedded_linked)]
#[test]
fn trampolines_tolerate_null_pointers_and_short_events() {
    use std::ffi::c_void;
    let recorder = Arc::new(Recorder::default());
    let holder: Arc<dyn WorkerCallbacks> = recorder.clone();
    let ctx = std::ptr::from_ref(&holder).cast_mut().cast::<c_void>();
    let short = ffi::RawJournalEvent {
        struct_size: 4,
        event: 0,
        request_id: 1,
        name: std::ptr::null(),
        step_index: 0,
        op_position: 0,
        prefix_hash: std::ptr::null(),
        occt_fingerprint: std::ptr::null(),
    };
    // SAFETY: every pointer is NULL or valid for the call; `ctx` names `holder`.
    unsafe {
        ffi::log_trampoline(2, std::ptr::null(), ctx);
        ffi::log_trampoline(2, std::ptr::null(), std::ptr::null_mut());
        ffi::journal_trampoline(std::ptr::null(), ctx);
        ffi::journal_trampoline(&short, ctx);
    }
    let lines = recorder.lines.lock().unwrap();
    assert_eq!(lines.as_slice(), &[(LogLevel::Info, String::new())]);
}

#[cfg(onecad_embedded_linked)]
#[test]
fn a_short_config_is_refused_reading_nothing_past_its_size() {
    use std::io::Write;
    use std::os::fd::{AsRawFd, IntoRawFd};
    // Too short to hold the fds: refused, and nothing closed (fds unknown).
    let (mut host, worker) = UnixStream::pair().unwrap();
    let fd = worker.as_raw_fd();
    assert_eq!(ffi::run_config_sized(4, fd, fd), 3);
    host.write_all(b"x").unwrap();
    let mut byte = [0u8; 1];
    (&worker)
        .read_exact(&mut byte)
        .expect("the caller's fd is still open");
    // Holds both fds but not the whole struct: refused, both closed.
    let (mut host, worker) = UnixStream::pair().unwrap();
    let output: OwnedFd = worker.try_clone().unwrap().into();
    let fds_end = u32::try_from(3 * std::mem::size_of::<i32>()).unwrap();
    assert_eq!(
        ffi::run_config_sized(fds_end, worker.into_raw_fd(), output.into_raw_fd()),
        3
    );
    assert_eq!(
        host.read(&mut byte).unwrap(),
        0,
        "EOF: both worker fds closed"
    );
}
