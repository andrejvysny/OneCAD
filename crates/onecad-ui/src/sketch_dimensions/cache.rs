//! Placement is transient presentation state, bounded by one sketch visit and 128 labels.
use egui::{Pos2, Rect, Vec2};
use onecad_core::ids::ConstraintId;
use onecad_interaction::{
    DocumentContext,
    sketch::{SketchSession, SketchSessionKey},
};
use onecad_scene::CaptureIdentity;

pub const MAX_BADGES: usize = 128;
#[derive(Clone)]
pub struct Placement {
    pub id: ConstraintId,
    pub offset: Vec2,
    pub hidden: bool,
    pub last_rect: Option<Rect>,
}
#[derive(Clone)]
pub struct Drag {
    pub id: ConstraintId,
    pub document: DocumentContext,
    pub capture: CaptureIdentity,
    pub origin: Vec2,
    pub pointer: Pos2,
}
#[derive(Clone)]
pub struct Cache {
    pub key: SketchSessionKey,
    pub document: DocumentContext,
    pub capture: CaptureIdentity,
    pub entries: Vec<Placement>,
    pub drag: Option<Drag>,
}

impl Cache {
    pub fn sync(previous: Option<Self>, session: &SketchSession, capture: CaptureIdentity) -> Self {
        let mut cache = previous
            .filter(|cache| cache.key == session.key)
            .unwrap_or_else(|| Self {
                key: session.key.clone(),
                document: session.document.clone(),
                capture,
                entries: Vec::new(),
                drag: None,
            });
        cache.entries.retain(|entry| {
            session
                .authoritative
                .get_constraint(entry.id)
                .is_some_and(|constraint| constraint.value().is_some())
        });
        for constraint in session
            .authoritative
            .constraints()
            .iter()
            .filter(|constraint| constraint.value().is_some())
            .take(MAX_BADGES)
        {
            if !cache
                .entries
                .iter()
                .any(|entry| entry.id == constraint.id())
                && cache.entries.len() < MAX_BADGES
            {
                cache.entries.push(Placement {
                    id: constraint.id(),
                    offset: egui::vec2(0.0, -26.0),
                    hidden: false,
                    last_rect: None,
                });
            }
        }
        if cache.document != session.document || cache.capture != capture {
            cache.drag = None;
            for entry in &mut cache.entries {
                entry.last_rect = None;
            }
        }
        cache.document = session.document.clone();
        cache.capture = capture;
        cache
    }
    pub fn current(&self, session: &SketchSession, capture: CaptureIdentity) -> bool {
        self.key == session.key && self.document == session.document && self.capture == capture
    }
}

pub fn cache_id() -> egui::Id {
    egui::Id::unique("sketch-dimension-placement")
}

pub fn place(
    anchor: Pos2,
    size: Vec2,
    offset: Vec2,
    viewport: Rect,
    occupied: &[Rect],
) -> Option<Rect> {
    let safe = viewport.shrink(6.0);
    if !anchor.is_finite()
        || !size.is_finite()
        || !offset.is_finite()
        || !safe.contains(anchor)
        || size.x > safe.width()
        || size.y > safe.height()
    {
        return None;
    }
    for index in 0..41 {
        let ring = ((index + 7) / 8) as f32;
        let delta = if index == 0 {
            Vec2::ZERO
        } else {
            let theta = (index - 1) as f32 * std::f32::consts::FRAC_PI_4;
            egui::vec2(theta.cos() * 32.0 * ring, theta.sin() * 26.0 * ring)
        };
        let center = anchor + offset + delta;
        let center = egui::pos2(
            center
                .x
                .clamp(safe.left() + size.x * 0.5, safe.right() - size.x * 0.5),
            center
                .y
                .clamp(safe.top() + size.y * 0.5, safe.bottom() - size.y * 0.5),
        );
        let candidate = Rect::from_center_size(center, size);
        if !occupied
            .iter()
            .any(|other| candidate.intersects(other.expand(3.0)))
        {
            return Some(candidate);
        }
    }
    None
}
