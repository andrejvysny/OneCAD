//! Measurement probes, all printed to stderr as `[metric] ...` lines.
//! Clock: CLOCK_UPTIME_RAW in ns, the same clock an external CGEvent injector can read,
//! so input-to-submit latency can be joined across processes.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static MAIN_NS: AtomicU64 = AtomicU64::new(0);
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);
static LATENCY: AtomicBool = AtomicBool::new(false);
static FRAME_LOG: AtomicBool = AtomicBool::new(false);
static FRAMES: Mutex<Vec<u64>> = Mutex::new(Vec::new());

pub fn now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid out-pointer; CLOCK_UPTIME_RAW exists on macOS.
    #[cfg(target_os = "macos")]
    unsafe {
        libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut ts);
    }
    #[cfg(not(target_os = "macos"))]
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

pub fn init() {
    MAIN_NS.store(now_ns(), Ordering::Relaxed);
    LATENCY.store(
        std::env::var_os("SPIKE_LATENCY").is_some(),
        Ordering::Relaxed,
    );
    FRAME_LOG.store(
        std::env::var_os("SPIKE_FRAME_LOG").is_some(),
        Ordering::Relaxed,
    );
    eprintln!(
        "[metric] main uptime_ns={}",
        MAIN_NS.load(Ordering::Relaxed)
    );
}

pub fn since_main_ms() -> f64 {
    (now_ns() - MAIN_NS.load(Ordering::Relaxed)) as f64 / 1e6
}

pub fn mark(what: &str) {
    eprintln!(
        "[metric] {what} since_main_ms={:.2} uptime_ns={}",
        since_main_ms(),
        now_ns()
    );
}

/// Called from the paint callback's `prepare`, i.e. inside egui-wgpu's paint, just before
/// it submits the frame and presents the surface.
pub fn on_prepare(probe: Option<[f32; 2]>) {
    let t = now_ns();
    if !FIRST_FRAME.swap(true, Ordering::Relaxed) {
        mark("first_frame_prepare");
        rss("after_first_frame");
    }
    if let Ok(mut f) = FRAMES.lock() {
        f.push(t);
    }
    if FRAME_LOG.load(Ordering::Relaxed) {
        eprintln!("[frame] prepare_ns={t}");
    }
    if LATENCY.load(Ordering::Relaxed)
        && let Some([x, y]) = probe
    {
        eprintln!("[latency] x={x:.1} y={y:.1} prepare_ns={t}");
    }
}

/// Clear the frame timestamp log (start of a benchmark window).
pub fn reset_frames() {
    if let Ok(mut f) = FRAMES.lock() {
        f.clear();
    }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// Summarize intervals between successive frame `prepare`s (frame-to-frame time), the UI
/// build times (`App::ui` only) and eframe's whole-frame CPU (ui + tessellate + paint +
/// submit, vsync wait excluded).
pub fn report(label: &str, ui_ms: &[f64], frame_cpu_ms: &[f64]) {
    let stamps = FRAMES.lock().map(|f| f.clone()).unwrap_or_default();
    if stamps.len() < 3 {
        return;
    }
    let mut iv: Vec<f64> = stamps
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64 / 1e6)
        .collect();
    iv.sort_by(f64::total_cmp);
    let mean = iv.iter().sum::<f64>() / iv.len() as f64;
    eprintln!(
        "[metric] {label} frames={} interval_ms mean={mean:.3} p50={:.3} p95={:.3} p99={:.3} max={:.3} fps={:.1}",
        iv.len(),
        pct(&iv, 0.5),
        pct(&iv, 0.95),
        pct(&iv, 0.99),
        iv[iv.len() - 1],
        1000.0 / mean
    );
    summary(label, "ui_cpu_ms", ui_ms);
    summary(label, "frame_cpu_ms", frame_cpu_ms);
}

fn summary(label: &str, name: &str, values: &[f64]) {
    if values.is_empty() {
        return;
    }
    let mut c = values.to_vec();
    c.sort_by(f64::total_cmp);
    let mean = c.iter().sum::<f64>() / c.len() as f64;
    eprintln!(
        "[metric] {label} {name} mean={mean:.3} p50={:.3} p95={:.3} max={:.3}",
        pct(&c, 0.5),
        pct(&c, 0.95),
        c[c.len() - 1]
    );
}

/// Resident set and physical footprint (macOS); peak RSS elsewhere.
pub fn rss(label: &str) {
    #[cfg(target_os = "macos")]
    // SAFETY: zeroed POD out-structs of the sizes the calls expect; our own pid.
    unsafe {
        let pid = libc::getpid();
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        let n = libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&raw mut info).cast(), size);
        let mut ru: libc::rusage_info_v4 = std::mem::zeroed();
        let ok = libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, (&raw mut ru).cast());
        eprintln!(
            "[metric] rss {label} resident_mb={:.1} phys_footprint_mb={:.1}",
            if n == size {
                info.pti_resident_size as f64 / 1048576.0
            } else {
                -1.0
            },
            if ok == 0 {
                ru.ri_phys_footprint as f64 / 1048576.0
            } else {
                -1.0
            }
        );
    }
    #[cfg(not(target_os = "macos"))]
    // SAFETY: zeroed POD out-struct; RUSAGE_SELF.
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut ru);
        eprintln!(
            "[metric] rss {label} max_rss_mb={:.1}",
            ru.ru_maxrss as f64 / 1048576.0
        );
    }
}
