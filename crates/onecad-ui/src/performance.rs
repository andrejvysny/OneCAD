//! Opt-in numeric timing evidence. GPU completion is not physical presentation.
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Producer and export batches together retain at most 32K records.
const CAPACITY: usize = 16_384;
const EXPORT_INTERVAL: Duration = Duration::from_millis(500);
static TRACE: OnceLock<Option<Arc<Trace>>> = OnceLock::new();

/// Stable schema-1 event codes; numeric values are documented in PERFORMANCE.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Stage {
    TouchArrival = 1,
    DisplayLink = 2,
    HostFrame = 3,
    EditorFrame = 4,
    Tessellate = 5,
    Acquire = 6,
    UiSubmit = 7,
    PresentRequested = 8,
    Accessibility = 9,
    TextInput = 10,
    GpuComplete = 11,
    InputDrain = 12,
    SnapshotCache = 13,
    RustFrame = 14,
    CameraApplied = 20,
    CadSkipped = 21,
    CadEncode = 22,
    CadSubmit = 23,
    HoverPick = 24,
    CadFenceRegistered = 25,
    GpuSamplingComplete = 26,
    GpuCadComplete = 27,
    CadUniforms = 28,
    PreviewDesired = 30,
    PreviewSubmitted = 31,
    PreviewCompleted = 32,
    PreviewObsolete = 33,
    PreviewService = 34,
    PreviewQueued = 35,
    PreviewKernel = 36,
    PreviewScene = 37,
    PreviewPacking = 38,
    PreviewUpload = 39,
    PreviewDisplayed = 40,
    PreviewDrawSubmitted = 41,
}

impl Stage {
    /// Host timing ABI accepts only host-owned event codes.
    pub fn host(code: u32) -> Option<Self> {
        Some(match code {
            1 => Self::TouchArrival,
            2 => Self::DisplayLink,
            3 => Self::HostFrame,
            9 => Self::Accessibility,
            10 => Self::TextInput,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy)]
struct Record {
    sequence: u64,
    timestamp_ns: u64,
    duration_ns: u64,
    stage: Stage,
    correlation: u64,
    values: [u64; 3],
}

struct Buffer {
    records: Vec<Record>,
    sequence: u64,
}

struct Trace {
    origin: Instant,
    buffer: Mutex<Buffer>,
    dropped: AtomicU64,
    exported: AtomicU64,
    in_flight: AtomicU64,
    export_errors: AtomicU64,
    exporter: OnceLock<mpsc::SyncSender<()>>,
}

/// Counters never wait for the producer buffer. `None` means it was busy.
#[derive(Clone, Copy, Debug, Default)]
pub struct Status {
    pub enabled: bool,
    pub buffered: Option<usize>,
    pub dropped: u64,
    pub exported: u64,
    pub in_flight: u64,
    pub export_errors: u64,
}

fn trace() -> Option<&'static Arc<Trace>> {
    TRACE
        .get_or_init(|| {
            (std::env::var("ONECAD_PERF_TRACE").as_deref() == Ok("1"))
                .then(|| Arc::new(Trace::new()))
        })
        .as_ref()
}

pub fn enabled() -> bool {
    trace().is_some()
}

fn nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

impl Trace {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            buffer: Mutex::new(Buffer {
                records: Vec::with_capacity(CAPACITY),
                sequence: 0,
            }),
            dropped: AtomicU64::new(0),
            exported: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            export_errors: AtomicU64::new(0),
            exporter: OnceLock::new(),
        }
    }

    fn push(&self, stage: Stage, correlation: u64, values: [u64; 3], duration_ns: u64) {
        let Ok(mut buffer) = self.buffer.try_lock() else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if buffer.records.len() == CAPACITY {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        buffer.sequence = buffer.sequence.saturating_add(1);
        let sequence = buffer.sequence;
        buffer.records.push(Record {
            sequence,
            timestamp_ns: nanos(self.origin.elapsed()),
            duration_ns,
            stage,
            correlation,
            values,
        });
    }

    fn status(&self) -> Status {
        Status {
            enabled: true,
            buffered: self
                .buffer
                .try_lock()
                .ok()
                .map(|buffer| buffer.records.len()),
            dropped: self.dropped.load(Ordering::Relaxed),
            exported: self.exported.load(Ordering::Relaxed),
            in_flight: self.in_flight.load(Ordering::Relaxed),
            export_errors: self.export_errors.load(Ordering::Relaxed),
        }
    }
}

pub fn status() -> Status {
    trace().map_or_else(Status::default, |trace| trace.status())
}

pub fn record(stage: Stage, correlation: u64, values: [u64; 3]) {
    record_duration(stage, correlation, values, 0);
}

pub fn record_duration(stage: Stage, correlation: u64, values: [u64; 3], duration_ns: u64) {
    if let Some(trace) = trace() {
        trace.push(stage, correlation, values, duration_ns);
    }
}

/// Disabled spans do not read a clock or allocate.
pub struct Span {
    start: Option<Instant>,
    stage: Stage,
    correlation: u64,
    values: [u64; 3],
}

pub fn span(stage: Stage, correlation: u64, values: [u64; 3]) -> Span {
    Span {
        start: enabled().then(Instant::now),
        stage,
        correlation,
        values,
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            record_duration(
                self.stage,
                self.correlation,
                self.values,
                nanos(start.elapsed()),
            );
        }
    }
}

