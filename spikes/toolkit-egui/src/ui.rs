//! Chrome around the viewport, styled from tokens.css to match OneCAD.png: title bar, body
//! tree (left), inspector (right), status bar. Sizes are the Tailwind classes of
//! `src/features/{shell,tree,inspector}` (h-8 rows, 220 px tree, 34 px status bar).

use crate::theme::Theme;
use crate::{App, fonts, icons};
use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Color32, CornerRadius, FontFamily, FontId, Frame, Layout, Margin, Rect, Sense, Ui, vec2,
};

pub const TREE_W: f32 = 220.0;
pub const INSPECTOR_W: f32 = 259.0;

pub fn font(size: f32, family: FontFamily) -> FontId {
    FontId::new(size, family)
}

pub fn label(
    ui: &mut Ui,
    text: &str,
    size: f32,
    color: Color32,
    family: FontFamily,
) -> egui::Response {
    ui.label(
        egui::RichText::new(text)
            .font(font(size, family))
            .color(color),
    )
}

/// "BODIES"-style header: 11 px medium uppercase, 0.07 em tracking, ink-5.
fn section_label(ui: &mut Ui, app: &App, text: &str) {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font(11.0, fonts::medium()),
            color: app.theme.ink5,
            extra_letter_spacing: 0.77,
            ..Default::default()
        },
    );
    ui.label(job);
}

/// A square icon button (ToolButton.tsx look). `active` = bg-sel-bg + accent glyph.
pub fn icon_button(
    ui: &mut Ui,
    t: &Theme,
    name: &str,
    size: f32,
    glyph: f32,
    active: bool,
    tip: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let bg = if active {
        t.sel_bg
    } else if resp.hovered() {
        t.hover3
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    let fg = if active { t.accent } else { t.ink4 };
    let accent = if active { t.accent } else { t.icon_accent };
    let g = Rect::from_center_size(rect.center(), vec2(glyph, glyph));
    icons::paint(ui.painter(), g, name, fg, accent, 1.7);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    resp.on_hover_text(tip)
}

fn separator_v(ui: &mut Ui, color: Color32, h: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, h), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color);
}

pub fn title_bar(app: &mut App, ui: &mut Ui) {
    let t = app.theme.clone();
    let frame = Frame::new()
        .fill(t.titlebar)
        .inner_margin(Margin::symmetric(12, 0));
    egui::Panel::top("title")
        .exact_size(44.0)
        .frame(frame)
        .resizable(false)
        .show(ui, |ui| {
            let bar = ui.max_rect();
            // The whole bar drags the window (the traffic lights sit in the inset on the left).
            let drag = ui.interact(bar, ui.scope_id().with("drag"), Sense::click_and_drag());
            drag.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TitleBar, true, "OneCAD — NEMA")
            });
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let max = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            ui.horizontal_centered(|ui| {
                ui.add_space(if cfg!(target_os = "macos") { 74.0 } else { 0.0 });
                label(ui, "File", 13.0, t.ink3, FontFamily::Proportional);
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                icons::paint(ui.painter(), r, "chevronDown", t.ink5, t.ink5, 2.0);
                ui.add_space(10.0);
                label(
                    ui,
                    "OneCAD — NEMA",
                    13.0,
                    t.titlebar_text,
                    fonts::semibold(),
                );
                label(ui, "•", 13.0, t.ink6, FontFamily::Proportional);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    icon_button(ui, &t, "close", 26.0, 16.0, false, "Close document");
                    ui.add_space(6.0);
                    icon_button(ui, &t, "body", 26.0, 16.0, false, "Workspace: Design");
                });
            });
        });
}

pub fn status_bar(app: &mut App, ui: &mut Ui) {
    let t = app.theme.clone();
    let frame = Frame::new()
        .fill(t.statusbar)
        .inner_margin(Margin::symmetric(14, 0));
    egui::Panel::bottom("status")
        .exact_size(35.0)
        .frame(frame)
        .resizable(false)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // StatusBar.tsx `gap-3`.
                ui.spacing_mut().item_spacing.x = 12.0;
                label(ui, &app.status, 12.0, t.ink3, FontFamily::Proportional);
                separator_v(ui, t.border, 14.0);
                // OneCAD.png shows a grey placeholder; today's StatusBar.tsx hides it with nothing selected.
                label(ui, "DOF: —", 12.0, t.ink5, FontFamily::Proportional);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    label(ui, "mm", 11.5, t.ink5, FontFamily::Proportional);
                    let w = app.cursor_world.unwrap_or_default();
                    let coords = format!("X {:>8.2}   Y {:>8.2}   Z {:>7.2}", w.x, w.y, w.z);
                    label(ui, &coords, 11.5, t.ink4, FontFamily::Monospace);
                    separator_v(ui, t.border, 14.0);
                    label(
                        ui,
                        &format!("FOV {:.0}°", crate::camera::FOV_DEG),
                        11.5,
                        t.ink5,
                        FontFamily::Proportional,
                    );
                    projection_toggle(app, ui, &t);
                });
            });
        });
}

