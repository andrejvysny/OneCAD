//! The central viewport: wgpu paint callback, navigation input, and the overlays drawn over
//! it (floating toolbar, view cube, nav pill, anchored dimension input).
//!
//! Navigation follows `src/viewport/engine/CadOrbitControls.ts`: RMB+Shift orbit, MMB/RMB pan,
//! wheel zoom-to-cursor; trackpad two-finger scroll pan, Shift+scroll orbit, pinch zoom.
//! Unlike the webview, the wheel event says whether it came from a line-stepped wheel or a
//! pixel-precise trackpad.

use crate::render::{Frame as GpuFrame, LineInstance, ViewportCallback};
use crate::ui::{font, icon_button};
use crate::{App, fonts};
use egui::{
    Align2, Color32, CornerRadius, Frame, Margin, MouseWheelUnit, PointerButton, Pos2, Rect, Sense,
    Stroke, Ui, pos2, vec2,
};
use glam::Vec3;

pub fn grid_lines(t: &crate::theme::Theme, scene: &crate::scene::Scene) -> Vec<LineInstance> {
    let extent = (scene.radius() * 6.0 / 10.0).ceil() * 10.0;
    let (minor, major) = (t.grid_minor.to_array(), t.grid_major.to_array());
    let mut v = Vec::new();
    let n = (extent / 5.0) as i32;
    for i in -n..=n {
        let c = i as f32 * 5.0;
        let col = if i % 5 == 0 { major } else { minor };
        v.push(LineInstance {
            a: [c, -extent, 0.0],
            b: [c, extent, 0.0],
            color: col,
            width: -1.0,
        });
        v.push(LineInstance {
            a: [-extent, c, 0.0],
            b: [extent, c, 0.0],
            color: col,
            width: -1.0,
        });
    }
    let r = scene.radius() * 0.25;
    v.push(LineInstance {
        a: [0.0; 3],
        b: [r, 0.0, 0.0],
        color: t.axis_x.to_array(),
        width: 2.0,
    });
    v.push(LineInstance {
        a: [0.0; 3],
        b: [0.0, r, 0.0],
        color: t.axis_y.to_array(),
        width: 2.0,
    });
    v
}

fn ndc(rect: Rect, p: Pos2) -> [f32; 2] {
    [
        ((p.x - rect.min.x) / rect.width()) * 2.0 - 1.0,
        1.0 - ((p.y - rect.min.y) / rect.height()) * 2.0,
    ]
}

fn navigate(app: &mut App, ui: &Ui, rect: Rect, resp: &egui::Response) {
    let aspect = rect.width() / rect.height().max(1.0);
    let (events, pointer, mods) = ui.input(|i| (i.events.clone(), i.pointer.clone(), i.modifiers));
    let hover = pointer.hover_pos().filter(|p| rect.contains(*p));
    if resp.dragged() || resp.drag_started() {
        let d = pointer.delta();
        if pointer.button_down(PointerButton::Secondary) && mods.shift {
            app.cam.orbit(d.x, d.y);
        } else if pointer.button_down(PointerButton::Middle)
            || pointer.button_down(PointerButton::Secondary)
        {
            app.cam.pan(d.x, d.y, rect.height());
        }
    }
    let Some(at) = hover else { return };
    for e in &events {
        match e {
            egui::Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta,
                modifiers,
                ..
            } => {
                // Pixel-precise source: a trackpad two-finger scroll.
                if modifiers.shift {
                    app.cam.orbit(-delta.x - delta.y, 0.0);
                } else {
                    app.cam.pan(-delta.x, -delta.y, rect.height());
                }
            }
            egui::Event::MouseWheel { delta, .. } => {
                let steps = delta.y + delta.x;
                app.cam.zoom_at(0.9f32.powf(steps), ndc(rect, at), aspect);
            }
            // Trackpad pinch (winit PinchGesture → egui Zoom).
            egui::Event::Zoom(f) => app.cam.zoom_at(1.0 / f, ndc(rect, at), aspect),
            _ => {}
        }
    }
}

