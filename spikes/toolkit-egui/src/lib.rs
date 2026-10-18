//! ONECAD-257 P0.2 toolkit spike, egui half. See
//! `docs/design/native-ui/toolkit-spike-egui.md` for the findings this code produced.

pub mod camera;
pub mod expr;
mod fonts;
#[cfg(target_os = "macos")]
mod gpubench;
mod icons;
mod input;
mod menu;
pub mod mesh1;
pub mod metrics;
mod render;
pub mod scene;
mod theme;
mod tree;
mod ui;
mod viewport;

use camera::Camera;
use eframe::egui;
use input::DimInput;
use scene::Scene;
use theme::Theme;

pub(crate) struct Bench {
    pub frames_left: u32,
    pub warmup: u32,
    pub step: f32,
    pub exit: bool,
}

pub(crate) struct App {
    pub theme: Theme,
    pub scene: Scene,
    pub cam: Camera,
    pub selected: Option<usize>,
    pub dim: DimInput,
    pub menu: Option<menu::NativeMenu>,
    pub bench: Option<Bench>,
    pub cpu_ms: Vec<f64>,
    /// eframe's CPU time of the previous whole frame (`IntegrationInfo::cpu_usage`).
    pub frame_cpu_ms: Vec<f64>,
    pub cursor_world: Option<glam::Vec3>,
    pub status: String,
    pub frame_no: u64,
    pub ortho: bool,
    /// Fit the camera on the next viewport frame, when its aspect is known.
    pub fit_pending: bool,
    pub input_log: bool,
}

fn bench_from_env() -> Option<Bench> {
    let spec = std::env::var("SPIKE_BENCH").ok()?;
    let frames = spec.strip_prefix("orbit:")?.parse().ok()?;
    Some(Bench {
        frames_left: frames,
        warmup: 30,
        step: 0.012,
        exit: std::env::var_os("SPIKE_EXIT").is_some(),
    })
}

