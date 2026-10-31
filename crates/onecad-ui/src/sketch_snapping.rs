//! Snaps are measured in the accepted viewport's logical pixels.
use egui::{Pos2, Rect};
use onecad_core::{
    math::Vec2,
    sketch::{Sketch, SketchPlane},
};
use onecad_interaction::sketch::{
    SketchTarget,
    inference::{AcceptedSnap, SnapSource},
};
use onecad_scene::ViewSnapshot;
mod candidates;
mod curves;
use candidates::{Candidate, collect_candidates};
pub use guide_types::{GuideKind, GuideShape, ResolvedSnap, SnapGuide};
mod guide_types;
#[cfg(test)]
mod tests;

pub const MAX_SNAP_ENTITIES: usize = 1024;
pub const MAX_SNAP_CANDIDATES: usize = 512;

#[derive(Clone, Copy, Debug)]
pub struct SnapSettings {
    pub tolerance: f64,
    pub hysteresis: f64,
    pub grid_step: Option<f64>,
    pub polar_step_degrees: Option<f64>,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            tolerance: 8.0,
            hysteresis: 3.0,
            grid_step: None,
            polar_step_degrees: Some(15.0),
        }
    }
}

pub struct SnapRequest<'a> {
    pub sketch: &'a Sketch,
    pub plane: SketchPlane,
    pub view: &'a ViewSnapshot,
    pub rect: Rect,
    pub pointer: Pos2,
    pub anchor: Option<(Vec2, Option<SketchTarget>)>,
    pub previous: Option<&'a AcceptedSnap>,
    pub generation: u64,
    pub settings: SnapSettings,
    /// Acquired endpoints whose line or arc may be followed past its end.
    pub extensions: &'a [SketchTarget],
    /// Snaps the user discarded with Esc; they stay hidden while still in reach.
    pub suppressed: &'a [AcceptedSnap],
}

#[derive(Clone, Debug, Default)]
pub struct Resolution {
    pub winner: Option<ResolvedSnap>,
    /// Discarded snaps that are still within reach this frame; the rest are released.
    pub suppressed_present: Vec<AcceptedSnap>,
}

pub fn snap_source(sketch: &Sketch, view: &ViewSnapshot, generation: u64) -> SnapSource {
    onecad_interaction::sketch::inference::source_for(sketch, view.identity(), generation)
}

#[cfg(test)]
pub fn resolve_snap(request: &SnapRequest<'_>) -> Option<AcceptedSnap> {
    resolve(request).winner.map(|winner| winner.snap)
}

pub fn resolve(request: &SnapRequest<'_>) -> Resolution {
    if request.sketch.entities().len() > MAX_SNAP_ENTITIES
        || request.plane != request.sketch.effective_plane()
        || !request.rect.contains(request.pointer)
        || !request.settings.tolerance.is_finite()
        || !(0.0..=32.0).contains(&request.settings.tolerance)
        || !request.settings.hysteresis.is_finite()
        || !(0.0..=16.0).contains(&request.settings.hysteresis)
        || (f64::from(request.rect.width()) - request.view.viewport().width).abs() > 0.001
        || (f64::from(request.rect.height()) - request.view.viewport().height).abs() > 0.001
    {
        return Resolution::default();
    }
    let source = snap_source(request.sketch, request.view, request.generation);
    let Some(mut candidates) = collect_candidates(request, source) else {
        return Resolution::default();
    };
    let suppressed = |candidate: &Candidate| {
        request
            .suppressed
            .iter()
            .any(|snap| candidate.same_target(snap))
    };
    let suppressed_present = request
        .suppressed
        .iter()
        .filter(|snap| {
            candidates
                .iter()
                .any(|candidate| candidate.same_target(snap))
        })
        .copied()
        .collect();
    candidates.retain(|candidate| !suppressed(candidate));
    Resolution {
        winner: arbitrate(request, &candidates, source).map(|winner| ResolvedSnap {
            snap: winner.snap,
            guides: winner.guides,
        }),
        suppressed_present,
    }
}

fn arbitrate<'c>(
    request: &SnapRequest<'_>,
    candidates: &'c [Candidate],
    source: SnapSource,
) -> Option<&'c Candidate> {
    let best = candidates
        .iter()
        .filter(|c| c.distance <= request.settings.tolerance)
        .min_by(|a, b| ranked(a, b, a.score(), b.score(), request.sketch));
    if let Some(previous) = request
        .previous
        .filter(|previous| previous.source == source)
        && let Some(retained) = candidates
            .iter()
            .filter(|candidate| candidate.same_target(previous))
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
        && best.is_none_or(|best| {
            !best.refines(retained)
                && retained.score() <= best.score() + request.settings.hysteresis
        })
    {
        return Some(retained);
    }
    best
}

fn ranked(
    a: &Candidate,
    b: &Candidate,
    a_value: f64,
    b_value: f64,
    sketch: &Sketch,
) -> std::cmp::Ordering {
    if (a_value - b_value).abs() < 1e-8 {
        a.tiebreak(b, sketch)
    } else {
        a_value.total_cmp(&b_value)
    }
}

fn project(request: &SnapRequest<'_>, point: Vec2) -> Option<Pos2> {
    let point = request.plane.to_world(point);
    let screen = request
        .view
        .project(onecad_scene::DVec3::new(point.x, point.y, point.z))?;
    Some(request.rect.min + egui::vec2(screen.logical.x as f32, screen.logical.y as f32))
}
