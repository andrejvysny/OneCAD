//! Camera-only navigation adapted from SolidCraft at
//! 6bde126a5cabd1f5af0c68e6955bd0176fab805a. Geometry stays Z-up.
use crate::theme::Theme;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};
use onecad_scene::{CameraBasis, DVec3, ViewSnapshot};

#[path = "cube_controls.rs"]
mod controls;
#[path = "cube_painter.rs"]
mod painter;
#[cfg(test)]
#[path = "cube_tests.rs"]
mod tests;
use painter::Facet;

const SIZE: f32 = crate::layout::CUBE;
const HALF: f64 = 40.0;
const INSET: f64 = 0.68;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CubeAction {
    SetDirection(DVec3),
    Orbit(egui::Vec2),
    QuarterOrbit { axis: DVec3, radians: f64 },
    Roll(f64),
    Home,
    Fit,
    ToggleProjection,
}

#[derive(Default)]
pub struct CubeResponse {
    pub actions: Vec<CubeAction>,
    pub owns_pointer: bool,
}

#[derive(Clone, Copy)]
struct CubePalette {
    accent: Color32,
    accent_soft: Color32,
    cube_face: Color32,
    cube_hover: Color32,
    cube_border: Color32,
    cube_text: Color32,
    triad: [Color32; 3],
}

impl CubePalette {
    fn new(theme: &Theme, active: bool) -> Self {
        Self {
            accent: theme.accent,
            accent_soft: theme.sel_bg,
            cube_face: if active { theme.surface } else { theme.panel },
            cube_hover: theme.sel_bg,
            cube_border: theme.border_strong,
            cube_text: theme.ink2,
            triad: [theme.axis_x, theme.axis_y, theme.axis_z],
        }
    }
}

#[derive(Clone, Copy)]
struct Gesture {
    origin: Pos2,
    last: Pos2,
    moved: bool,
    direction: DVec3,
}

pub fn rect(viewport_rect: Rect) -> Rect {
    let size = SIZE
        .min(viewport_rect.width())
        .min(viewport_rect.height())
        .max(0.0);
    Rect::from_min_size(
        pos2(viewport_rect.right() - size, viewport_rect.top()),
        vec2(size, size),
    )
}

fn project(basis: CameraBasis, point: DVec3, scale: f64) -> egui::Vec2 {
    vec2(
        (point.dot(basis.right) * scale) as f32,
        -(point.dot(basis.up) * scale) as f32,
    )
}

fn direction_label(d: DVec3) -> String {
    [
        (d.z, "BOTTOM", "TOP"),
        (d.y, "FRONT", "BACK"),
        (d.x, "LEFT", "RIGHT"),
    ]
    .into_iter()
    .filter_map(|(v, neg, pos)| {
        if v < 0.0 {
            Some(neg)
        } else if v > 0.0 {
            Some(pos)
        } else {
            None
        }
    })
    .collect::<Vec<_>>()
    .join(" · ")
}

fn facet(
    basis: CameraBasis,
    center: Pos2,
    half: f64,
    normal: DVec3,
    d: DVec3,
    mut vertices: Vec<DVec3>,
    axes: Option<(DVec3, DVec3)>,
) -> Facet {
    let middle = vertices.iter().copied().sum::<DVec3>() / vertices.len() as f64;
    vertices.sort_by(|a, b| {
        let a = project(basis, *a - middle, half);
        let b = project(basis, *b - middle, half);
        a.y.atan2(a.x).total_cmp(&b.y.atan2(b.x))
    });
    let outer_edges = vertices
        .iter()
        .zip(vertices.iter().cycle().skip(1))
        .take(vertices.len())
        .map(|(a, b)| {
            [(a.x, b.x), (a.y, b.y), (a.z, b.z)]
                .into_iter()
                .filter(|(a, b)| a.abs() == 1.0 && a == b)
                .count()
                >= 2
        })
        .collect();
    Facet {
        direction: d,
        centre: center + project(basis, middle, half),
        points: vertices
            .into_iter()
            .map(|v| center + project(basis, v, half))
            .collect(),
        outer_edges,
        label: direction_label(d),
        text_axes: axes.map(|(r, u)| (project(basis, r, 1.0), -project(basis, u, 1.0))),
        depth: normal.dot(basis.back),
    }
}

