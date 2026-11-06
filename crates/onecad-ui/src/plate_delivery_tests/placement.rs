//! Native component placement through the production host with the real worker and
//! Metal: arm from the library detail page, hover a hole rim, place, Copy, cancel and
//! repeat. Synthetic pointer delivery; not physical input acceptance.
use super::*;
use crate::{
    placement_gesture::{Phase, snap::SnapIntent},
    ui_state::StartPage,
};
use onecad_core::document::record::{ComponentParamValue, MateKind};
use onecad_scene::{Camera, Projection};

const SCREEN: egui::Vec2 = egui::vec2(1280.0, 820.0);

pub(super) fn frame(host: &mut Harness, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers))
        .chain(events)
        .collect();
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
        events,
        focused: true,
        ..Default::default()
    };
    crate::input::route(&mut host.app, &host.context, &mut raw);
    let mut output = host.context.run_ui(raw, |ui| {
        eframe::App::ui(&mut host.app, ui, &mut host.frame)
    });
    output.textures_delta.clear();
    logic_tick(&mut host.app, &host.context, &mut host.frame, false);
}

pub(super) async fn until(host: &mut Harness, what: &str, done: impl Fn(&App) -> bool) {
    let waited = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while !done(&host.app) {
            frame(host, vec![], egui::Modifiers::NONE);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    let state = host.app.doc.placement.as_ref().map(|p| {
        format!(
            "hover={:?} snap={:?} notice={:?} sent={:?}",
            p.hover, p.snap, p.notice, p.sent
        )
    });
    assert!(
        waited.is_ok(),
        "timed out waiting for {what}: {} / status={:?} tool={:?} / {state:?} / catalog={:?}",
        host.app.doc.status,
        host.app.doc.interaction.status,
        host.app
            .doc
            .interaction
            .tool
            .as_ref()
            .map(|t| (t.phase, t.preview.clone())),
        host.app
            .ui
            .library
            .catalog
            .detail()
            .map(|view| &view.status)
    );
}

/// A hovered pointer position persists across frames until it moves.
fn hover(host: &mut Harness, position: egui::Pos2) {
    frame(
        host,
        vec![egui::Event::PointerMoved(position)],
        egui::Modifiers::NONE,
    );
}

pub(super) fn click(host: &mut Harness, position: egui::Pos2, modifiers: egui::Modifiers) {
    let button = |pressed| egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    };
    frame(
        host,
        vec![egui::Event::PointerMoved(position), button(true)],
        modifiers,
    );
    frame(host, vec![button(false)], modifiers);
}

pub(super) fn rim_position(host: &Harness, rim: DVec3) -> egui::Pos2 {
    let view = host.app.doc.view.unwrap();
    let viewport = crate::layout::chrome(
        egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN),
        &host.app.ui,
    )
    .viewport;
    let logical = view.project(rim).unwrap().logical;
    viewport.min + egui::vec2(logical.x as f32, logical.y as f32)
}

/// A 4 mm plate with one hole; returns a world point on the hole's top rim.
pub(super) async fn plate_with_hole(host: &mut Harness) -> DVec3 {
    plate_with_holes(host, 1).await[0]
}

/// The same plate with `holes` (1 or 2) holes; a top-rim point of each.
async fn plate_with_holes(host: &mut Harness, holes: usize) -> Vec<DVec3> {
    host.primitive(SketchTool::Rectangle, [0.0, 0.0], [80.0, 50.0])
        .await;
    let mut rims = Vec::new();
    for centre in [15.0, 65.0].into_iter().take(holes) {
        host.primitive(SketchTool::Circle, [centre, 25.0], [centre + 3.0, 25.0])
            .await;
        let plane = host.sketch().effective_plane();
        let on_rim = plane.to_world(Vec2::new(centre + 3.0, 25.0).unwrap());
        rims.push(DVec3::new(
            on_rim.x + 4.0 * plane.normal.x,
            on_rim.y + 4.0 * plane.normal.y,
            on_rim.z + 4.0 * plane.normal.z,
        ));
    }
    host.finish().await;
    extrude_holes(host, holes).await;
    host.app.ui.start_open = false;
    host.app.ui.explorer_open = false;
    host.app.ui.inspector_open = false;
    host.app.doc.fit_pending = false;
    host.app.doc.camera = Camera {
        target: DVec3::new(-25.0, 40.0, 4.0),
        offset: DVec3::new(0.0, 0.0, 120.0),
        projection: Projection::Orthographic,
        ..Default::default()
    };
    host.app.draw_dirty = true;
    until(host, "the plate view", |app| {
        !app.draw_dirty && app.doc.view.is_some()
    })
    .await;
    rims
}

