//! Per-session snap memory: endpoints acquired by dwelling and snaps discarded with Esc.
use super::CachedSnap;
use crate::app::App;
use egui::{Rect, Stroke, Ui};
use onecad_interaction::sketch::{
    SketchSession, SketchSessionKey, SketchTarget, SketchTool,
    inference::{AcceptedSnap, SnapIntent, SnapKind},
};
use onecad_scene::{DVec3, ViewSnapshot};

const DWELL_SECONDS: f64 = 0.3;
const MAX_ACQUIRED: usize = 4;
const MAX_SUPPRESSED: usize = 8;

fn acquired_id() -> egui::Id {
    crate::documents::scoped_id("sketch-acquired-endpoints")
}

fn suppressed_id() -> egui::Id {
    crate::documents::scoped_id("sketch-suppressed-snaps")
}

#[derive(Clone, Default)]
pub(super) struct Acquired {
    key: Option<SketchSessionKey>,
    tool: Option<SketchTool>,
    hover: Option<(SketchTarget, f64)>,
    pub(super) list: Vec<SketchTarget>,
}

#[derive(Clone)]
pub(super) struct Suppressed {
    key: SketchSessionKey,
    tool: SketchTool,
    draft_generation: u64,
    pub(super) list: Vec<AcceptedSnap>,
}

impl Suppressed {
    fn current(&self, session: &SketchSession) -> bool {
        self.key == session.key
            && self.tool == session.tool
            && self.draft_generation == session.draft_generation
    }

    fn empty_for(session: &SketchSession) -> Self {
        Self {
            key: session.key.clone(),
            tool: session.tool,
            draft_generation: session.draft_generation,
            list: Vec::new(),
        }
    }
}

/// Returns seconds until a repaint must re-evaluate the dwell.
pub(super) fn acquire(
    state: &mut Acquired,
    winner: Option<&AcceptedSnap>,
    now: f64,
) -> Option<f64> {
    let target = winner.and_then(|snap| match (snap.kind, snap.intent) {
        (SnapKind::Endpoint, SnapIntent::Coincident(target)) => Some(target),
        _ => None,
    });
    let Some(target) = target else {
        state.hover = None;
        return None;
    };
    match state.hover {
        Some((hovered, start)) if hovered == target => {
            let waited = now - start;
            if waited < DWELL_SECONDS {
                return Some(DWELL_SECONDS - waited);
            }
            state.list.retain(|existing| *existing != target);
            state.list.insert(0, target);
            state.list.truncate(MAX_ACQUIRED);
            None
        }
        _ => {
            state.hover = Some((target, now));
            Some(DWELL_SECONDS)
        }
    }
}

pub(super) fn load_acquired(ctx: &egui::Context, session: &SketchSession) -> Acquired {
    let mut state = ctx
        .data(|data| data.get_temp::<Acquired>(acquired_id()))
        .filter(|state| {
            state.key.as_ref() == Some(&session.key) && state.tool == Some(session.tool)
        })
        .unwrap_or_else(|| Acquired {
            key: Some(session.key.clone()),
            tool: Some(session.tool),
            ..Default::default()
        });
    state
        .list
        .retain(|target| target.point_coordinates(&session.authoritative).is_some());
    state
}

pub(super) fn store_acquired(ctx: &egui::Context, state: Acquired) {
    ctx.data_mut(|data| data.insert_temp(acquired_id(), state));
}

/// A dwell that outlives the pointer must not count when the pointer returns.
pub(super) fn reset_hover(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Acquired>(acquired_id())
            .hover = None
    });
}

pub(super) fn load_suppressed(ctx: &egui::Context, session: &SketchSession) -> Suppressed {
    ctx.data(|data| data.get_temp::<Suppressed>(suppressed_id()))
        .filter(|state| state.current(session))
        .unwrap_or_else(|| Suppressed::empty_for(session))
}

pub(super) fn store_suppressed(ctx: &egui::Context, state: Suppressed) {
    ctx.data_mut(|data| {
        if state.list.is_empty() {
            data.remove::<Suppressed>(suppressed_id());
        } else {
            data.insert_temp(suppressed_id(), state);
        }
    });
}

pub(crate) fn discard_snap(ctx: &egui::Context, app: &App) -> bool {
    app.doc
        .sketch
        .session
        .as_ref()
        .is_some_and(|session| discard(ctx, session))
}

pub(super) fn discard(ctx: &egui::Context, session: &SketchSession) -> bool {
    let cache = crate::documents::scoped_id("sketch-winning-snap");
    let Some(cached) = ctx
        .data(|data| data.get_temp::<CachedSnap>(cache))
        .filter(|cached| cached.current(session))
    else {
        return false;
    };
    let mut state = load_suppressed(ctx, session);
    if !state.list.contains(&cached.snap.snap) {
        state.list.push(cached.snap.snap);
        if state.list.len() > MAX_SUPPRESSED {
            state.list.remove(0);
        }
    }
    store_suppressed(ctx, state);
    ctx.data_mut(|data| data.remove::<CachedSnap>(cache));
    ctx.request_repaint();
    true
}

