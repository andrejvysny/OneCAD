//! OneCAD Phase 0 toolkit spike (ONECAD-257), Slint half. See
//! docs/design/native-ui/toolkit-spike-slint.md for what this proves and the measurements.

slint::include_modules!();

pub mod camera;
mod commands;
pub mod expr;
mod input;
pub mod mesh1;
mod metrics;
mod render;
mod state;

mod tokens {
    include!(concat!(env!("OUT_DIR"), "/tokens.rs"));
}

use metrics::uptime_ns;
use slint::wgpu_30::{WGPUConfiguration, WGPUSettings};
#[cfg(target_os = "macos")]
use slint::winit_030::winit;
use slint::{ComponentHandle, ModelRc, VecModel};
use state::{Bench, State, format_mm};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(crate) type Shared = Rc<RefCell<State>>;

fn token(name: &str) -> [u8; 4] {
    tokens::DARK
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .unwrap_or([255, 0, 255, 255])
}

fn linear(name: &str) -> [f32; 4] {
    let c = token(name);
    let f = |u: u8| {
        let s = f32::from(u) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    [f(c[0]), f(c[1]), f(c[2]), f32::from(c[3]) / 255.0]
}

struct Palette {
    clear: [f64; 3],
    fill: [f32; 4],
    pal: [[f32; 4]; 8],
}

fn palette() -> Palette {
    let bg = linear("canvas");
    let mut minor = linear("viewport-grid-minor");
    minor[3] = 0.55;
    Palette {
        clear: [f64::from(bg[0]), f64::from(bg[1]), f64::from(bg[2])],
        fill: linear("body-fill"),
        pal: [
            linear("body-edge"),
            linear("accent"),
            minor,
            linear("viewport-grid-major"),
            linear("axis-x"),
            linear("axis-y"),
            // Hemisphere ambient (lighting constants, not design colors): sky, ground.
            [1.25, 1.25, 1.25, 1.0],
            [0.25, 0.25, 0.25, 1.0],
        ],
    }
}

fn uniforms(st: &State, size: (u32, u32), scale: f32, p: &Palette) -> render::Uniforms {
    let cam = &st.cam;
    let eye = cam.eye();
    let back = (eye - cam.target).normalize();
    let r0 = (cam.distance * 0.9).max(60.0);
    render::Uniforms {
        vp: cam
            .view_proj(size.0 as f32 / size.1 as f32)
            .to_cols_array_2d(),
        eye: [eye.x, eye.y, eye.z, if cam.ortho { 0.0 } else { 1.0 }],
        back: [back.x, back.y, back.z, 0.0],
        screen: [size.0 as f32, size.1 as f32, 1.25 * scale, 1.0 * scale],
        grid: [cam.target.x, cam.target.y, r0, r0 * 2.6],
        pal: p.pal,
    }
}

pub(crate) static LOG_INPUT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) fn log_input(msg: std::fmt::Arguments) {
    if LOG_INPUT.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("[spike] input {msg}");
    }
}

pub(crate) fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| v != "0")
}