async fn extrude_holes(host: &mut Harness, holes: usize) {
    let SketchStatus::Finished { result, .. } =
        &host.app.doc.sketch.session.as_ref().unwrap().status
    else {
        panic!("Finish result")
    };
    let region = result
        .regions
        .iter()
        .find(|region| region.holes.len() == holes)
        .unwrap()
        .region_id
        .clone();
    let display = host.app.doc.display.as_ref().unwrap();
    let profile = display
        .profiles
        .iter()
        .find(|profile| profile.reference.region.to_string() == region)
        .unwrap();
    let context = host.app.doc.interaction.context.clone().unwrap();
    let view = ViewSnapshot::new(
        context.scene,
        host.app.view_number + 1,
        Default::default(),
        onecad_scene::Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let world = host
        .sketch()
        .effective_plane()
        .to_world(Vec2::new(45.0, 10.0).unwrap());
    let logical = view
        .project(DVec3::new(world.x, world.y, world.z))
        .unwrap()
        .logical;
    let (anchor, _) = profile.pick(&view, logical).expect("material profile");
    let mut reference = profile.reference.clone();
    reference.region_anchor = Some(anchor);
    let step_index = display.projection.applied_ops as u32;
    host.app
        .dispatch(Action::SelectProfile(Box::new(ProfileSelection {
            profile: reference,
            context,
            capture: view.identity(),
        })));
    host.app.dispatch(Action::BeginExtrude {
        record_id: RecordId::new(),
        gesture: 1,
        step_index,
    });
    host.app.dispatch(Action::DraftChanged("4".into()));
    host.wait(|app| {
        app.doc.interaction.tool.as_ref().is_some_and(|tool| {
            matches!(&tool.draft_status, DraftStatus::Valid(_))
                && tool.preview == PreviewStatus::Ready
        })
    })
    .await;
    host.app.dispatch(Action::Confirm);
    host.wait(|app| {
        matches!(
            &app.doc.interaction.status,
            CommitStatus::Completed {
                outcome: FeatureOutcome::Published,
                ..
            }
        )
    })
    .await;
}

/// Opens the seeded screw on its detail page with thread `M10`, so a hole-driven size
/// is visibly an automatic change from the draft.
pub(super) async fn open_detail(host: &mut Harness, root: &std::path::Path) {
    let library = root.join("qa").join("data").join("library");
    onecad_app::library::seed_and_reindex_at(&library).unwrap();
    onecad_app::library::component_detail_at(&library, "onecad.std.iso4762", "1.0.1").unwrap();
    host.app.ui.start_open = true;
    host.app.ui.start_page = StartPage::Library;
    until(host, "the catalog", |app| app.ui.library.catalog.loaded()).await;
    crate::library_pages::open_component(&mut host.app.ui, "onecad.std.iso4762", "1.0.1");
    until(host, "the detail", |app| {
        app.ui
            .library
            .catalog
            .detail()
            .is_some_and(|view| !view.versions.is_empty())
    })
    .await;
    let draft = host
        .app
        .ui
        .library
        .catalog
        .detail_mut()
        .and_then(|view| view.draft.as_mut())
        .unwrap();
    draft.set_text("thread", "M10".into());
}

/// Presses Insert (the page owner arms on the next frame) and waits for the ghost.
pub(super) async fn arm(host: &mut Harness) {
    host.app.ui.start_open = true;
    host.app.ui.start_page = StartPage::ComponentDetail;
    host.app.ui.library.insert_requested = true;
    until(host, "the placement to arm", |app| {
        app.doc
            .placement
            .as_ref()
            .is_some_and(|p| matches!(p.phase, Phase::Armed { .. }))
            && app
                .doc
                .interaction
                .tool
                .as_ref()
                .is_some_and(|t| t.preview == PreviewStatus::Ready)
    })
    .await;
    assert!(!host.app.ui.start_open, "arming shows the editor");
}

fn placed_params(app: &App) -> onecad_core::document::record::PlaceComponentParams {
    match &app.doc.interaction.tool.as_ref().unwrap().effective_op() {
        Operation::Known(KnownOperation::PlaceComponent(p)) => p.clone(),
        other => panic!("not a placement: {other:?}"),
    }
}

/// Hover `world` until the worker-resolved ghost carries a mate of `kind`.
pub(super) async fn snap_to(host: &mut Harness, world: DVec3, kind: MateKind) -> egui::Pos2 {
    let position = rim_position(host, world);
    hover(host, position);
    // The previous snap may already satisfy the wait below: let this hover take effect.
    for _ in 0..5 {
        frame(host, vec![], egui::Modifiers::NONE);
    }
    until(host, "the rim snap", |app| {
        app.doc.placement.as_ref().is_some_and(|p| {
            matches!(&p.snap, SnapIntent::Mate(c) if c.kind == kind)
        }) && app.doc.interaction.tool.as_ref().is_some_and(|t| {
            t.preview == PreviewStatus::Ready
                && matches!(&t.effective_op(), Operation::Known(KnownOperation::PlaceComponent(p))
                    if p.mate.as_ref().is_some_and(|m| m.kind == kind))
        })
    })
    .await;
    position
}

fn history(app: &App) -> (usize, usize) {
    let display = app.doc.display.as_ref().unwrap();
    (display.records.len(), display.scene.bodies().len())
}

fn count(app: &App, detached: bool) -> usize {
    app.doc
        .display
        .as_ref()
        .unwrap()
        .records
        .iter()
        .filter(|record| match &record.op {
            Operation::Known(KnownOperation::PlaceComponent(_)) => !detached,
            Operation::Known(KnownOperation::DetachComponent(_)) => detached,
            _ => false,
        })
        .count()
}

pub(super) async fn published(host: &mut Harness) {
    until(host, "the placement to publish", |app| {
        matches!(
            &app.doc.interaction.status,
            CommitStatus::Completed {
                outcome: FeatureOutcome::Published,
                ..
            }
        ) && app.doc.interaction.tool.is_none()
    })
    .await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn component_placement_snaps_to_a_hole_places_once_and_undoes() {
    assert!(std::env::var_os("ONECAD_LIBRARY_ROOT").is_none());
    let area = tempfile::tempdir().unwrap();
    let mut host = Harness::new(area.path()).await;
    let rim = plate_with_hole(&mut host).await;
    open_detail(&mut host, area.path()).await;
    let before = history(&host.app);

    // Linked: hover the rim, the hole sizes the screw, one click places one record.
    arm(&mut host).await;
    let position = snap_to(&mut host, rim, MateKind::ConcentricAndCoincident).await;
    let draft = placed_params(&host.app);
    assert_eq!(
        draft.params.get("thread"),
        Some(&ComponentParamValue::Text("M6".into()))
    );
    click(&mut host, position, egui::Modifiers::NONE);
    published(&mut host).await;
    assert!(host.app.doc.placement.is_none(), "a plain click disarms");
    assert_eq!(history(&host.app), (before.0 + 1, before.1 + 1));
    assert_eq!(count(&host.app, false), 1);
    let placed = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .records
        .last()
        .unwrap()
        .clone();
    let Operation::Known(KnownOperation::PlaceComponent(params)) = &placed.op else {
        panic!("PlaceComponent")
    };
    assert_eq!(
        params.params.get("thread"),
        Some(&ComponentParamValue::Text("M6".into()))
    );
    assert_eq!(
        params.mate.as_ref().unwrap().kind,
        MateKind::ConcentricAndCoincident
    );

    // One Undo step removes it; Redo restores it.
    crate::input::submit_history(&mut host.app, Command::Undo);
    host.wait(|app| history(app) == before).await;
    assert_eq!(count(&host.app, false), 0);
    crate::input::submit_history(&mut host.app, Command::Redo);
    host.wait(|app| history(app) == (before.0 + 1, before.1 + 1))
        .await;
    assert_eq!(count(&host.app, false), 1);
    host.app.stop_discard();
    host.wait(|app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn component_placement_copy_cancel_and_repeat() {
    assert!(std::env::var_os("ONECAD_LIBRARY_ROOT").is_none());
    let area = tempfile::tempdir().unwrap();
    let mut host = Harness::new(area.path()).await;
    let rim = plate_with_hole(&mut host).await;
    // The occupied hole is no longer the target of the last leg: a clear spot on the top face.
    let face = DVec3::new(rim.x, rim.y + 42.0, rim.z);
    open_detail(&mut host, area.path()).await;
    let before = history(&host.app);

    // Esc leaves history unchanged and nothing armed.
    arm(&mut host).await;
    snap_to(&mut host, rim, MateKind::ConcentricAndCoincident).await;
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut host, vec![escape], egui::Modifiers::NONE);
    until(&mut host, "the cancel", |app| {
        app.doc.interaction.tool.is_none() && app.doc.placement.is_none()
    })
    .await;
    assert_eq!(history(&host.app), before);

    // Copy places an inert DetachComponent.
    arm(&mut host).await;
    crate::placement_gesture::set_copy(&mut host.app, true);
    let position = snap_to(&mut host, rim, MateKind::ConcentricAndCoincident).await;
    click(&mut host, position, egui::Modifiers::NONE);
    published(&mut host).await;
    assert_eq!(count(&host.app, true), 1);
    assert_eq!(count(&host.app, false), 0);
    assert_eq!(history(&host.app), (before.0 + 1, before.1 + 1));

    // Shift+click places and stays armed for the next one.
    arm(&mut host).await;
    let position = snap_to(&mut host, face, MateKind::Coincident).await;
    click(&mut host, position, egui::Modifiers::SHIFT);
    until(&mut host, "the repeat to re-arm", |app| {
        count(app, false) == 1
            && app
                .doc
                .placement
                .as_ref()
                .is_some_and(|p| matches!(p.phase, Phase::Armed { .. }))
    })
    .await;
    assert!(host.app.doc.interaction.tool.is_some());
    crate::pill::cancel_tool(&mut host.app);
    until(&mut host, "the final cancel", |app| {
        app.doc.placement.is_none()
    })
    .await;
    host.app.stop_discard();
    host.wait(|app| app.may_close).await;
}

fn escape() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn component_placement_arming_and_lookup_guards() {
    use crate::placement_gesture::{Knowledge, REQUEST_TIMEOUT};
    assert!(std::env::var_os("ONECAD_LIBRARY_ROOT").is_none());
    let area = tempfile::tempdir().unwrap();
    let mut host = Harness::new(area.path()).await;
    let rim = plate_with_hole(&mut host).await;
    open_detail(&mut host, area.path()).await;
    let before = history(&host.app);

    // Esc while the draft is still being prepared cancels the placement.
    crate::placement_gesture::arm_from_detail(&mut host.app);
    assert!(crate::placement_gesture::arming(&host.app));
    assert!(host.app.busy(), "an arming placement owns the editor");
    frame(&mut host, vec![escape()], egui::Modifiers::NONE);
    assert!(host.app.doc.placement.is_none());

    // A tool the user opens before the reply is kept; the placement is dropped.
    crate::placement_gesture::arm_from_detail(&mut host.app);
    crate::modeling_ui::begin_body_operation(
        &mut host.app,
        crate::tool_registry::ToolId::Generators,
    );
    assert!(host.app.doc.interaction.tool.is_some());
    until(&mut host, "the placement to drop", |app| {
        app.doc.placement.is_none()
    })
    .await;
    assert!(matches!(
        host.app.doc.interaction.tool.as_ref().map(|t| &t.record.op),
        Some(Operation::Known(KnownOperation::Gear(_)))
    ));
    crate::pill::cancel_tool(&mut host.app);

    // A reply that never comes times out; the user may retry.
    crate::placement_gesture::arm_from_detail(&mut host.app);
    crate::placement_gesture::poll_at(&mut host.app, std::time::Instant::now() + REQUEST_TIMEOUT);
    assert!(host.app.doc.placement.is_none());
    assert!(
        host.app.doc.status.contains("timed out"),
        "{}",
        host.app.doc.status
    );

    // Armed: the card button shares the guard of the click.
    arm(&mut host).await;
    snap_to(&mut host, rim, MateKind::ConcentricAndCoincident).await;
    host.app.doc.placement.as_mut().unwrap().hover.class = Knowledge::Pending;
    crate::placement_gesture::confirm(&mut host.app);
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().phase,
        onecad_interaction::SessionPhase::Editing
    );

    // A lookup that never answers times out; a late reply is dropped.
    let target = host
        .app
        .doc
        .placement
        .as_ref()
        .unwrap()
        .hover
        .target
        .clone()
        .unwrap();
    let context = host.app.doc.interaction.context.clone().unwrap();
    let stale = {
        let mut stale = context.clone();
        stale.revision += 1;
        stale
    };
    let key = (target.body, target.topology.clone());
    let flight = |sequence, context| crate::placement_gesture::InFlight {
        sequence,
        key: key.clone(),
        context,
        since: std::time::Instant::now(),
    };
    // Stale context: the (failure) reply is dropped, not shown.
    let catalog = &mut host.app.ui.library.catalog;
    let sequence = catalog.request_classify(crate::library_catalog::ClassifyTarget {
        body_id: String::new(),
        topo_key: None,
        element_id: None,
        point: [0.0; 3],
        fence: None,
    });
    catalog.refuse(sequence, "late".into());
    let placement = host.app.doc.placement.as_mut().unwrap();
    placement.hover.in_flight = Some(flight(sequence, stale));
    placement.hover.class = Knowledge::Pending;
    placement.notice = None;
    crate::placement_gesture::poll_at(&mut host.app, std::time::Instant::now());
    let placement = host.app.doc.placement.as_ref().unwrap();
    assert!(placement.notice.is_none(), "{:?}", placement.notice);
    // Timeout: in flight clears, the target resets, and the lane forgets the sequence.
    let sequence =
        host.app
            .ui
            .library
            .catalog
            .request_classify(crate::library_catalog::ClassifyTarget {
                body_id: String::new(),
                topo_key: None,
                element_id: None,
                point: [0.0; 3],
                fence: None,
            });
    let placement = host.app.doc.placement.as_mut().unwrap();
    placement.hover.in_flight = Some(flight(sequence, context));
    placement.hover.class = Knowledge::Pending;
    crate::placement_gesture::poll_at(&mut host.app, std::time::Instant::now() + REQUEST_TIMEOUT);
    let placement = host.app.doc.placement.as_ref().unwrap();
    assert!(placement.hover.target.is_none() && placement.hover.in_flight.is_none());
    assert!(
        placement
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("timed out"))
    );
    host.app.ui.library.catalog.refuse(sequence, "late".into());
    assert!(
        host.app
            .ui
            .library
            .catalog
            .take_placement_reply(sequence)
            .is_none()
    );
    crate::pill::cancel_tool(&mut host.app);
    until(&mut host, "disarm", |app| app.doc.placement.is_none()).await;
    assert_eq!(history(&host.app), before);
    host.app.stop_discard();
    host.wait(|app| app.may_close).await;
}

/// Bytes this document's budget owner holds, by kind.
fn held(app: &App) -> std::collections::BTreeMap<String, usize> {
    let budget = &app.doc.bridge.shared.host_budget;
    let snapshot = budget.snapshot();
    snapshot
        .owners
        .get(&budget.owner_id())
        .map(|owner| {
            owner
                .kinds
                .iter()
                .map(|(kind, bytes)| (format!("{kind:?}"), *bytes))
                .collect()
        })
        .unwrap_or_default()
}

/// Everything preview-shaped is returned once the gesture is over.
async fn transient_released(host: &mut Harness, what: &str) {
    // Display and highlight reservations belong to the committed document; the
    // placement ghost's Preview/PreviewSource portions must all be back.
    let ghost = |app: &App| {
        held(app)
            .iter()
            .filter(|(kind, _)| matches!(kind.as_str(), "Preview" | "PreviewSource"))
            .map(|(_, bytes)| *bytes)
            .sum::<usize>()
    };
    until(host, what, |app| {
        app.doc.interaction.tool.is_none() && app.doc.pending_preview.is_none()
    })
    .await;
    for i in 0..60 {
        frame(host, vec![], egui::Modifiers::NONE);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        if i % 1000 == 999 {
            eprintln!("BUDGET t={i} {what}: {:?}", held(&host.app));
        }
    }
    eprintln!("BUDGET after {what}: {:?}", held(&host.app));
    assert_eq!(ghost(&host.app), 0, "{what}: {:?}", held(&host.app));
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn component_placement_returns_preview_budget_across_sequential_placements() {
    assert!(std::env::var_os("ONECAD_LIBRARY_ROOT").is_none());
    let area = tempfile::tempdir().unwrap();
    let mut host = Harness::new(area.path()).await;
    assert_eq!(host.app.tabs.parked.len(), 1, "a second document");
    let rim = plate_with_hole(&mut host).await;
    open_detail(&mut host, area.path()).await;
    eprintln!("BUDGET baseline: {:?}", held(&host.app));
    let spot = |dy: f64| DVec3::new(rim.x, rim.y + dy, rim.z);

    // 1 Linked at the rim, 2 Copy on the face, 3+4 Shift repeat, 5 cancel.
    arm(&mut host).await;
    let at = snap_to(&mut host, rim, MateKind::ConcentricAndCoincident).await;
    click(&mut host, at, egui::Modifiers::NONE);
    published(&mut host).await;
    transient_released(&mut host, "linked commit").await;

    arm(&mut host).await;
    crate::placement_gesture::set_copy(&mut host.app, true);
    let at = snap_to(&mut host, spot(18.0), MateKind::Coincident).await;
    click(&mut host, at, egui::Modifiers::NONE);
    published(&mut host).await;
    transient_released(&mut host, "copy commit").await;

    arm(&mut host).await;
    let at = snap_to(&mut host, spot(30.0), MateKind::Coincident).await;
    click(&mut host, at, egui::Modifiers::SHIFT);
    until(&mut host, "the repeat to re-arm", |app| {
        app.doc
            .placement
            .as_ref()
            .is_some_and(|p| matches!(p.phase, Phase::Armed { .. }))
            && app
                .doc
                .interaction
                .tool
                .as_ref()
                .is_some_and(|t| t.preview == PreviewStatus::Ready)
    })
    .await;
    let at = snap_to(&mut host, spot(42.0), MateKind::Coincident).await;
    click(&mut host, at, egui::Modifiers::NONE);
    published(&mut host).await;
    transient_released(&mut host, "repeat commit").await;

    arm(&mut host).await;
    snap_to(&mut host, spot(54.0), MateKind::Coincident).await;
    frame(&mut host, vec![escape()], egui::Modifiers::NONE);
    transient_released(&mut host, "cancel").await;

    // A further preview still arrives.
    arm(&mut host).await;
    snap_to(&mut host, spot(54.0), MateKind::Coincident).await;
    crate::pill::cancel_tool(&mut host.app);
    transient_released(&mut host, "last cancel").await;
    host.app.stop_discard();
    host.wait(|app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; control for the budget reproduction"]
async fn control_second_feature_commit_budget() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Harness::new(area.path()).await;
    plate_with_hole(&mut host).await;
    eprintln!("BUDGET control baseline: {:?}", held(&host.app));
    for round in 0..4 {
        crate::modeling_ui::begin_body_operation(
            &mut host.app,
            crate::tool_registry::ToolId::Generators,
        );
        until(&mut host, "gear preview", |app| {
            app.doc
                .interaction
                .tool
                .as_ref()
                .is_some_and(|t| t.preview == PreviewStatus::Ready)
        })
        .await;
        host.app.dispatch(Action::Confirm);
        published(&mut host).await;
        for _ in 0..100 {
            frame(&mut host, vec![], egui::Modifiers::NONE);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        eprintln!("BUDGET control round {round}: {:?}", held(&host.app));
    }
    let before = history(&host.app);
    let mut initial = None;
    for cycle in 0..20 {
        crate::input::submit_history(&mut host.app, Command::Undo);
        host.wait(|app| history(app).1 < before.1).await;
        crate::input::submit_history(&mut host.app, Command::Redo);
        host.wait(|app| history(app).1 == before.1).await;
        let snapshot = host.app.doc.bridge.shared.host_budget.snapshot();
        let start = *initial.get_or_insert(snapshot.used);
        eprintln!("BUDGET mixed undo/redo {cycle}: {snapshot:?}");
        assert!(
            snapshot.used <= start + (16 << 20),
            "mixed history retention grew: {snapshot:?}"
        );
    }
}

/// The placed instance's record id and the element its mate seats on.
fn instance(app: &App) -> (RecordId, String, [f64; 3]) {
    let display = app.doc.display.as_ref().unwrap();
    let record = display
        .records
        .iter()
        .find(|record| {
            matches!(
                record.op,
                Operation::Known(KnownOperation::PlaceComponent(_))
            )
        })
        .expect("a placed instance");
    let Operation::Known(KnownOperation::PlaceComponent(params)) = &record.op else {
        unreachable!()
    };
    let mate = params.mate.as_ref().expect("mated");
    let element = mate
        .target
        .primary
        .as_ref()
        .unwrap()
        .element
        .as_str()
        .to_owned();
    (
        record.record_id,
        element,
        params.placement.translate.clone().map(|s| s.value),
    )
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn component_placement_edit_moves_an_instance_to_another_hole_with_one_undo() {
    assert!(std::env::var_os("ONECAD_LIBRARY_ROOT").is_none());
    let area = tempfile::tempdir().unwrap();
    let mut host = Harness::new(area.path()).await;
    let rims = plate_with_holes(&mut host, 2).await;
    open_detail(&mut host, area.path()).await;
    let before = history(&host.app);

    arm(&mut host).await;
    let at = snap_to(&mut host, rims[0], MateKind::ConcentricAndCoincident).await;
    click(&mut host, at, egui::Modifiers::NONE);
    published(&mut host).await;
    let (id, first, first_pose) = instance(&host.app);
    let placed = history(&host.app);
    assert_eq!(placed, (before.0 + 1, before.1 + 1));

    // Edit placement: the same record, Linked only, re-seated on the other hole.
    crate::placement_gesture::edit_existing(&mut host.app, &id.to_string());
    until(&mut host, "the edit to arm", |app| {
        crate::placement_gesture::armed(app)
    })
    .await;
    let tool = host.app.doc.interaction.tool.as_ref().unwrap();
    assert!(tool.editing_existing && tool.tag.record_id == id);
    assert!(host.app.doc.placement.as_ref().unwrap().editing.is_some());
    // Hovering the instance itself falls through to what is behind it, never to itself.
    let own = onecad_core::ids::BodyId(id.0);
    let over_self = rim_position(&host, rims[0]);
    hover(&mut host, over_self);
    until(&mut host, "the hover over the instance", |app| {
        app.doc.placement.as_ref().is_some_and(|p| {
            p.hover.target.is_some()
                && matches!(p.hover.class, crate::placement_gesture::Knowledge::Known(_))
        })
    })
    .await;
    let target = host
        .app
        .doc
        .placement
        .as_ref()
        .unwrap()
        .hover
        .target
        .clone()
        .unwrap();
    assert_ne!(
        target.body, own,
        "the instance's own body is never a target"
    );
    let at = snap_to(&mut host, rims[1], MateKind::ConcentricAndCoincident).await;
    click(&mut host, at, egui::Modifiers::NONE);
    published(&mut host).await;
    let (same, second, second_pose) = instance(&host.app);
    assert_eq!(same, id, "the same record");
    assert_eq!(history(&host.app), placed, "an update, not a new feature");
    assert_ne!(first, second, "seated on the other hole");
    assert_ne!(first_pose, second_pose);

    // One Undo restores the original pose; Redo re-applies the edit.
    crate::input::submit_history(&mut host.app, Command::Undo);
    host.wait(|app| instance(app).1 == first).await;
    assert_eq!(instance(&host.app).2, first_pose);
    assert_eq!(history(&host.app), placed);
    crate::input::submit_history(&mut host.app, Command::Redo);
    host.wait(|app| instance(app).1 == second).await;

    // Esc restores: no history entry.
    crate::placement_gesture::edit_existing(&mut host.app, &id.to_string());
    until(&mut host, "the second edit to arm", |app| {
        crate::placement_gesture::armed(app)
    })
    .await;
    snap_to(&mut host, rims[0], MateKind::ConcentricAndCoincident).await;
    frame(&mut host, vec![escape()], egui::Modifiers::NONE);
    until(&mut host, "the cancel", |app| {
        app.doc.interaction.tool.is_none() && app.doc.placement.is_none()
    })
    .await;
    assert_eq!(instance(&host.app).1, second);
    assert_eq!(history(&host.app), placed);
    host.app.stop_discard();
    host.wait(|app| app.may_close).await;
}