fn facets(basis: CameraBasis, center: Pos2, half: f64) -> Vec<Facet> {
    let mut out = Vec::new();
    for (n, r, u) in [
        (DVec3::X, DVec3::Y, DVec3::Z),
        (-DVec3::X, -DVec3::Y, DVec3::Z),
        (DVec3::Y, -DVec3::X, DVec3::Z),
        (-DVec3::Y, DVec3::X, DVec3::Z),
        (DVec3::Z, DVec3::X, DVec3::Y),
        (-DVec3::Z, DVec3::X, -DVec3::Y),
    ] {
        if n.dot(basis.back) <= 1e-5 {
            continue;
        }
        let bands = [
            (-1.0, -INSET, -1.0),
            (-INSET, INSET, 0.0),
            (INSET, 1.0, 1.0),
        ];
        for (x0, x1, sx) in bands {
            for (y0, y1, sy) in bands {
                let d = if sx != 0.0 && sy != 0.0 {
                    n + r * sx + u * sy
                } else {
                    n
                };
                let vertices = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .into_iter()
                    .map(|(x, y)| n + r * x + u * y)
                    .collect();
                let axes = (sx == 0.0 && sy == 0.0).then_some((r, u));
                out.push(facet(basis, center, half, n, d, vertices, axes));
            }
        }
    }
    out.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    out
}

fn contains(points: &[Pos2], p: Pos2) -> bool {
    if !p.is_finite() || points.len() < 3 {
        return false;
    }
    let mut sign = 0.0_f32;
    for (a, b) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        let edge = *b - *a;
        let offset = p - *a;
        let cross = edge.x * offset.y - edge.y * offset.x;
        if cross.abs() > 1e-3 {
            if sign != 0.0 && sign.signum() != cross.signum() {
                return false;
            }
            sign = cross;
        }
    }
    sign != 0.0
}

fn hit(faces: &[Facet], p: Pos2) -> Option<usize> {
    faces
        .iter()
        .enumerate()
        .rev()
        .find(|(_, f)| contains(&f.points, p))
        .map(|(i, _)| i)
}

fn update_gesture(
    state: &mut Gesture,
    position: Pos2,
    released: bool,
    actions: &mut Vec<CubeAction>,
) {
    let delta = position - state.last;
    state.moved |= position.distance_sq(state.origin) > 0.01;
    if delta != egui::Vec2::ZERO && state.moved {
        actions.push(CubeAction::Orbit(delta));
    }
    state.last = position;
    if released && !state.moved {
        actions.push(CubeAction::SetDirection(state.direction));
    }
}

fn store_gesture(ui: &egui::Ui, id: egui::Id, state: Option<Gesture>) {
    ui.data_mut(|d| {
        if let Some(s) = state {
            d.insert_temp(id, s);
        } else {
            d.remove::<Gesture>(id);
        }
    });
}

fn pointer_gesture(
    ui: &egui::Ui,
    id: egui::Id,
    faces: &[Facet],
    enabled: bool,
) -> (Vec<CubeAction>, bool) {
    let (pressed, released, down, position, focused) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.primary_down(),
            i.pointer.interact_pos(),
            i.focused,
        )
    });
    let mut state = ui.data(|d| d.get_temp::<Gesture>(id));
    if !focused || !enabled {
        ui.data_mut(|d| d.remove::<Gesture>(id));
        return (Vec::new(), state.is_some());
    }
    if pressed
        && let Some(p) = position
        && crate::viewport_widgets::allows_pointer(ui, ui.clip_rect(), p)
        && let Some(face) = hit(faces, p)
    {
        state = Some(Gesture {
            origin: p,
            last: p,
            moved: false,
            direction: faces[face].direction,
        });
    }
    if !down && !released {
        state = None;
    }
    let mut actions = Vec::new();
    let owned = state.is_some();
    if let Some(s) = state.as_mut()
        && let Some(p) = position
    {
        update_gesture(s, p, released, &mut actions);
    }
    if released || !down {
        state = None;
    }
    store_gesture(ui, id, state);
    (actions, owned)
}

