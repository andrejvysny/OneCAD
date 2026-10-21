//! Adapted from SolidCraft 6bde126a5cabd1f5af0c68e6955bd0176fab805a.
//! Navigation artwork; captured projection and picking live in navigation_cube.
use super::CubePalette;
use egui::{Color32, FontId, Pos2, Rect, Shape, Stroke, pos2, vec2};
use onecad_scene::DVec3 as Vec3;
pub(super) struct Facet {
    pub(super) direction: Vec3,
    pub(super) points: Vec<Pos2>,
    pub(super) outer_edges: Vec<bool>,
    pub(super) centre: Pos2,
    pub(super) label: String,
    pub(super) text_axes: Option<(egui::Vec2, egui::Vec2)>,
    pub(super) depth: f64,
}
// Private marker for label meshes with atlas-pixel UVs. The output hook replaces
// it with the font texture before any renderer sees the frame.
pub(super) const LABEL_PIXELS: egui::TextureId =
    egui::TextureId::User(u64::from_le_bytes(*b"cubetext"));

struct LabelAtlas;

impl egui::plugin::Plugin for LabelAtlas {
    fn debug_name(&self) -> &'static str {
        "navigation cube label atlas"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        let size = ctx.fonts(|f| f.font_image_size());
        let normalize = vec2(1.0 / size[0].max(1) as f32, 1.0 / size[1].max(1) as f32);
        for shape in &mut output.shapes {
            if let Shape::Mesh(mesh) = &mut shape.shape
                && mesh.texture_id == LABEL_PIXELS
            {
                let mesh = std::sync::Arc::make_mut(mesh);
                mesh.texture_id = egui::TextureId::default();
                for v in &mut mesh.vertices {
                    v.uv = (v.uv.to_vec2() * normalize).to_pos2();
                }
            }
        }
    }
}

/// Warp the font mesh into the projected face, so labels turn with the model.
pub(super) fn face_text(painter: &egui::Painter, face: &Facet, ink: Color32, font_size: f32) {
    let Some((r, down)) = face.text_axes else {
        return;
    };
    let opacity = ((face.depth - 0.18) / 0.17).clamp(0.0, 1.0) as f32;
    if opacity <= 0.0 {
        return;
    }
    let ink = ink.gamma_multiply(opacity);
    let galley = painter.layout_no_wrap(face.label.clone(), FontId::proportional(font_size), ink);
    let centre = galley.rect.center();
    // Keep atlas-pixel UVs until the frame is complete: another label or widget
    // can grow the atlas after this mesh is created, invalidating early UVs.
    painter.ctx().add_plugin(LabelAtlas);
    let text = Shape::Text(egui::epaint::TextShape::new(Pos2::ZERO, galley, ink));
    let atlas = painter.ctx().fonts(|f| f.font_image_size());
    let primitives = painter.ctx().tessellate(
        vec![egui::epaint::ClippedShape {
            clip_rect: Rect::EVERYTHING,
            shape: text,
            transform_after_tessellation: Default::default(),
        }],
        painter.ctx().pixels_per_point(),
    );
    for primitive in primitives {
        if let egui::epaint::Primitive::Mesh(mut mesh) = primitive.primitive {
            mesh.texture_id = LABEL_PIXELS;
            for v in &mut mesh.vertices {
                v.uv = pos2(v.uv.x * atlas[0] as f32, v.uv.y * atlas[1] as f32);
                let local = v.pos - centre;
                v.pos = face.centre + r * local.x + down * (local.y * 1.35);
            }
            painter.add(Shape::mesh(mesh));
        }
    }
}

fn facet_fill(face: &Facet, highlighted: bool, t: CubePalette) -> Color32 {
    if highlighted {
        return t.cube_hover;
    }
    let base = t.cube_face;
    let shade = (0.94 + 0.06 * face.depth.clamp(0.0, 1.0)) as f32;
    Color32::from_rgb(
        (f32::from(base.r()) * shade) as u8,
        (f32::from(base.g()) * shade) as u8,
        (f32::from(base.b()) * shade) as u8,
    )
}

type CubeEdge = (Pos2, Pos2, usize, bool);

fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
    (a - o).x * (b - o).y - (a - o).y * (b - o).x
}