pub(super) fn paint_acquired(app: &App, ui: &Ui, rect: Rect, view: &ViewSnapshot) {
    let Some(session) = app.doc.sketch.session.as_ref() else {
        return;
    };
    let state = load_acquired(ui.ctx(), session);
    let painter = ui.painter().with_clip_rect(rect);
    let stroke = Stroke::new(1.25, app.theme.sketch_inference);
    for target in &state.list {
        let Some(point) = target.point_coordinates(&session.authoritative) else {
            continue;
        };
        let world = session.plane.to_world(point);
        let Some(screen) = view.project(DVec3::new(world.x, world.y, world.z)) else {
            continue;
        };
        let at = rect.min + egui::vec2(screen.logical.x as f32, screen.logical.y as f32);
        if !at.is_finite() {
            continue;
        }
        painter.line_segment(
            [at - egui::vec2(4.0, 0.0), at + egui::vec2(4.0, 0.0)],
            stroke,
        );
        painter.line_segment(
            [at - egui::vec2(0.0, 4.0), at + egui::vec2(0.0, 4.0)],
            stroke,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch_snapping::ResolvedSnap;
    use onecad_core::{
        ids::{EntityId, SketchId, SnapshotId},
        math::Vec2,
        sketch::{CurvePosition, Sketch, WorldPlane},
    };
    use onecad_interaction::{
        DocumentContext,
        sketch::{SketchAction, SketchInteraction, inference::SnapSource},
    };
    use onecad_scene::{Camera, SceneIdentity, Viewport};

    fn endpoint(entity: EntityId) -> SketchTarget {
        SketchTarget::Point {
            entity,
            role: CurvePosition::Start,
        }
    }

    fn source() -> SnapSource {
        let view = ViewSnapshot::new(
            SceneIdentity {
                session: 1,
                scene: 1,
            },
            1,
            Camera::default(),
            Viewport {
                width: 800.0,
                height: 600.0,
                scale_factor: 1.0,
            },
        )
        .unwrap();
        let sketch = Sketch::on_world_plane(SketchId::new(), "S", WorldPlane::XY);
        onecad_interaction::sketch::inference::source_for(&sketch, view.identity(), 0)
    }

    fn snap(target: SketchTarget) -> AcceptedSnap {
        AcceptedSnap {
            source: source(),
            kind: SnapKind::Endpoint,
            point: Vec2::new(0.0, 0.0).unwrap(),
            intent: SnapIntent::Coincident(target),
        }
    }

    fn session() -> SketchSession {
        let context = DocumentContext {
            document_id: "doc".into(),
            runtime_session: "rt".into(),
            revision: 1,
            snapshot_id: SnapshotId(1),
            scene: SceneIdentity {
                session: 1,
                scene: 1,
            },
        };
        let mut interaction = SketchInteraction::default();
        interaction.dispatch(SketchAction::ContextChanged(Some(context.clone())));
        interaction.dispatch(SketchAction::Begin {
            sketch: Box::new(Sketch::on_world_plane(
                SketchId(uuid::Uuid::from_u128(1)),
                "Sketch",
                WorldPlane::XY,
            )),
            create: false,
            context,
        });
        interaction.session.unwrap()
    }

    fn cached(session: &SketchSession, target: SketchTarget) -> CachedSnap {
        CachedSnap {
            snap: ResolvedSnap {
                snap: snap(target),
                guides: [None, None],
            },
            key: session.key.clone(),
            tool: session.tool,
            draft_generation: session.draft_generation,
        }
    }

    #[test]
    fn dwell_under_threshold_does_not_acquire() {
        let target = endpoint(EntityId::new());
        let mut state = Acquired::default();
        let hit = snap(target);
        assert_eq!(acquire(&mut state, Some(&hit), 1.0), Some(DWELL_SECONDS));
        let wait = acquire(&mut state, Some(&hit), 1.2).unwrap();
        assert!((wait - 0.1).abs() < 1e-9);
        assert!(state.list.is_empty());
        assert_eq!(acquire(&mut state, Some(&hit), 1.3), None);
        assert_eq!(state.list, vec![target]);
    }

    #[test]
    fn five_targets_keep_newest_four_and_rehover_moves_to_front() {
        let targets: Vec<_> = (0..5).map(|_| endpoint(EntityId::new())).collect();
        let mut state = Acquired::default();
        let mut now = 0.0;
        for target in &targets {
            let hit = snap(*target);
            acquire(&mut state, Some(&hit), now);
            now += 0.4;
            acquire(&mut state, Some(&hit), now);
            now += 0.1;
        }
        assert_eq!(
            state.list,
            vec![targets[4], targets[3], targets[2], targets[1]]
        );
        let hit = snap(targets[2]);
        acquire(&mut state, Some(&hit), now);
        acquire(&mut state, Some(&hit), now + 0.4);
        assert_eq!(
            state.list,
            vec![targets[2], targets[4], targets[3], targets[1]]
        );
        assert_eq!(acquire(&mut state, None, now + 1.0), None);
        assert!(state.hover.is_none());
    }

    #[test]
    fn session_or_tool_change_resets_acquisition() {
        let ctx = egui::Context::default();
        let session = session();
        let mut state = load_acquired(&ctx, &session);
        state.list.push(endpoint(EntityId::new()));
        store_acquired(&ctx, state);
        let mut other = session.clone();
        other.tool = if session.tool == SketchTool::Line {
            SketchTool::Circle
        } else {
            SketchTool::Line
        };
        assert!(load_acquired(&ctx, &other).list.is_empty());
    }

    #[test]
    fn discard_needs_a_current_cached_snap_and_consumes_it() {
        let ctx = egui::Context::default();
        let session = session();
        let cache = crate::documents::scoped_id("sketch-winning-snap");
        assert!(!discard(&ctx, &session));
        let target = endpoint(EntityId::new());
        let cached = cached(&session, target);
        let hit = cached.snap.snap;
        ctx.data_mut(|data| data.insert_temp(cache, cached));
        assert!(discard(&ctx, &session));
        assert_eq!(load_suppressed(&ctx, &session).list, vec![hit]);
        assert!(!discard(&ctx, &session));
    }
}