/// The edge the dimension measures: a real MESH1 edge, the front edge of the motor's top
/// face. Of the straight horizontal edge segments longer than 20 mm, take the highest (to
/// 0.1 mm), then the front-most (smallest mid Y). Ends ordered by X.
fn dimension_anchor(scene: &Scene) -> [glam::Vec3; 3] {
    use glam::Vec3;
    let mut best: Option<(Vec3, Vec3)> = None;
    for s in scene.bodies.iter().flat_map(|b| &b.mesh.edge_segments) {
        let (a, c) = (Vec3::new(s[0], s[1], s[2]), Vec3::new(s[3], s[4], s[5]));
        if (a.z - c.z).abs() > 1e-3 || a.distance(c) < 20.0 {
            continue;
        }
        let better = best.is_none_or(|(p, q)| {
            let (z, zb) = ((a.z * 10.0).round(), (p.z * 10.0).round());
            z > zb || (z == zb && a.y + c.y < p.y + q.y - 1e-3)
        });
        if better {
            best = Some(if a.x <= c.x { (a, c) } else { (c, a) });
        }
    }
    let (lo, hi) = (scene.bbox_min, scene.bbox_max);
    let (a, b) = best.unwrap_or((
        Vec3::new(lo[0], lo[1], hi[2]),
        Vec3::new(hi[0], lo[1], hi[2]),
    ));
    eprintln!(
        "[metric] dimension_anchor a={a:?} b={b:?} len_mm={:.3}",
        a.distance(b)
    );
    [(a + b) * 0.5, a, b]
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, mut scene: Scene) -> Result<Self, String> {
        // eframe has created the window, the wgpu device and the surface by now.
        metrics::mark("app_new_enter");
        let t = metrics::now_ns();
        let resolved = fonts::install(&cc.egui_ctx);
        eprintln!("[metric] icons parsed={}", icons::count());
        eprintln!(
            "[metric] fonts ui={:?} mono={:?} ms={:.2}",
            resolved.ui,
            resolved.mono,
            (metrics::now_ns() - t) as f64 / 1e6
        );
        let theme = theme::dark();
        theme::apply(&cc.egui_ctx, &theme);
        let rs = cc
            .wgpu_render_state
            .as_ref()
            .ok_or("eframe started without wgpu")?;
        let info = rs.adapter.get_info();
        eprintln!(
            "[metric] adapter name={:?} backend={:?} target_format={:?}",
            info.name, info.backend, rs.target_format
        );
        let t = metrics::now_ns();
        let grid = viewport::grid_lines(&theme, &scene);
        let renderer = render::Renderer::new(&rs.device, rs.target_format, &scene, &grid);
        renderer.set_edge_color(&rs.queue, &scene, theme.body_edge.to_array());
        eprintln!(
            "[metric] gpu_upload bodies={} tris={} edge_segments={} bytes={} ms={:.2}",
            scene.bodies.len(),
            scene.triangle_count(),
            scene.segment_count(),
            renderer.uploaded_bytes,
            (metrics::now_ns() - t) as f64 / 1e6
        );
        rs.renderer.write().callback_resources.insert(renderer);
        let anchor = dimension_anchor(&scene);
        // The GPU owns the geometry now; keep only counts and boxes on the CPU.
        for b in &mut scene.bodies {
            b.mesh.positions = Vec::new();
            b.mesh.normals = Vec::new();
            b.mesh.indices = Vec::new();
            b.mesh.edge_segments = Vec::new();
        }
        let menu = match if std::env::var_os("SPIKE_NO_MENU").is_some() {
            Err("disabled by SPIKE_NO_MENU".to_owned())
        } else {
            menu::install(&cc.egui_ctx)
        } {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("[metric] native_menu unavailable: {e}");
                None
            }
        };
        let width = f64::from((anchor[2] - anchor[1]).length());
        let cam = Camera::fit(scene.bbox_min, scene.bbox_max, 1.0);
        let request_focus = std::env::var_os("SPIKE_AUTOFOCUS_DIM").is_some();
        metrics::mark("app_created");
        Ok(App {
            theme,
            scene,
            cam,
            selected: None,
            dim: DimInput {
                value_mm: width,
                text: format!("{width:.2}"),
                editing: false,
                error: None,
                undo: Vec::new(),
                redo: Vec::new(),
                anchor,
                request_focus,
                id: None,
                composing: false,
            },
            menu,
            bench: bench_from_env(),
            cpu_ms: Vec::new(),
            frame_cpu_ms: Vec::new(),
            cursor_world: None,
            status: "Ready".into(),
            frame_no: 0,
            ortho: false,
            fit_pending: true,
            input_log: std::env::var_os("SPIKE_INPUT_LOG").is_some(),
        })
    }

    fn handle_menu(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let actions = self
            .menu
            .as_ref()
            .map(menu::NativeMenu::drain)
            .unwrap_or_default();
        menu::remove_duplicate_events(&actions, &mut raw.events);
        for action in actions {
            match action {
                menu::Action::Undo | menu::Action::Redo => {
                    self.dim
                        .route_edit(ctx, action, &mut self.status, &mut raw.events);
                }
                menu::Action::Cut
                | menu::Action::Copy
                | menu::Action::Paste
                | menu::Action::SelectAll => {
                    self.dim.route_clipboard(ctx, action, &mut raw.events);
                }
                menu::Action::Fit | menu::Action::Home => self.fit_pending = true,
                menu::Action::Save => self.status = "Save (spike: no-op)".into(),
            }
        }
    }

    fn step_bench(&mut self, ctx: &egui::Context) {
        let Some(b) = &mut self.bench else { return };
        self.cam.orbit(b.step / camera::ORBIT_SPEED, 0.0);
        ctx.request_repaint();
        if b.warmup > 0 {
            b.warmup -= 1;
            if b.warmup == 0 {
                metrics::reset_frames();
                self.cpu_ms.clear();
                self.frame_cpu_ms.clear();
                metrics::rss("bench_start");
            }
            return;
        }
        b.frames_left = b.frames_left.saturating_sub(1);
        if b.frames_left == 0 {
            metrics::report("orbit_bench", &self.cpu_ms, &self.frame_cpu_ms);
            metrics::rss("bench_end");
            let exit = b.exit;
            self.bench = None;
            if exit {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.handle_menu(ctx, raw);
        self.dim.prepare_input(ctx, raw);
        if self.input_log {
            for e in &raw.events {
                match e {
                    egui::Event::Ime(ime) => eprintln!("[ime] {ime:?}"),
                    egui::Event::Text(t) => eprintln!("[key] text {t:?}"),
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        eprintln!(
                            "[key] {key:?} cmd={} shift={} alt={}",
                            modifiers.command, modifiers.shift, modifiers.alt
                        )
                    }
                    egui::Event::Copy | egui::Event::Cut => eprintln!("[key] {e:?}"),
                    egui::Event::PointerButton {
                        button,
                        pressed,
                        pos,
                        ..
                    } => {
                        eprintln!(
                            "[ptr] {button:?} pressed={pressed} x={:.1} y={:.1}",
                            pos.x, pos.y
                        )
                    }
                    egui::Event::WindowFocused(f) => eprintln!("[win] focused={f}"),
                    egui::Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                        ..
                    } => {
                        eprintln!(
                            "[wheel] unit={unit:?} dx={:.2} dy={:.2} shift={}",
                            delta.x, delta.y, modifiers.shift
                        )
                    }
                    egui::Event::Zoom(f) => eprintln!("[wheel] zoom factor={f:.4}"),
                    egui::Event::Paste(t) => eprintln!("[key] paste {t:?}"),
                    _ => {}
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let t0 = metrics::now_ns();
        if let Some(s) = frame.info().cpu_usage {
            self.frame_cpu_ms.push(f64::from(s) * 1e3);
        }
        let ctx = ui.ctx().clone();
        self.frame_no += 1;
        if self.frame_no == 2 {
            let v = ctx.input(|i| i.viewport_rect());
            eprintln!(
                "[metric] frame={} viewport_rect={v:?} ppp={}",
                self.frame_no,
                ctx.pixels_per_point()
            );
        }
        self.step_bench(&ctx);
        ui::title_bar(self, ui);
        ui::status_bar(self, ui);
        ui::tree(self, ui);
        ui::inspector(self, ui);
        viewport::show(self, ui);
        self.cpu_ms.push((metrics::now_ns() - t0) as f64 / 1e6);
        if self.cpu_ms.len() > 100_000 {
            self.cpu_ms.clear();
            self.frame_cpu_ms.clear();
        }
    }
}

pub fn native_options() -> eframe::NativeOptions {
    let novsync = std::env::var_os("SPIKE_NOVSYNC").is_some();
    let mut wgpu_options = egui_wgpu::WgpuConfiguration::default();
    if novsync {
        wgpu_options.surface.present_mode = egui_wgpu::wgpu::PresentMode::AutoNoVsync;
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("OneCAD — NEMA")
        // OneCAD.png's window, so screenshots compare pixel for pixel.
        .with_inner_size([1521.0, 921.0])
        .with_min_inner_size([900.0, 560.0]);
    if let Some((x, y)) = std::env::var("SPIKE_WINDOW_POS").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?))
    }) {
        viewport = viewport.with_position([x, y]);
    }
    if cfg!(target_os = "macos") {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    eframe::NativeOptions {
        viewport,
        multisampling: render::MSAA as u16,
        depth_buffer: 32,
        wgpu_options,
        ..Default::default()
    }
}

pub fn run() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    metrics::init();
    let dir = scene::fixture_dir();
    let t = metrics::now_ns();
    let scene = match scene::load(&dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("fixture load failed: {e}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "[metric] scene_load dir={} bodies={} tris={} bytes={} ms={:.2}",
        dir.display(),
        scene.bodies.len(),
        scene.triangle_count(),
        scene.bytes,
        (metrics::now_ns() - t) as f64 / 1e6
    );
    #[cfg(target_os = "macos")]
    if let Ok(spec) = std::env::var("SPIKE_GPU_BENCH") {
        let (frames, scale) = spec.split_once('x').unwrap_or((&spec, "1"));
        let scale: u32 = scale.parse().unwrap_or(1);
        let size = [1042 * scale, 842 * scale];
        if let Err(e) = gpubench::run(&scene, &theme::dark(), frames.parse().unwrap_or(300), size) {
            eprintln!("gpu bench failed: {e}");
        }
        return Ok(());
    }
    eframe::run_native(
        "OneCAD",
        native_options(),
        Box::new(move |cc| {
            App::new(cc, scene)
                .map(|a| Box::new(a) as Box<dyn eframe::App>)
                .map_err(Into::into)
        }),
    )
}