/// Monotone chain; interior lies where `cross(h[i], h[i + 1], p) > 0`.
fn convex_hull(points: &[Pos2]) -> Vec<Pos2> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| a.distance_sq(*b) < 1e-6);
    if pts.len() < 3 {
        return pts;
    }
    let mut hull: Vec<Pos2> = Vec::with_capacity(pts.len() * 2);
    for pass in [pts.clone(), pts.into_iter().rev().collect()] {
        let start = hull.len();
        for p in pass {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

fn round_corners(hull: &[Pos2], radius: f32) -> Vec<Pos2> {
    let n = hull.len();
    if n < 3 {
        return hull.to_vec();
    }
    let mut out = Vec::with_capacity(n * 5);
    for (i, &v) in hull.iter().enumerate() {
        let (prev, next) = (hull[(i + n - 1) % n], hull[(i + 1) % n]);
        let r = radius.min(0.45 * v.distance(prev).min(v.distance(next)));
        let a = v + (prev - v).normalized() * r;
        let b = v + (next - v).normalized() * r;
        for k in 0..=4u8 {
            let s = f32::from(k) / 4.0;
            let w = 1.0 - s;
            out.push(pos2(
                w * w * a.x + 2.0 * w * s * v.x + s * s * b.x,
                w * w * a.y + 2.0 * w * s * v.y + s * s * b.y,
            ));
        }
    }
    out
}

/// Sutherland–Hodgman against a convex polygon oriented like `convex_hull`.
fn clip_convex(subject: &[Pos2], clip: &[Pos2]) -> Vec<Pos2> {
    let mut out = subject.to_vec();
    for (i, &a) in clip.iter().enumerate() {
        let b = clip[(i + 1) % clip.len()];
        let input = std::mem::take(&mut out);
        let Some(&last) = input.last() else {
            break;
        };
        let mut s = last;
        for &e in &input {
            let (ds, de) = (cross(a, b, s), cross(a, b, e));
            if (ds >= 0.0) != (de >= 0.0) {
                out.push(s + (e - s) * (ds / (ds - de)));
            }
            if de >= 0.0 {
                out.push(e);
            }
            s = e;
        }
    }
    out
}

fn clip_segment(a: Pos2, b: Pos2, clip: &[Pos2]) -> Option<(Pos2, Pos2)> {
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (i, &p) in clip.iter().enumerate() {
        let q = clip[(i + 1) % clip.len()];
        let (da, db) = (cross(p, q, a), cross(p, q, b));
        if da < 0.0 && db < 0.0 {
            return None;
        }
        if da < 0.0 {
            t0 = t0.max(da / (da - db));
        } else if db < 0.0 {
            t1 = t1.min(da / (da - db));
        }
    }
    (t0 < t1).then(|| (a + (b - a) * t0, a + (b - a) * t1))
}

fn append_face(
    mesh: &mut egui::Mesh,
    edges: &mut Vec<CubeEdge>,
    face: &Facet,
    highlighted: bool,
    t: CubePalette,
    clip: &[Pos2],
) {
    let fill = facet_fill(face, highlighted, t);
    let clipped = clip_convex(&face.points, clip);
    if clipped.len() >= 3 {
        let base = mesh.vertices.len() as u32;
        for &p in &clipped {
            mesh.colored_vertex(p, fill);
        }
        for j in 1..clipped.len() as u32 - 1 {
            mesh.add_triangle(base, base + j, base + j + 1);
        }
    }
    for (j, (&a, &b)) in face
        .points
        .iter()
        .zip(face.points.iter().cycle().skip(1))
        .take(face.points.len())
        .enumerate()
    {
        // Picking subdivisions have no visible seams; merge their shared boundaries.
        let outer = face.outer_edges.get(j).copied().unwrap_or(false);
        if !highlighted && !outer {
            continue;
        }
        if let Some(edge) = edges.iter_mut().find(|(p, q, _, _)| {
            (p.distance_sq(a) < 1e-6 && q.distance_sq(b) < 1e-6)
                || (p.distance_sq(b) < 1e-6 && q.distance_sq(a) < 1e-6)
        }) {
            edge.2 += usize::from(highlighted);
            edge.3 |= outer;
        } else {
            edges.push((a, b, usize::from(highlighted), outer));
        }
    }
}

pub(super) fn paint_cube(
    painter: &egui::Painter,
    faces: &[Facet],
    hovered: Option<usize>,
    t: CubePalette,
    labels: bool,
    scale: f32,
) {
    // One opaque mesh avoids antialiasing fringes between coplanar picking regions.
    let mut fill_mesh = egui::Mesh::default();
    let mut edges = Vec::new();
    let direction = hovered.and_then(|i| faces.get(i)).map(|f| f.direction);
    let all: Vec<Pos2> = faces
        .iter()
        .flat_map(|f| f.points.iter().copied())
        .collect();
    let silhouette = round_corners(&convex_hull(&all), 5.0 * scale);
    for face in faces {
        append_face(
            &mut fill_mesh,
            &mut edges,
            face,
            direction == Some(face.direction),
            t,
            &silhouette,
        );
    }
    painter.add(Shape::mesh(fill_mesh));
    if silhouette.len() >= 3 {
        painter.add(Shape::closed_line(
            silhouette.clone(),
            Stroke::new(1.0, t.cube_border),
        ));
    }
    for (a, b, highlights, outer) in edges {
        // Hover outlines the face, including its silhouette side; seams between its picking
        // regions (highlighted twice) stay hidden.
        if highlights == 0 || (!outer && highlights != 1) {
            continue;
        }
        if let Some((a, b)) = clip_segment(a, b, &silhouette) {
            painter.line_segment([a, b], Stroke::new(1.0, t.accent));
        }
    }
    if labels {
        for face in faces {
            face_text(painter, face, t.cube_text, 14.0 * scale);
        }
    }
}
pub(super) fn arrow(
    painter: &egui::Painter,
    at: Pos2,
    direction: egui::Vec2,
    hover: bool,
    t: CubePalette,
) {
    let side = vec2(-direction.y, direction.x);
    painter.add(Shape::convex_polygon(
        vec![
            at + direction * 7.0,
            at - direction * 5.0 + side * 7.0,
            at - direction * 5.0 - side * 7.0,
        ],
        if hover { t.accent_soft } else { t.cube_face },
        Stroke::new(1.0, if hover { t.accent } else { t.cube_border }),
    ));
}
pub(super) fn curved_arrow(
    painter: &egui::Painter,
    at: Pos2,
    clockwise: bool,
    hover: bool,
    t: CubePalette,
    scale: f32,
) {
    let (start, end) = if clockwise {
        (-std::f32::consts::FRAC_PI_4 + 0.12, 0.0)
    } else {
        (
            -std::f32::consts::FRAC_PI_4 - 0.12,
            -std::f32::consts::FRAC_PI_2,
        )
    };
    let points: Vec<_> = (0..=16)
        .map(|i| {
            let angle = egui::lerp(start..=end, f32::from(i as u8) / 16.0);
            at + vec2(angle.cos(), angle.sin()) * (32.0 * scale)
        })
        .collect();
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(5.0 * scale, if hover { t.accent } else { t.cube_border }),
    ));
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(3.0 * scale, if hover { t.accent_soft } else { t.cube_face }),
    ));
    if let Some(tip) = points.last() {
        let sign = if clockwise { 1.0 } else { -1.0 };
        arrow(
            painter,
            *tip,
            vec2(-end.sin(), end.cos()) * (sign * scale),
            hover,
            t,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(points: &[Pos2]) -> f32 {
        let n = points.len();
        (0..n)
            .map(|i| cross(Pos2::ZERO, points[i], points[(i + 1) % n]))
            .sum::<f32>()
            .abs()
            / 2.0
    }

    #[test]
    fn rounded_hull_trims_square_corners() {
        let square = [
            pos2(0.0, 0.0),
            pos2(100.0, 0.0),
            pos2(100.0, 100.0),
            pos2(0.0, 100.0),
        ];
        let hull = convex_hull(&square);
        assert_eq!(hull.len(), 4);
        let rounded = round_corners(&hull, 5.0);
        let clipped = clip_convex(&square, &rounded);
        assert!(clipped.len() >= 16);
        let a = area(&clipped);
        assert!(a < 10_000.0 && a > 9_950.0, "area {a}");
        for p in &clipped {
            assert!((-1e-3..=100.001).contains(&p.x) && (-1e-3..=100.001).contains(&p.y));
            assert!(p.distance(pos2(0.0, 0.0)) > 1.0);
        }
        assert!(clip_segment(pos2(-10.0, 50.0), pos2(110.0, 50.0), &rounded).is_some());
        assert!(clip_segment(pos2(-10.0, -1.0), pos2(-1.0, -10.0), &rounded).is_none());
    }
}