pub fn run() -> Result<(), slint::PlatformError> {
    let t_main = Instant::now();
    // fontique resolves the hidden ".SF NS Mono" family to SFNSMonoItalic.ttf. Registering the
    // upright system file (read from disk at runtime, not bundled) makes it resolve upright.
    if std::env::var_os("SLINT_FONT_PATH").is_none() {
        // SAFETY: first statement after the clock read, before Slint, winit or wgpu start threads.
        unsafe { std::env::set_var("SLINT_FONT_PATH", "/System/Library/Fonts/SFNSMono.ttf") };
    }
    metrics::init_log();
    eprintln!("[spike] start uptime_ns={}", uptime_ns());
    let dir = mesh1::fixture_dir();
    let bodies = mesh1::load_fixture(&dir).map_err(slint::PlatformError::Other)?;
    let tris: usize = bodies.iter().map(|b| b.mesh.indices.len() / 3).sum();
    let edges: usize = bodies.iter().map(|b| b.mesh.edge_ranges.len()).sum();
    eprintln!(
        "[spike] fixture {} bodies={} tris={tris} edges={edges} load_ms={:.1}",
        dir.display(),
        bodies.len(),
        t_main.elapsed().as_secs_f64() * 1000.0
    );

    // Slint's default device limits are `downlevel_webgl2_defaults` (no storage buffers); the
    // viewport needs one storage buffer, which the downlevel set allows.
    let mut settings = WGPUSettings::default();
    settings.device_required_limits = slint::wgpu_30::wgpu::Limits::downlevel_defaults();
    let selector =
        slint::BackendSelector::new().require_wgpu_30(WGPUConfiguration::Automatic(settings));
    #[cfg(target_os = "macos")]
    let selector = selector.with_winit_window_attributes_hook(|a| {
        use winit::platform::macos::WindowAttributesExtMacOS;
        a.with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true)
            .with_accepts_first_mouse(true)
    });
    selector.select()?;

    let app = AppWindow::new()?;
    eprintln!(
        "[spike] window_created ms_since_main={:.1}",
        t_main.elapsed().as_secs_f64() * 1000.0
    );
    app.set_ui_font(
        std::env::var("ONECAD_UI_FONT")
            .unwrap_or_else(|_| ".SF NS".into())
            .into(),
    );
    app.global::<Theme>().set_mono_font(
        std::env::var("ONECAD_MONO_FONT")
            .unwrap_or_else(|_| ".SF NS Mono".into())
            .into(),
    );
    let mut st = State::new(&bodies, t_main);
    st.exit_after_first_frame = env_flag("SPIKE_EXIT_AFTER_FIRST_FRAME");
    st.metrics.log_frames = env_flag("SPIKE_LOG_FRAMES");
    let rows: Rc<VecModel<BodyRow>> = Rc::new(VecModel::from(
        st.bodies
            .iter()
            .map(|b| BodyRow {
                name: b.label.clone().into(),
                visible: true,
                selected: false,
            })
            .collect::<Vec<_>>(),
    ));
    app.set_bodies(ModelRc::from(rows.clone()));
    app.set_dim_text(format_mm(st.dim_value).trim_end_matches(" mm").into());
    let state: Shared = Rc::new(RefCell::new(st));

    install_rendering(&app, &state, Rc::new(bodies))?;
    input::install_raw_input(&app, &state);
    input::install_viewport_input(&app, &state);
    commands::install_tree(&app, &state, &rows);
    commands::install_dimension(&app, &state);
    commands::install_menu(&app, &state);

    if env_flag("SPIKE_NO_CHROME") {
        app.set_show_chrome(false);
    }
    if let Ok(size) = std::env::var("SPIKE_WINDOW")
        && let Some((w, h)) = size.split_once('x')
        && let (Ok(w), Ok(h)) = (w.parse::<f32>(), h.parse::<f32>())
    {
        app.window().set_size(slint::LogicalSize::new(w, h));
    }
    if let Ok(pos) = std::env::var("SPIKE_WINDOW_POS")
        && let Some((x, y)) = pos.split_once(',')
        && let (Ok(x), Ok(y)) = (x.parse::<f32>(), y.parse::<f32>())
    {
        app.window().set_position(slint::LogicalPosition::new(x, y));
    }
    if let Ok(n) = std::env::var("SPIKE_SELECT")
        && let Ok(i) = n.parse::<i32>()
    {
        app.invoke_body_clicked(i);
    }
    if env_flag("SPIKE_DIM_FOCUS") {
        let weak = app.as_weak();
        slint::Timer::single_shot(Duration::from_millis(400), move || {
            if let Some(app) = weak.upgrade() {
                app.invoke_dim_begin();
                if let Ok(t) = std::env::var("SPIKE_DIM_TEXT") {
                    app.set_dim_text(t.clone().into());
                    app.invoke_dim_edited(t.into());
                }
            }
        });
    }
    app.run()
}

pub(crate) fn place_dim(app: &AppWindow, st: &State) {
    let (w, h) = (app.get_vp_width(), app.get_vp_height());
    match st.cam.project(st.dim_anchor, w, h) {
        Some((x, y)) if x >= 0.0 && y >= 0.0 && x <= w && y <= h => {
            app.set_dim_visible(true);
            app.set_dim_x(x);
            app.set_dim_y(y);
        }
        _ => app.set_dim_visible(app.get_dim_focused()),
    }
}

