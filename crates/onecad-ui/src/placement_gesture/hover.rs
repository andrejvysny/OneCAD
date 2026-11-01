//! Viewport side of the gesture: hover picking, click-to-place and the placement keys.
use super::{Knowledge, Target, reconcile};
use crate::{app::App, library_catalog::ClassifyTarget};
use onecad_interaction::selection_set::TopologyId;
use onecad_scene::{DVec2, DVec3, PickPolicy, ViewSnapshot};

fn array(v: DVec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

/// Called every frame the editor viewport is shown. Hover follows the pointer unless
/// the camera is moving or a widget owns it; a click places.
pub(crate) fn viewport(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    (view, display): (&ViewSnapshot, &crate::bridge::Display),
    overlay_owns: bool,
) {
    if !super::armed(app) {
        return;
    }
    let navigating = app.doc.navigation.active(ui.input(|input| input.time));
    let pointer = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|point| rect.contains(*point))
        .filter(|point| crate::viewport_widgets::allows_pointer(ui, rect, *point));
    let Some(position) = pointer.filter(|_| !overlay_owns && !navigating) else {
        return;
    };
    let logical = DVec2::new(
        f64::from(position.x - rect.min.x),
        f64::from(position.y - rect.min.y),
    );
    track(app, view, display, logical, ui.ctx());
    if response.clicked() {
        super::place(app, ui.input(|input| input.modifiers.shift));
    }
}

fn track(
    app: &mut App,
    view: &ViewSnapshot,
    display: &crate::bridge::Display,
    logical: DVec2,
    ctx: &egui::Context,
) {
    let Ok(ray) = view.ray(logical) else { return };
    let tolerance = crate::viewport_widgets::edge_tolerance(ctx);
    let own = app
        .doc
        .placement
        .as_ref()
        .and_then(|p| p.editing.as_ref())
        .map(|record| record.record_id);
    let skip = |body| own.is_some_and(|record| super::own_body(body, record));
    let hit = onecad_scene::pick_skipping(
        &display.scene,
        view,
        view.identity(),
        logical,
        (PickPolicy::FacesAndEdges, tolerance),
        &skip,
    )
    .ok()
    .flatten()
    .map(|hit| Target {
        body: hit.body_id,
        topology: hit.topology_id.to_owned(),
        point: array(hit.world_position),
    });
    let Some(mut p) = app.doc.placement.take() else {
        return;
    };
    p.hover.ray = Some((array(ray.origin), array(ray.direction)));
    let same = hit.as_ref().map(|t| (t.body, &t.topology))
        == p.hover.target.as_ref().map(|t| (t.body, &t.topology));
    if same {
        if let (Some(current), Some(next)) = (p.hover.target.as_mut(), hit) {
            current.point = next.point;
        }
    } else {
        p.hover.class = if hit.is_some() {
            Knowledge::Pending
        } else {
            Knowledge::None
        };
        p.hover.target = hit;
        p.attachment_cursor = 0;
    }
    pump_classify(app, &mut p);
    app.doc.placement = Some(p);
    reconcile::reconcile(app);
}

/// Asks about the current target unless a request is already on the wire; a target
/// passed over meanwhile is simply asked about next (latest only, one in flight).
pub(super) fn pump_classify(app: &mut App, p: &mut super::ComponentPlacement) {
    if p.hover.in_flight.is_some() || p.hover.class != Knowledge::Pending {
        return;
    }
    let Some(target) = p.hover.target.clone() else {
        return;
    };
    let Some(context) = app.doc.interaction.context.clone() else {
        return;
    };
    let (topo_key, element_id) = match crate::model_pick::topology_id(&target.topology) {
        TopologyId::Snapshot(key) => (Some(key.as_str().to_owned()), None),
        TopologyId::Element(id) => (None, Some(id.as_str().to_owned())),
    };
    let sequence = app.ui.library.catalog.request_classify(ClassifyTarget {
        body_id: onecad_app::worker::wire::body_id_wire(target.body),
        topo_key,
        element_id,
        point: target.point,
        fence: Some(onecad_app::dto::GeometryReadFenceDto {
            document_id: context.document_id.clone(),
            runtime_session: context.runtime_session.clone(),
            snapshot_id: context.snapshot_id.0,
        }),
    });
    p.hover.in_flight = Some(super::InFlight {
        sequence,
        key: (target.body, target.topology),
        context,
        since: std::time::Instant::now(),
    });
    crate::library_catalog::flush(app);
}

/// A flips the seat, Tab cycles compatible attachments, Enter places. Text fields
/// keep their keys.
pub(crate) fn keys(app: &mut App, raw: &mut egui::RawInput, text: bool) {
    if text || !super::armed(app) {
        return;
    }
    let (mut flip, mut cycle, mut enter) = (false, false, false);
    raw.events.retain(|event| {
        let egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } = event
        else {
            return true;
        };
        if !modifiers.is_none() {
            return true;
        }
        match key {
            egui::Key::A => flip = true,
            egui::Key::Tab => cycle = true,
            egui::Key::Enter => enter = true,
            _ => return true,
        }
        false
    });
    if let Some(p) = app.doc.placement.as_mut() {
        if flip && matches!(p.snap, super::snap::SnapIntent::Mate(_)) {
            p.flipped = !p.flipped;
        }
        if cycle && p.compatible > 1 {
            p.attachment_cursor += 1;
        }
    }
    reconcile::reconcile(app);
    if enter {
        super::place(app, false);
    }
}