/// Cursor ray onto world Z=0, for the status bar readout.
fn cursor_on_ground(app: &App, rect: Rect, p: Pos2) -> Option<Vec3> {
    let aspect = rect.width() / rect.height().max(1.0);
    let inv = app.cam.view_proj(aspect).inverse();
    let n = ndc(rect, p);
    let a = inv * glam::Vec4::new(n[0], n[1], 0.0, 1.0);
    let b = inv * glam::Vec4::new(n[0], n[1], 1.0, 1.0);
    let (a, b) = (a.truncate() / a.w, b.truncate() / b.w);
    let d = b - a;
    (d.z.abs() > 1e-9)
        .then(|| a + d * (-a.z / d.z))
        .filter(|_| (-a.z / d.z) > 0.0)
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let frame = Frame::new().fill(app.theme.canvas);
    egui::CentralPanel::no_frame().frame(frame).show(ui, |ui| {
        let rect = ui.max_rect();
        if std::mem::take(&mut app.fit_pending) {
            let aspect = rect.width() / rect.height().max(1.0);
            app.cam = crate::camera::Camera::fit(app.scene.bbox_min, app.scene.bbox_max, aspect);
        }
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, "3D viewport"));
        navigate(app, ui, rect, &resp);
        let probe = ui.input(|i| {
            (i.pointer.delta() != egui::Vec2::ZERO)
                .then(|| i.pointer.latest_pos())
                .flatten()
        });
        if let Some(p) = ui.input(|i| i.pointer.hover_pos())
            && rect.contains(p)
        {
            app.cursor_world = cursor_on_ground(app, rect, p);
        }
        let ppp = ui.ctx().pixels_per_point();
        let aspect = rect.width() / rect.height().max(1.0);
        let colors = (0..app.scene.bodies.len())
            .map(|i| {
                let c = if app.selected == Some(i) {
                    app.theme.accent
                } else {
                    app.theme.body_fill
                };
                c.to_normalized_gamma_f32()
            })
            .collect();
        let gpu = GpuFrame {
            view_proj: app.cam.view_proj(aspect),
            eye: app.cam.eye(),
            target: app.cam.target,
            grid_fade: app.scene.radius() * 6.0,
            visible: app.scene.bodies.iter().map(|b| b.visible).collect(),
            colors,
            probe: probe.map(|p| [p.x, p.y]),
            size_px: [rect.width() * ppp, rect.height() * ppp],
        };
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            ViewportCallback(gpu),
        ));
        toolbar(app, ui, rect);
        view_cube(app, ui, rect);
        nav_pill(app, ui, rect);
        dimension(app, ui, rect);
    });
}

const TOOLS: &[&[(&str, &str)]] = &[
    &[
        ("select", "Select (V)"),
        ("sketch", "New sketch (S)"),
        ("plane", "Datum plane (D)"),
    ],
    &[
        ("extrude", "Extrude (E)"),
        ("revolve", "Revolve (R)"),
        ("fillet", "Fillet / Chamfer (F)"),
        ("booleanUnion", "Combine (B)"),
    ],
    &[
        ("shell", "Shell (K)"),
        ("pushpull", "Offset face (⇧O)"),
        ("hole", "Hole (⇧H)"),
        ("patternLinear", "Linear pattern (P)"),
        ("patternCircular", "Circular pattern (C)"),
        ("mirrorBody", "Mirror (M)"),
        ("move", "Move (T)"),
    ],
    &[("measure", "Measure (?)")],
];

fn floating(t: &crate::theme::Theme) -> Frame {
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(4))
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(140),
        })
}

fn toolbar(app: &mut App, ui: &mut Ui, rect: Rect) {
    let t = app.theme.clone();
    egui::Area::new(ui.scope_id().with("toolbar"))
        .fixed_pos(pos2(ui.ctx().content_rect().center().x, rect.min.y + 12.0))
        .pivot(Align2::CENTER_TOP)
        .show(ui.ctx(), |ui| {
            floating(&t).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                    for (g, group) in TOOLS.iter().enumerate() {
                        if g > 0 {
                            ui.add_space(3.0);
                            let (r, _) = ui.allocate_exact_size(vec2(1.0, 20.0), Sense::hover());
                            ui.painter().rect_filled(r, 0.0, t.border);
                            ui.add_space(3.0);
                        }
                        for (name, tip) in group.iter() {
                            icon_button(ui, &t, name, 34.0, 20.0, *name == "select", tip);
                        }
                    }
                });
            });
        });
}

fn nav_pill(app: &mut App, ui: &mut Ui, rect: Rect) {
    let t = app.theme.clone();
    let mut fit = false;
    egui::Area::new(ui.scope_id().with("nav"))
        .fixed_pos(pos2(rect.min.x + 10.0, rect.max.y - 10.0))
        .pivot(Align2::LEFT_BOTTOM)
        .show(ui.ctx(), |ui| {
            floating(&t).show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (name, tip) in [
                        ("home", "Home view"),
                        ("fit", "Fit all"),
                        ("isolate", "Isolate"),
                        ("stack", "Display"),
                    ] {
                        if icon_button(ui, &t, name, 30.0, 17.0, false, tip).clicked()
                            && (name == "home" || name == "fit")
                        {
                            fit = true;
                        }
                    }
                });
            });
        });
    if fit {
        app.fit_pending = true;
    }
}

