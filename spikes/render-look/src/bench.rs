//! Frame-time measurement at the output size.
//!
//! Primary: CPU-side submit-to-done wall time of command buffers that repeat a pass (or a whole
//! frame) REPS times, divided by REPS, median over ROUNDS submissions. Repeating amortises the
//! per-submit driver/wake-up overhead and gives the steady-state GPU cost of the pass.
//! Secondary: single-frame submit-to-done latency, and timestamp queries at pass boundaries,
//! printed as recorded (Apple GPUs sample only at stage boundaries; a negative value is a bad
//! sample and is reported, not hidden).

use std::time::Instant;

use crate::camera::Camera;
use crate::gpu::Gpu;
use crate::look::{self, Look};
use crate::render::{self, frame_passes, EdgeExp, Parts, Pass, Renderer};
use crate::scene::Scene;
use crate::timing::Timer;

const REPS: usize = 40;
const ROUNDS: usize = 11;
const WARMUP: usize = 10;
const FRAMES: usize = 60;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

fn p95(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() as f64 * 0.95) as usize).min(v.len() - 1)]
}

/// ms per repetition of `passes`, recorded REPS times into one command buffer.
fn repeated(gpu: &Gpu, r: &Renderer, passes: &[Pass], quality: bool, parts: Parts, clear: [f32; 4]) -> f64 {
    let mut rounds = Vec::new();
    for round in 0..ROUNDS + 2 {
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        for _ in 0..REPS {
            for &p in passes {
                r.encode_pass(&mut enc, &mut None, p, quality, parts, clear);
            }
        }
        let cb = enc.finish();
        let t0 = Instant::now();
        gpu.queue.submit([cb]);
        gpu.wait();
        if round >= 2 {
            rounds.push(t0.elapsed().as_secs_f64() * 1e3 / REPS as f64);
        }
    }
    median(rounds)
}