fn face_widgets(
    ui: &mut egui::Ui,
    faces: &[Facet],
    hovered: Option<usize>,
    output: &mut CubeResponse,
) {
    for (_, face) in faces.iter().enumerate().filter(|(i, f)| {
        !faces[..*i]
            .iter()
            .any(|earlier| earlier.direction == f.direction)
    }) {
        let response = ui.interact(
            Rect::from_center_size(face.centre, vec2(18.0, 18.0)),
            ui.scope_id().with((
                "cube face",
                face.direction.x as i8,
                face.direction.y as i8,
                face.direction.z as i8,
            )),
            Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{} view", face.label))
        });
        // Pointer clicks use the raw gesture path, including corners and subthreshold drags.
        if response.clicked() && !response.clicked_by(egui::PointerButton::Primary) {
            output
                .actions
                .push(CubeAction::SetDirection(face.direction));
        }
    }
    if let Some(face) = hovered.and_then(|i| faces.get(i)) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let at = ui.input(|i| i.pointer.hover_pos()).unwrap_or(face.centre);
        ui.interact(
            Rect::from_center_size(at, vec2(2.0, 2.0)),
            ui.scope_id().with("cube tooltip"),
            Sense::hover(),
        )
        .on_hover_text(format!("{} view; drag to orbit", face.label));
    }
}

fn world_triad(
    painter: &egui::Painter,
    basis: CameraBasis,
    center: Pos2,
    scale: f32,
    palette: CubePalette,
) {
    let origin = center + vec2(-60.0, 52.0) * scale;
    for (axis, color, label) in [
        (DVec3::X, palette.triad[0], "X"),
        (DVec3::Y, palette.triad[1], "Y"),
        (DVec3::Z, palette.triad[2], "Z"),
    ] {
        let tip = origin + project(basis, axis, 17.0 * f64::from(scale));
        painter.line_segment([origin, tip], Stroke::new(1.5, color));
        painter.text(
            tip,
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(8.0),
            color,
        );
    }
}

/// Draw against the captured view; results never mutate application or document state.
pub fn show(
    ui: &mut egui::Ui,
    viewport_rect: Rect,
    view: &ViewSnapshot,
    theme: &Theme,
) -> CubeResponse {
    let outer = rect(viewport_rect);
    if !outer.intersect(ui.clip_rect()).is_positive() {
        return CubeResponse::default();
    }
    let scale = outer.width() / 164.0;
    let center = outer.center();
    let faces = facets(view.basis(), center, HALF * f64::from(scale));
    let over = ui
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|position| crate::viewport_widgets::allows_pointer(ui, outer, position));
    let enabled = crate::viewport_widgets::gesture_allowed(ui);
    let (actions, dragging) = pointer_gesture(
        ui,
        ui.scope_id().with("cube gesture"),
        &faces,
        enabled
            && (over
                || ui
                    .data(|d| d.get_temp::<Gesture>(ui.scope_id().with("cube gesture")))
                    .is_some()),
    );
    let mut output = CubeResponse {
        actions,
        owns_pointer: enabled && over || dragging,
    };
    let hovered = if enabled && !dragging {
        ui.input(|i| i.pointer.hover_pos())
            .filter(|_| over)
            .and_then(|p| hit(&faces, p))
    } else {
        None
    };
    let palette = CubePalette::new(theme, over || dragging);
    let painter = ui.painter_at(outer);
    painter::paint_cube(&painter, &faces, hovered, palette, true, scale);
    ui.add_enabled_ui(enabled, |ui| face_widgets(ui, &faces, hovered, &mut output));
    ui.add_enabled_ui(enabled, |ui| {
        controls::show(ui, &painter, center, scale, view, palette, &mut output)
    });
    world_triad(&painter, view.basis(), center, scale, palette);
    output
}