/// `src/ui/SegmentedToggle.tsx` size "sm": a canvas well with 2 px padding holding 20 px
/// segments (`px-2.5`, 11 px semibold); the active one is `bg-sel-bg text-accent`.
fn projection_toggle(app: &mut App, ui: &mut Ui, t: &Theme) {
    let segments = [("Persp", !app.ortho), ("Ortho", app.ortho)];
    let galleys: Vec<_> = segments
        .iter()
        .map(|(name, on)| {
            let color = if *on { t.accent } else { t.ink4 };
            ui.painter()
                .layout_no_wrap((*name).into(), font(11.0, fonts::semibold()), color)
        })
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 20.0).collect();
    let size = vec2(widths.iter().sum::<f32>() + 4.0, 24.0);
    let (well, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(well, CornerRadius::same(4), t.canvas);
    let mut x = well.min.x + 2.0;
    for (((name, on), galley), w) in segments.into_iter().zip(galleys).zip(widths) {
        let seg = Rect::from_min_size(egui::pos2(x, well.min.y + 2.0), vec2(w, 20.0));
        x += w;
        let resp = ui.interact(
            seg,
            ui.scope_id().with(("projection", name)),
            Sense::click(),
        );
        if on {
            ui.painter()
                .rect_filled(seg, CornerRadius::same(4), t.sel_bg);
        }
        ui.painter()
            .galley(seg.center() - galley.size() * 0.5, galley, t.ink4);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, name));
        if resp.clicked() {
            app.ortho = name == "Ortho";
        }
    }
}

pub fn tree(app: &mut App, ui: &mut Ui) {
    let t = app.theme.clone();
    let frame = Frame::new().fill(t.panel).inner_margin(Margin {
        left: 0,
        right: 0,
        top: 9,
        bottom: 8,
    });
    egui::Panel::left("tree")
        .exact_size(TREE_W)
        .frame(frame)
        .resizable(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                section_label(ui, app, "BODIES");
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(ui.available_height() - 80.0)
                .show(ui, |ui| {
                    crate::tree::show(ui, &t, &mut app.scene.bodies, &mut app.selected);
                });
            for s in ["SKETCHES", "DATUMS"] {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    section_label(ui, app, s);
                });
            }
        });
}

fn kv(ui: &mut Ui, app: &App, k: &str, v: &str) {
    ui.horizontal(|ui| {
        label(ui, k, 12.0, app.theme.ink5, FontFamily::Proportional);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            label(ui, v, 11.5, app.theme.ink3, FontFamily::Monospace);
        });
    });
    ui.add_space(6.0);
}

pub fn inspector(app: &mut App, ui: &mut Ui) {
    let t = app.theme.clone();
    let frame = Frame::new().fill(t.panel).inner_margin(Margin {
        left: 16,
        right: 16,
        top: 12,
        bottom: 16,
    });
    egui::Panel::right("inspector")
        .exact_size(INSPECTOR_W)
        .frame(frame)
        .resizable(false)
        .show(ui, |ui| {
            let Some(i) = app.selected else {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 20.0, t.well);
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(rect.center(), vec2(18.0, 18.0)),
                        "select",
                        t.ink6,
                        t.ink6,
                        1.7,
                    );
                    ui.add_space(8.0);
                    label(ui, "Nothing selected", 13.0, t.ink3, fonts::semibold());
                    ui.add_space(2.0);
                    let mut job = LayoutJob {
                        halign: Align::Center,
                        ..Default::default()
                    };
                    job.wrap.max_width = 200.0;
                    job.append(
                        "Select a body, sketch, face or edge to see its parameters and history.",
                        0.0,
                        TextFormat {
                            font_id: font(12.0, FontFamily::Proportional),
                            color: t.ink6,
                            line_height: Some(18.0),
                            ..Default::default()
                        },
                    );
                    // `px-2` inside the panel padding: the text wraps 16 px narrower.
                    let w = ui.available_width() - 16.0;
                    ui.allocate_ui_with_layout(
                        vec2(w, 40.0),
                        Layout::top_down(Align::Center),
                        |ui| ui.label(job),
                    );
                });
                return;
            };
            let b = &app.scene.bodies[i];
            label(ui, &b.label, 15.0, t.ink, fonts::semibold());
            label(
                ui,
                "Body · imported (STEP)",
                12.0,
                t.ink5,
                FontFamily::Proportional,
            );
            ui.add_space(14.0);
            section_label(ui, app, "PROPERTIES");
            ui.add_space(8.0);
            let m = &b.mesh;
            kv(ui, app, "Triangles", &format!("{}", m.triangle_count));
            kv(ui, app, "Vertices", &format!("{}", m.vertex_count));
            kv(ui, app, "Edges", &format!("{}", m.edge_count));
            let d: Vec<f32> = (0..3).map(|k| m.bbox_max[k] - m.bbox_min[k]).collect();
            kv(
                ui,
                app,
                "Size",
                &format!("{:.2} × {:.2} × {:.2}", d[0], d[1], d[2]),
            );
            kv(ui, app, "File", &b.file[..8.min(b.file.len())]);
            ui.add_space(10.0);
            section_label(ui, app, "PARAMETERS");
            ui.add_space(8.0);
            kv(
                ui,
                app,
                "Flange width",
                &format!("{:.2} mm", app.dim.value_mm),
            );
        });
}