/// Starts one process-local writer. The caller supplies its own permitted output directory.
pub fn initialize(directory: &Path) {
    let Some(trace) = trace() else { return };
    let (sender, receiver) = mpsc::sync_channel(1);
    if trace.exporter.set(sender).is_err() {
        return;
    }
    let directory = directory.to_path_buf();
    let recorder = Arc::clone(trace);
    let result = std::thread::Builder::new()
        .name("onecad-perf-export".into())
        .spawn(move || export_loop(&recorder, &directory, &receiver));
    if let Err(error) = result {
        trace.export_errors.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(%error, "performance exporter could not start");
    }
}

/// Nonblocking, coalesced wake; normal export also runs every 500 ms.
pub fn request_export() {
    if let Some(sender) = trace().and_then(|trace| trace.exporter.get()) {
        let _ = sender.try_send(());
    }
}

fn export_loop(trace: &Trace, directory: &Path, receiver: &mpsc::Receiver<()>) {
    let session = format!(
        "{}-{}",
        std::process::id(),
        nanos(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
        )
    );
    if let Err(error) = write_metadata(directory, &session) {
        trace.export_errors.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(%error, "performance metadata export failed");
    }
    let mut batch = Vec::with_capacity(CAPACITY);
    let mut number = 0_u64;
    let mut last_warning: Option<Instant> = None;
    loop {
        let _ = receiver.recv_timeout(EXPORT_INTERVAL);
        if let Ok(mut buffer) = trace.buffer.try_lock() {
            std::mem::swap(&mut batch, &mut buffer.records);
        }
        if batch.is_empty() {
            continue;
        }
        number = number.saturating_add(1);
        trace.in_flight.store(batch.len() as u64, Ordering::Relaxed);
        let file = directory.join(format!("trace-{session}-{number}.csv"));
        if let Err(error) = write_batch(&file, &batch, trace.status()) {
            trace.export_errors.fetch_add(1, Ordering::Relaxed);
            trace
                .dropped
                .fetch_add(batch.len() as u64, Ordering::Relaxed);
            if last_warning.is_none_or(|last| last.elapsed() >= Duration::from_secs(60)) {
                tracing::warn!(%error, "performance batch export failed");
                last_warning = Some(Instant::now());
            }
        } else {
            trace
                .exported
                .fetch_add(batch.len() as u64, Ordering::Relaxed);
        }
        trace.in_flight.store(0, Ordering::Relaxed);
        batch.clear();
    }
}

fn write_metadata(directory: &Path, session: &str) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    let metadata = serde_json::json!({
        "schemaVersion": 1, "session": session, "packageVersion": env!("CARGO_PKG_VERSION"),
        "debugAssertions": cfg!(debug_assertions), "capacity": CAPACITY * 2,
        "producerCapacity": CAPACITY,
        "exportIntervalMs": EXPORT_INTERVAL.as_millis(), "clock": "Rust session-relative Instant nanoseconds",
        "sourceAndArtifactIdentity": "external companion required",
        "presentation": "GPU completion and present request are not physical presentation"
    });
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(format!("session-{session}.json")))?;
    serde_json::to_writer_pretty(file, &metadata).map_err(std::io::Error::other)
}

fn write_batch(path: &Path, records: &[Record], status: Status) -> std::io::Result<()> {
    let pending = path.with_extension("partial");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    let mut writer = BufWriter::new(file);
    writeln!(
        writer,
        "#schema=1,dropped={},exported_before={},export_errors={},buffered_before={},in_flight={}",
        status.dropped,
        status.exported,
        status.export_errors,
        status.buffered.map_or(u64::MAX, |count| count as u64),
        status.in_flight
    )?;
    writeln!(
        writer,
        "sequence,timestamp_ns,duration_ns,stage,correlation,a,b,c"
    )?;
    for record in records {
        writeln!(
            writer,
            "{},{},{},{},{},{},{},{}",
            record.sequence,
            record.timestamp_ns,
            record.duration_ns,
            record.stage as u16,
            record.correlation,
            record.values[0],
            record.values[1],
            record.values[2]
        )?;
    }
    writer.flush()?;
    drop(writer);
    fs::rename(pending, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_or_contended_buffers_drop_without_growing_or_waiting() {
        let trace = Trace::new();
        for _ in 0..=CAPACITY {
            trace.push(Stage::TouchArrival, 1, [2, 3, 4], 5);
        }
        assert_eq!(trace.status().buffered, Some(CAPACITY));
        assert_eq!(trace.status().dropped, 1);
        let guard = trace.buffer.lock().unwrap();
        trace.push(Stage::TouchArrival, 2, [0; 3], 0);
        assert_eq!(trace.status().dropped, 2);
        assert_eq!(guard.records.len(), CAPACITY);
    }

    #[test]
    fn numeric_export_preserves_correlation_duration_and_loss_status() {
        let trace = Trace::new();
        trace.push(Stage::PreviewSubmitted, 42, [7, 8, 9], 123);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("trace.csv");
        let buffer = trace.buffer.lock().unwrap();
        write_batch(
            &path,
            &buffer.records,
            Status {
                dropped: 2,
                ..Status::default()
            },
        )
        .unwrap();
        let csv = fs::read_to_string(path).unwrap();
        assert!(csv.contains("dropped=2"));
        assert!(csv.lines().last().unwrap().ends_with(",123,31,42,7,8,9"));
    }
}