/// A small orientation cube: the camera's rotation applied to a unit cube, back faces culled.
fn view_cube(app: &mut App, ui: &mut Ui, rect: Rect) {
    let t = &app.theme;
    // In a narrow window the toolbar reaches the corner: drop below it.
    let toolbar_right = rect.center().x + 300.0;
    let top = if rect.max.x - 110.0 < toolbar_right {
        64.0 + 56.0
    } else {
        64.0
    };
    let c = pos2(rect.max.x - 64.0, rect.min.y + top);
    let v = app.cam.view();
    let s = 30.0;
    let proj = |p: Vec3| {
        let q = v.transform_vector3(p);
        (pos2(c.x + q.x * s, c.y - q.y * s), q.z)
    };
    let faces: [(Vec3, Vec3, Vec3, &str); 6] = [
        (Vec3::Z, Vec3::X, Vec3::Y, "TOP"),
        (-Vec3::Z, Vec3::X, -Vec3::Y, "BOTTOM"),
        (-Vec3::Y, Vec3::X, Vec3::Z, "FRONT"),
        (Vec3::Y, -Vec3::X, Vec3::Z, "BACK"),
        (Vec3::X, Vec3::Y, Vec3::Z, "RIGHT"),
        (-Vec3::X, -Vec3::Y, Vec3::Z, "LEFT"),
    ];
    let p = ui.painter();
    let mut drawn: Vec<(f32, [Pos2; 4], Pos2, &str, f32)> = Vec::new();
    for (n, u, w, name) in faces {
        let facing = v.transform_vector3(n).z;
        if facing <= 0.02 {
            continue;
        }
        let corners = [n - u - w, n + u - w, n + u + w, n - u + w].map(|q| proj(q).0);
        drawn.push((facing, corners, proj(n).0, name, facing));
    }
    drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, corners, center, name, facing) in drawn {
        let shade = Color32::from_gray((60.0 + 40.0 * facing) as u8);
        p.add(egui::Shape::convex_polygon(
            corners.to_vec(),
            t.surface.lerp_to_gamma(shade, 0.25),
            Stroke::new(1.0, t.border_strong),
        ));
        p.text(
            center,
            Align2::CENTER_CENTER,
            name,
            font(8.5 * facing.sqrt().max(0.6), fonts::semibold()),
            t.ink5,
        );
    }
    let mut y = c.y + 64.0;
    for (name, tip, active) in [
        ("body", "Display mode", false),
        ("grid", "Grid", true),
        ("plus", "Add view", false),
    ] {
        let r = Rect::from_center_size(pos2(c.x, y), vec2(36.0, 36.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let f = floating(t).inner_margin(Margin::same(1));
        f.show(&mut child, |ui| {
            icon_button(ui, t, name, 32.0, 17.0, active, tip)
        });
        y += 44.0;
    }
}

/// Floating dimension input anchored to a world point, re-projected every frame.
fn dimension(app: &mut App, ui: &mut Ui, rect: Rect) {
    let t = app.theme.clone();
    let size = [rect.width(), rect.height()];
    let to_screen = |p: Vec3| {
        app.cam
            .world_to_screen(p, size)
            .map(|s| pos2(rect.min.x + s[0], rect.min.y + s[1]))
    };
    let (Some(mid), Some(a), Some(b)) = (
        to_screen(app.dim.anchor[0]),
        to_screen(app.dim.anchor[1]),
        to_screen(app.dim.anchor[2]),
    ) else {
        return;
    };
    // Dimension line with extension ticks, offset toward the viewer's screen-down.
    let off = vec2(0.0, 26.0);
    let p = ui.painter();
    let line = Stroke::new(1.25, t.accent);
    p.line_segment(
        [a, a + off * 1.25],
        Stroke::new(1.0, t.accent.gamma_multiply(0.7)),
    );
    p.line_segment(
        [b, b + off * 1.25],
        Stroke::new(1.0, t.accent.gamma_multiply(0.7)),
    );
    p.line_segment([a + off, b + off], line);
    for (q, dir) in [
        (a + off, (b - a).normalized()),
        (b + off, (a - b).normalized()),
    ] {
        let n = vec2(-dir.y, dir.x) * 3.5;
        p.add(egui::Shape::convex_polygon(
            vec![q, q + dir * 8.0 + n, q + dir * 8.0 - n],
            t.accent,
            Stroke::NONE,
        ));
    }
    let id = ui.scope_id().with("dimension");
    let at = mid + off;
    egui::Area::new(id)
        .fixed_pos(at)
        .pivot(Align2::CENTER_CENTER)
        .show(ui.ctx(), |ui| {
            let err = app.dim.error.is_some();
            let border = if err {
                t.traffic_close
            } else if app.dim.editing {
                t.accent
            } else {
                t.border_strong
            };
            Frame::new()
                .fill(t.surface)
                .stroke(Stroke::new(1.0, border))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(6, 3))
                .shadow(egui::Shadow {
                    offset: [0, 1],
                    blur: 3,
                    spread: 0,
                    color: Color32::from_black_alpha(90),
                })
                .show(ui, |ui| {
                    crate::input::dimension_field(&mut app.dim, &mut app.status, ui, &t)
                });
        });
}
