//! Spike app state: camera, selection, visibility, the dimension value with its undo stack.

use crate::camera::Camera;
use crate::mesh1::Body;
use crate::metrics::Metrics;
use glam::Vec3;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ScrollDevice {
    Wheel,
    Trackpad,
}

#[derive(Clone, Copy)]
pub enum DragMode {
    Orbit,
    Pan,
}

pub struct BodyInfo {
    pub label: String,
    pub file: String,
    pub visible: bool,
    pub tris: usize,
    pub verts: usize,
    pub faces: u32,
    pub edges: usize,
    pub min: Vec3,
    pub max: Vec3,
}

pub struct Bench {
    pub frame: u32,
    pub frames: u32,
    pub start_yaw: f32,
    pub exit_after: bool,
}

pub struct State {
    pub cam: Camera,
    pub home: Camera,
    pub bodies: Vec<BodyInfo>,
    pub selected: Option<usize>,
    pub scene_dirty: bool,
    pub bodies_dirty: bool,
    pub drag: Option<(DragMode, f32, f32)>,
    pub gesture: Option<(f32, f32, f32)>,
    pub scroll_device: ScrollDevice,
    pub last_raw_input: Option<Instant>,
    pub pending_input: Option<Instant>,
    pub inflight_input: Option<Instant>,
    pub dim_anchor: Vec3,
    pub dim_value: f64,
    pub dim_undo: Vec<f64>,
    pub dim_redo: Vec<f64>,
    pub metrics: Metrics,
    pub bench: Option<Bench>,
    pub t_main: Instant,
    pub first_frame_done: bool,
    pub exit_after_first_frame: bool,
}

impl State {
    pub fn new(bodies: &[Body], t_main: Instant) -> Self {
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let infos: Vec<BodyInfo> = bodies
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (bmin, bmax) = (Vec3::from(b.mesh.bbox_min), Vec3::from(b.mesh.bbox_max));
                min = min.min(bmin);
                max = max.max(bmax);
                BodyInfo {
                    label: format!("Body {}", i + 1),
                    file: b.file.clone(),
                    visible: true,
                    tris: b.mesh.indices.len() / 3,
                    verts: b.mesh.positions.len(),
                    faces: b.mesh.face_count,
                    edges: b.mesh.edge_ranges.len(),
                    min: bmin,
                    max: bmax,
                }
            })
            .collect();
        let cam = Camera::fit(min, max);
        // Anchor the dimension at the top of the model's bounding box above the centre (the
        // NEMA shaft tip); its value starts at the model height.
        let dim_anchor = Vec3::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, max.z);
        State {
            cam,
            home: cam,
            bodies: infos,
            selected: None,
            scene_dirty: true,
            bodies_dirty: true,
            drag: None,
            gesture: None,
            scroll_device: ScrollDevice::Wheel,
            last_raw_input: None,
            pending_input: None,
            inflight_input: None,
            dim_anchor,
            dim_value: f64::from(max.z - min.z),
            dim_undo: Vec::new(),
            dim_redo: Vec::new(),
            metrics: Metrics::default(),
            bench: None,
            t_main,
            first_frame_done: false,
            exit_after_first_frame: false,
        }
    }

    /// Marks the scene for re-render and attributes the frame to the latest raw input event.
    pub fn camera_changed(&mut self) {
        self.scene_dirty = true;
        if self.pending_input.is_none() {
            self.pending_input = self.last_raw_input;
        }
    }

    /// Re-frames the home view for the viewport aspect once it is known.
    pub fn fit_home(&mut self, aspect: f32) {
        let (min, max) = self.bounds(false);
        self.home = Camera::fit_aspect(min, max, aspect);
        self.cam = self.home;
    }

    fn bounds(&self, visible_only: bool) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        for b in self.bodies.iter().filter(|b| b.visible || !visible_only) {
            min = min.min(b.min);
            max = max.max(b.max);
        }
        (min, max)
    }

    pub fn fit_visible(&mut self, aspect: f32) {
        let (min, max) = self.bounds(true);
        if min.x <= max.x {
            let fitted = Camera::fit_aspect(min, max, aspect);
            self.cam.target = fitted.target;
            self.cam.distance = fitted.distance;
        }
    }

    /// Per-body GPU rows: linear fill rgb + flag (0 hidden, 1 visible, 2 selected).
    pub fn body_rows(&self, fill: [f32; 4]) -> Vec<[f32; 4]> {
        self.bodies
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let flag = if !b.visible {
                    0.0
                } else if self.selected == Some(i) {
                    2.0
                } else {
                    1.0
                };
                [fill[0], fill[1], fill[2], flag]
            })
            .collect()
    }

    /// World point on the Z=0 ground plane under logical pixel (`x`, `y`).
    pub fn ground_point(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Vec3> {
        let inv = self.cam.view_proj(w / h.max(1.0)).inverse();
        let ndc = glam::Vec2::new(x / w * 2.0 - 1.0, 1.0 - y / h * 2.0);
        // Reversed depth: 1 is the near plane; 0.5 is a finite point further along the ray.
        let near = inv.project_point3(ndc.extend(1.0));
        let far = inv.project_point3(ndc.extend(0.5));
        let dir = far - near;
        if dir.z.abs() < 1e-9 {
            return None;
        }
        let t = -near.z / dir.z;
        (t > 0.0).then(|| near + dir * t)
    }
}

pub fn format_mm(v: f64) -> String {
    format!("{v:.2} mm")
}