fn install_rendering(
    app: &AppWindow,
    state: &Shared,
    bodies: Rc<Vec<mesh1::Body>>,
) -> Result<(), slint::PlatformError> {
    let weak = app.as_weak();
    let state = state.clone();
    let pal = palette();
    let mut before: Option<Instant> = None;
    let mut gpu_bench: Option<u32> = std::env::var("SPIKE_GPU_BENCH")
        .ok()
        .and_then(|v| v.parse().ok());
    let mut renderer: Option<render::Renderer> = None;
    let mut last_size = (0, 0);
    let mut rendered = false;
    let bench_frames: Option<u32> = std::env::var("SPIKE_BENCH")
        .ok()
        .and_then(|v| v.parse().ok());
    let notifier = move |rs: slint::RenderingState, api: &slint::GraphicsAPI| {
        let Some(app) = weak.upgrade() else { return };
        match rs {
            slint::RenderingState::RenderingSetup => {
                if let slint::GraphicsAPI::WGPU30 { device, queue, .. } = api {
                    let r = render::Renderer::new(device, queue, &bodies);
                    let (t, l) = r.counts();
                    eprintln!(
                        "[spike] gpu_upload_ms={:.1} tris={t} line_instances={l} adapter={:?}",
                        r.upload_time.as_secs_f64() * 1000.0,
                        device.adapter_info().name
                    );
                    renderer = Some(r);
                    let mut st = state.borrow_mut();
                    st.bodies_dirty = true;
                    st.scene_dirty = true;
                }
            }
            slint::RenderingState::BeforeRendering => {
                rendered = false;
                before = Some(Instant::now());
                state.borrow_mut().metrics.slint_frames += 1;
                let Some(r) = renderer.as_mut() else { return };
                let size = (
                    app.get_vp_phys_width().max(1) as u32,
                    app.get_vp_phys_height().max(1) as u32,
                );
                let mut st = state.borrow_mut();
                if st.bodies_dirty {
                    r.set_bodies(&st.body_rows(pal.fill));
                    st.bodies_dirty = false;
                    st.scene_dirty = true;
                }
                if !st.scene_dirty && size == last_size {
                    return;
                }
                if size != last_size {
                    if last_size == (0, 0) {
                        st.fit_home(size.0 as f32 / size.1 as f32);
                    }
                    place_dim(&app, &st);
                }
                if let Some(n) = gpu_bench.take() {
                    gpu_benchmark(r, &mut st, size, app.window().scale_factor(), &pal, n);
                }
                let t = Instant::now();
                let u = uniforms(&st, size, app.window().scale_factor(), &pal);
                let (tex, fresh) = r.render(size, &u, pal.clear);
                st.metrics.scene_cpu.push(t.elapsed());
                if fresh && let Ok(img) = slint::Image::try_from(tex) {
                    app.set_viewport_image(img);
                }
                last_size = size;
                st.scene_dirty = false;
                st.inflight_input = st.pending_input.take();
                rendered = true;
            }
            slint::RenderingState::AfterRendering => {
                if !rendered {
                    return;
                }
                let mut st = state.borrow_mut();
                if let Some(t) = before.take() {
                    st.metrics.frame_cpu.push(t.elapsed());
                }
                after_scene_frame(&app, &mut st, bench_frames);
            }
            slint::RenderingState::RenderingTeardown => renderer = None,
            _ => {}
        }
    };
    app.window()
        .set_rendering_notifier(notifier)
        .map_err(|e| slint::PlatformError::Other(format!("{e:?}")))
}

/// Renders `n` scene frames back to back and waits for the GPU after each one, so the wall time
/// per frame is the scene pass's real cost (encode + submit + GPU execution + MSAA resolve).
fn gpu_benchmark(
    r: &mut render::Renderer,
    st: &mut State,
    size: (u32, u32),
    scale: f32,
    pal: &Palette,
    n: u32,
) {
    let yaw0 = st.cam.yaw;
    let mut series = metrics::Series::default();
    for i in 0..n {
        st.cam.yaw = yaw0 + std::f32::consts::TAU * i as f32 / n as f32;
        let u = uniforms(st, size, scale, pal);
        let t = Instant::now();
        r.render(size, &u, pal.clear);
        r.wait_idle();
        series.push(t.elapsed());
    }
    st.cam.yaw = yaw0;
    eprintln!(
        "[spike] gpu_bench viewport_px={}x{} msaa=4 {}",
        size.0,
        size.1,
        series.summary()
    );
}

fn after_scene_frame(app: &AppWindow, st: &mut State, bench_frames: Option<u32>) {
    let now = Instant::now();
    if let Some(prev) = st.metrics.last_after
        && now - prev < Duration::from_millis(250)
    {
        st.metrics.frame_interval.push(now - prev);
    }
    st.metrics.last_after = Some(now);
    st.metrics.frames += 1;
    if let Some(t) = st.inflight_input.take() {
        st.metrics.input_latency.push(now - t);
        if st.metrics.log_frames {
            eprintln!(
                "[spike] frame {} input_to_present_ms={:.3} uptime_ns={}",
                st.metrics.frames,
                (now - t).as_secs_f64() * 1000.0,
                uptime_ns()
            );
        }
    }
    if !st.first_frame_done {
        st.first_frame_done = true;
        eprintln!(
            "[spike] first_frame ms_since_main={:.1} uptime_ns={}",
            st.t_main.elapsed().as_secs_f64() * 1000.0,
            uptime_ns()
        );
        if st.exit_after_first_frame {
            let _ = slint::quit_event_loop();
        }
        if let Some(frames) = bench_frames {
            start_bench(st, frames, true);
            app.window().request_redraw();
        }
        return;
    }
    let Some(b) = st.bench.as_mut() else { return };
    b.frame += 1;
    let (frame, frames, yaw0, exit) = (b.frame, b.frames, b.start_yaw, b.exit_after);
    st.cam.yaw = yaw0 + std::f32::consts::TAU * frame as f32 / frames as f32;
    st.scene_dirty = true;
    place_dim(app, st);
    if frame >= frames {
        st.bench = None;
        eprintln!(
            "[spike] bench done viewport_px={}x{} scale={}",
            app.get_vp_phys_width(),
            app.get_vp_phys_height(),
            app.window().scale_factor()
        );
        st.metrics.report("bench");
        if exit {
            let _ = slint::quit_event_loop();
        }
    }
    app.window().request_redraw();
}

pub(crate) fn start_bench(st: &mut State, frames: u32, exit_after: bool) {
    st.metrics.reset();
    st.bench = Some(Bench {
        frame: 0,
        frames,
        start_yaw: st.cam.yaw,
        exit_after,
    });
    st.scene_dirty = true;
    eprintln!("[spike] bench start frames={frames}");
}
