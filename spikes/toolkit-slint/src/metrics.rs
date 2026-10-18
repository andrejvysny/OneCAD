//! Timing collection for the spike measurements. All lines go to stderr with a `[spike]` prefix.

use std::time::{Duration, Instant};

/// Monotonic uptime in ns (CLOCK_UPTIME_RAW), comparable across processes on the same machine.
pub fn uptime_ns() -> u64 {
    // <time.h> on Darwin: CLOCK_UPTIME_RAW = 8.
    unsafe extern "C" {
        fn clock_gettime_nsec_np(clock_id: u32) -> u64;
    }
    // SAFETY: clock_gettime_nsec_np only reads a clock; id 8 is valid on macOS.
    unsafe { clock_gettime_nsec_np(8) }
}

#[derive(Default)]
pub struct Series(Vec<f64>);

impl Series {
    pub fn push(&mut self, d: Duration) {
        self.0.push(d.as_secs_f64() * 1000.0);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// `n=… p50=… p95=… p99=… max=…` in milliseconds.
    pub fn summary(&self) -> String {
        if self.0.is_empty() {
            return "n=0".into();
        }
        let mut v = self.0.clone();
        v.sort_by(f64::total_cmp);
        let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        format!(
            "n={} mean={:.3} p50={:.3} p95={:.3} p99={:.3} max={:.3} ms",
            v.len(),
            mean,
            q(0.5),
            q(0.95),
            q(0.99),
            v[v.len() - 1]
        )
    }
}

#[derive(Default)]
pub struct Metrics {
    /// CPU time to encode and submit the scene pass.
    pub scene_cpu: Series,
    /// BeforeRendering to AfterRendering: Slint's UI render plus the scene encode, CPU side.
    pub frame_cpu: Series,
    /// Interval between consecutive presented frames that re-rendered the scene.
    pub frame_interval: Series,
    /// Raw winit event arrival to AfterRendering (just before present) of the frame it caused.
    pub input_latency: Series,
    pub last_after: Option<Instant>,
    pub frames: u64,
    /// Every Slint render pass (BeforeRendering), with or without a scene re-render.
    pub slint_frames: u64,
    pub log_frames: bool,
}

impl Metrics {
    pub fn report(&self, tag: &str) {
        eprintln!(
            "[spike] {tag} slint_frames={} scene_frames={}",
            self.slint_frames, self.frames
        );
        eprintln!("[spike] {tag} scene_cpu   {}", self.scene_cpu.summary());
        eprintln!("[spike] {tag} frame_cpu   {}", self.frame_cpu.summary());
        eprintln!(
            "[spike] {tag} frame_intvl {}",
            self.frame_interval.summary()
        );
        eprintln!(
            "[spike] {tag} input_to_present {}",
            self.input_latency.summary()
        );
    }

    pub fn reset(&mut self) {
        self.scene_cpu.clear();
        self.frame_cpu.clear();
        self.frame_interval.clear();
        self.input_latency.clear();
        self.last_after = None;
        self.slint_frames = 0;
        self.frames = 0;
    }
}

/// Minimal stderr logger for winit/wgpu/Slint diagnostics, enabled by `SPIKE_LOG=<level>`.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("[log {} {}] {}", r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

pub fn init_log() {
    static LOGGER: StderrLog = StderrLog;
    if let Ok(level) = std::env::var("SPIKE_LOG")
        && let Ok(filter) = level.parse::<log::LevelFilter>()
        && log::set_logger(&LOGGER).is_ok()
    {
        log::set_max_level(filter);
    }
}