struct Latency {
    cpu_median: f64,
    cpu_p95: f64,
    timestamps: Vec<(&'static str, f64, usize)>,
}

/// Single-frame submit-to-done, plus timestamp medians (and the count of negative samples).
fn latency(gpu: &Gpu, r: &Renderer, quality: bool, contact: bool, parts: Parts, clear: [f32; 4]) -> Latency {
    let mut timer = gpu.timestamps.then(|| Timer::new(&gpu.device, &gpu.queue));
    let mut cpu = Vec::new();
    let mut per: Vec<(&'static str, Vec<f64>)> = Vec::new();
    for f in 0..WARMUP + FRAMES {
        if let Some(t) = timer.as_mut() {
            t.reset();
        }
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        render::encode(r, &mut enc, timer.as_mut(), quality, contact, parts, clear);
        let cb = enc.finish();
        let t0 = Instant::now();
        gpu.queue.submit([cb]);
        gpu.wait();
        let dt = t0.elapsed().as_secs_f64() * 1e3;
        let ts = timer.as_ref().map(|t| t.read(&gpu.device)).unwrap_or_default();
        if f < WARMUP {
            continue;
        }
        cpu.push(dt);
        for (k, (label, ms)) in ts.into_iter().enumerate() {
            if per.len() <= k {
                per.push((label, Vec::new()));
            }
            per[k].1.push(ms);
        }
    }
    let timestamps = per
        .into_iter()
        .map(|(l, v)| {
            let neg = v.iter().filter(|x| **x < 0.0).count();
            (l, median(v), neg)
        })
        .collect();
    Latency { cpu_median: median(cpu.clone()), cpu_p95: p95(cpu), timestamps }
}

fn section(gpu: &Gpu, r: &Renderer, title: &str, quality: bool, contact: bool, parts: Parts, clear: [f32; 4]) {
    println!("  {title}");
    let passes = frame_passes(quality, contact, parts);
    let lat = latency(gpu, r, quality, contact, parts, clear);
    println!("    {:<22} {:>9} {:>14}", "pass", "repeated", "timestamp med");
    let mut sum = 0.0;
    for &p in &passes {
        let ms = repeated(gpu, r, &[p], quality, parts, clear);
        sum += ms;
        let ts = lat
            .timestamps
            .iter()
            .find(|(l, _, _)| *l == p.label())
            .map(|(_, m, neg)| format!("{m:8.3} ({neg}/{FRAMES} neg)"))
            .unwrap_or_else(|| "   (none: compute pass)".into());
        println!("    {:<22} {ms:6.3} ms   {ts}", p.label());
    }
    let frame = repeated(gpu, r, &passes, quality, parts, clear);
    println!("    {:<22} {sum:6.3} ms", "sum of passes");
    println!("    {:<22} {frame:6.3} ms   (whole frame repeated {REPS}x per submit)", "frame");
    println!("    {:<22} {:6.3} ms median, {:.3} ms p95 (one frame per submit)", "submit → done", lat.cpu_median, lat.cpu_p95);
}

fn parts(ground: bool, silhouettes: bool, sil_vertex_cull: bool, edges: bool) -> Parts {
    Parts { bodies: true, ground, silhouettes, sil_vertex_cull, edges, a2c: false, edge_exp: EdgeExp::None }
}

fn main_ms(gpu: &Gpu, r: &Renderer, p: Parts, clear: [f32; 4]) -> f64 {
    repeated(gpu, r, &[Pass::Main], true, p, clear)
}

pub fn run(gpu: &Gpu, r: &Renderer, scene: &Scene, cam: &Camera, base: &Look, qual: &Look, size: [u32; 2]) {
    println!(
        "\n  timings at {}x{} (repeated: {REPS} per submit, median of {ROUNDS}; latency: {WARMUP} warm-up + {FRAMES} frames)",
        size[0], size[1]
    );
    let clear = qual.palette.canvas.linear4();
    r.set_frame(gpu, &look::frame_uniform(scene, cam, base, r.size));
    section(gpu, r, "baseline frame (opaque lines, like today)", false, false, Parts::BASELINE, clear);
    let bodies = repeated(gpu, r, &[Pass::Main], false, parts(false, false, false, false), clear);
    println!("  baseline main pass, bodies only: {bodies:6.3} ms");
    let grid = repeated(gpu, r, &[Pass::Main], false, Parts { edges: false, ..Parts::BASELINE }, clear);
    println!("  baseline main pass, bodies + grid: {grid:6.3} ms");
    let blended = repeated(gpu, r, &[Pass::Main], false, Parts::DEFAULT, clear);
    println!("  baseline main pass, lines alpha-blended instead (previous spike): {blended:6.3} ms");
    // The default quality look: geometric silhouette lines (compute compaction + indirect draw).
    let qu = look::frame_uniform(scene, cam, qual, r.size);
    r.set_frame(gpu, &qu);
    section(gpu, r, "quality frame (camera move: contact map cached)", true, false, Parts::ALL, clear);
    section(gpu, r, "quality frame + contact-map rebuild (geometry change)", true, true, Parts::ALL, clear);
    println!("    visible silhouette segments this frame: {} of {} candidates", r.visible_silhouettes(gpu), scene.silhouettes.len());

    println!("  quality main pass, incremental (each row adds one part; repeated timing):");
    println!("    {:<38} {:6.3} ms", "bodies (PBR + IBL + AO)", main_ms(gpu, r, parts(false, false, false, false), clear));
    println!("    {:<38} {:6.3} ms", "+ ground (grid + contact shadow)", main_ms(gpu, r, parts(true, false, false, false), clear));
    println!("    {:<38} {:6.3} ms", "+ silhouette lines (compacted)", main_ms(gpu, r, parts(true, true, false, false), clear));
    println!("    {:<38} {:6.3} ms", "+ feature edges (= default look)", main_ms(gpu, r, Parts::ALL, clear));
    println!("  alternatives (main pass with ground + edges on):");
    println!("    {:<38} {:6.3} ms", "silhouettes culled in line VS", main_ms(gpu, r, parts(true, true, true, true), clear));
    let a2c = Parts { a2c: true, ..Parts::ALL };
    println!("    {:<38} {:6.3} ms", "lines via alpha-to-coverage", main_ms(gpu, r, a2c, clear));
    let a2c_frame = repeated(gpu, r, &frame_passes(true, false, a2c), true, a2c, clear);
    println!("    {:<38} {:6.3} ms   (whole quality frame, contact map cached)", "  quality frame, a2c lines", a2c_frame);
    // Hard coverage (quad = line, no AA margin) through the same pipeline: plain MSAA lines.
    let mut hard = qu;
    hard.lines[2] = 0.0;
    r.set_frame(gpu, &hard);
    println!("    {:<38} {:6.3} ms", "lines hard, MSAA only", main_ms(gpu, r, a2c, clear));
    let hard_frame = repeated(gpu, r, &frame_passes(true, false, a2c), true, a2c, clear);
    println!("    {:<38} {:6.3} ms   (whole quality frame, contact map cached)", "  quality frame, MSAA-only lines", hard_frame);
    r.set_frame(gpu, &qu);
    let mut frag = qu;
    frag.material[2] = 1.0;
    r.set_frame(gpu, &frag);
    println!("    {:<38} {:6.3} ms", "fragment silhouettes (no lines)", main_ms(gpu, r, Parts::DEFAULT, clear));
    r.set_frame(gpu, &qu);
    println!("  edge-cost experiments (main pass, all parts, default blended lines unless noted):");
    for (label, exp) in [
        ("edge quads culled after vertex work", EdgeExp::CulledAfterVs),
        ("edges opaque (no blend; wrong image)", EdgeExp::Opaque),
        ("edges blended + discard uncovered", EdgeExp::Discard),
    ] {
        println!("    {:<38} {:6.3} ms", label, main_ms(gpu, r, Parts { edge_exp: exp, ..Parts::ALL }, clear));
    }
    let (area, sub_px) = edge_coverage(scene, cam, size, qual);
    println!(
        "  feature-edge quads: {:.1} M px of quad area (visible + hidden), {} of {} segments shorter than 1 px on screen",
        area / 1e6,
        sub_px,
        scene.segments.len()
    );
}

/// Screen-space quad area the edge instances rasterise (before depth testing).
fn edge_coverage(scene: &Scene, cam: &Camera, size: [u32; 2], qual: &Look) -> (f64, usize) {
    let u = look::frame_uniform(scene, cam, qual, size);
    let vp = cam.view_proj();
    let h = u.lines[0] as f64 * 0.5 + 1.0;
    let half = glam::DVec2::new(size[0] as f64 * 0.5, size[1] as f64 * 0.5);
    let px = |p: [f32; 3]| {
        let c = vp * glam::Vec4::new(p[0], p[1], p[2], 1.0);
        glam::DVec2::new((c.x / c.w) as f64, (c.y / c.w) as f64) * half
    };
    let mut area = 0.0;
    let mut sub = 0;
    for s in &scene.segments {
        let len = (px(s.b) - px(s.a)).length();
        if len < 1.0 {
            sub += 1;
        }
        area += (len + 2.0 * h) * 2.0 * h;
    }
    (area, sub)
}
