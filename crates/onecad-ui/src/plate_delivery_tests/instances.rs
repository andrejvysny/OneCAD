//! Placed component instances through the production host with the real worker and Metal:
//! Explorer rows, parameter edit, reset, detach and the upgrade review, each one undo
//! step. Store-level actions stand in for button presses; synthetic delivery is not
//! physical input acceptance.
use super::{
    placement::{arm, click, frame, open_detail, plate_with_hole, published, snap_to, until},
    *,
};
use crate::component_instances::{InstanceView, Op, model};
use onecad_app::dto::ComponentStatusDto;
use onecad_core::document::record::{ComponentParamValue as Value, MateKind};

/// Runs a frame and returns the text it painted.
fn painted(host: &mut Harness) -> String {
    fn walk(shape: &egui::Shape, out: &mut String) {
        match shape {
            egui::Shape::Text(text) => {
                out.push_str(text.galley.text());
                out.push('\n');
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 820.0),
        )),
        focused: true,
        ..Default::default()
    };
    let mut output = host.context.run_ui(raw, |ui| {
        eframe::App::ui(&mut host.app, ui, &mut host.frame)
    });
    let mut text = String::new();
    output.shapes.iter().for_each(|s| walk(&s.shape, &mut text));
    output.textures_delta.clear();
    text
}

/// Paints frames until the page says `needle` (panels slide in over a few frames).
async fn painted_with(host: &mut Harness, needle: &str) -> String {
    let mut page = String::new();
    for _ in 0..60 {
        page = painted(host);
        if page.contains(needle) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    page
}

fn scope(host: &Harness) -> crate::component_instances::Scope {
    crate::component_instances::scope(&host.app).expect("open document")
}

fn revision(app: &App) -> u64 {
    app.doc.document_state.as_ref().map_or(0, |s| s.revision)
}

fn items(app: &App) -> &[InstanceView] {
    app.doc.instances.items()
}

fn length(view: &InstanceView) -> Value {
    view.dto.effective["length"].clone()
}

/// Waits until the listing describes the current revision and satisfies `done`.
async fn listed(host: &mut Harness, what: &str, done: impl Fn(&[InstanceView]) -> bool) {
    until(host, what, |app| {
        app.doc.instances.settled(revision(app)) && !app.doc.instances.blocks() && done(items(app))
    })
    .await;
}

/// Selects the library draft's thread so every placement is an M6 screw.
fn m6(host: &mut Harness) {
    let draft = host
        .app
        .ui
        .library
        .catalog
        .detail_mut()
        .and_then(|view| view.draft.as_mut())
        .unwrap();
    draft.set_text("thread", "M6".into());
}

/// Arms from the detail page and places one record at `world`.
async fn place(host: &mut Harness, world: DVec3, kind: MateKind, copy: bool) {
    m6(host);
    arm(host).await;
    crate::placement_gesture::set_copy(&mut host.app, copy);
    let position = snap_to(host, world, kind).await;
    click(host, position, egui::Modifiers::NONE);
    published(host).await;
}

/// The harness of the other plate tests starts a second document and leaves the first
/// one's scene reserved; three placement previews need that memory, so this one starts
/// on the document the app opened itself.
async fn harness(root: &std::path::Path) -> Harness {
    let (mut app, context) = create_app(root).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    app.new_sketch();
    let mut host = Harness {
        app,
        context,
        frame,
    };
    host.wait(|app| {
        app.doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
    host
}

async fn undo(host: &mut Harness) {
    let before = revision(&host.app);
    crate::input::submit_history(&mut host.app, Command::Undo);
    until(host, "undo", |app| revision(app) != before).await;
}

fn linked(app: &App) -> Vec<&InstanceView> {
    items(app).iter().filter(|v| v.dto.linked).collect()
}

fn mutate(host: &mut Harness, what: &'static str, op: Op) {
    let scope = scope(host);
    assert!(host.app.doc.instances.mutate(&scope, what, op));
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic delivery is not physical acceptance"]
async fn instances_list_edit_reset_detach_and_upgrade_each_in_one_undo_step() {
    assert!(std::env::var_os("ONECAD_LIBRARY_ROOT").is_none());
    let area = tempfile::tempdir().unwrap();
    let mut host = harness(area.path()).await;
    let rim = plate_with_hole(&mut host).await;
    open_detail(&mut host, area.path()).await;
    let spot_b = DVec3::new(rim.x, rim.y + 42.0, rim.z);
    let spot_copy = DVec3::new(rim.x, rim.y + 25.0, rim.z);
    place(&mut host, rim, MateKind::ConcentricAndCoincident, false).await;
    place(&mut host, spot_b, MateKind::Coincident, false).await;
    place(&mut host, spot_copy, MateKind::Coincident, true).await;

    // Explorer: three rows, two Linked and one Copy.
    host.app.ui.start_open = false;
    host.app.ui.explorer_open = true;
    host.app.ui.inspector_open = true;
    listed(&mut host, "three instances", |v| v.len() == 3).await;
    assert_eq!(linked(&host.app).len(), 2);
    assert!(
        linked(&host.app)
            .iter()
            .all(|v| v.dto.status == ComponentStatusDto::Linked)
    );
    assert!(
        linked(&host.app)
            .iter()
            .all(|v| model::title(&v.dto) == "ISO 4762 M6x20")
    );
    let copy_id = items(&host.app)
        .iter()
        .find(|v| !v.dto.linked)
        .unwrap()
        .dto
        .record_id
        .clone();
    let page = painted_with(&mut host, "COMPONENTS").await;
    assert!(page.contains("COMPONENTS"), "{page}");
    assert_eq!(page.matches("Linked").count(), 2, "{page}");
    assert_eq!(page.matches("Copy").count(), 1, "{page}");

    // Edit length on one: one undo step, the other instance is unchanged.
    let (first, second) = {
        let l = linked(&host.app);
        (l[0].dto.record_id.clone(), l[1].dto.record_id.clone())
    };
    crate::pill::select_feature(&mut host.app, first.parse().unwrap());
    until(&mut host, "the inspector draft", |app| {
        app.doc.instances.edit.is_some()
    })
    .await;
    let page = painted(&mut host);
    assert!(
        page.contains("PARAMETERS") && page.contains("Detach"),
        "{page}"
    );
    let before = revision(&host.app);
    host.app
        .doc
        .instances
        .edit_for(&first)
        .unwrap()
        .draft
        .set_text("length", "35".into());
    listed(&mut host, "the edited length", |v| {
        v.iter()
            .any(|i| i.dto.record_id == first && length(i) == Value::Number(35.0))
    })
    .await;
    assert_eq!(revision(&host.app), before + 1, "one history step");
    let other = host.app.doc.instances.find(&second).unwrap();
    assert_eq!(length(other), Value::Number(20.0));
    assert!(other.dto.overrides.is_empty());
    assert!(
        host.app
            .doc
            .instances
            .find(&first)
            .unwrap()
            .dto
            .overrides
            .contains(&"length".to_owned())
    );
    undo(&mut host).await;
    listed(&mut host, "the undone length", |v| {
        v.iter().all(|i| length(i) == Value::Number(20.0))
    })
    .await;
    let redo_before = revision(&host.app);
    crate::input::submit_history(&mut host.app, Command::Redo);
    until(&mut host, "redo", |app| revision(app) != redo_before).await;
    listed(&mut host, "the redone length", |v| {
        v.iter()
            .any(|i| i.dto.record_id == first && length(i) == Value::Number(35.0))
    })
    .await;

    // Reset returns the key to the package default in one step.
    let before = revision(&host.app);
    mutate(
        &mut host,
        "Reset component parameters",
        Op::Reset {
            record_id: first.clone(),
            keys: Some(vec!["length".into()]),
        },
    );
    listed(&mut host, "the reset", |v| {
        v.iter().all(|i| length(i) == Value::Number(20.0))
    })
    .await;
    assert_eq!(revision(&host.app), before + 1);
    assert!(
        host.app
            .doc
            .instances
            .find(&first)
            .unwrap()
            .dto
            .overrides
            .is_empty()
    );

    // Detach asks first, then the row is a Copy; one undo links it again.
    host.app.doc.instances.confirm_detach = Some(first.clone());
    let page = painted_with(&mut host, "Detach breaks the link").await;
    assert!(
        page.contains(crate::component_instances::DETACH_WARNING) && page.contains("Confirm"),
        "{page}"
    );
    host.app.doc.instances.confirm_detach = None;
    mutate(
        &mut host,
        "Detach component",
        Op::Detach {
            record_id: first.clone(),
        },
    );
    listed(&mut host, "the detach", |v| {
        v.iter().filter(|i| !i.dto.linked).count() == 2
    })
    .await;
    assert!(!host.app.doc.instances.find(&first).unwrap().dto.linked);
    undo(&mut host).await;
    listed(&mut host, "the detach undone", |v| {
        v.iter().filter(|i| i.dto.linked).count() == 2
    })
    .await;

    // A newer package version: Review updates, apply, both linked on 1.1.0, one undo.
    let library = area.path().join("qa").join("data").join("library");
    let manifest =
        std::fs::read_to_string(library.join("onecad.std.iso4762").join("component.toml")).unwrap();
    let next = library.join("onecad.std.iso4762-1.1.0");
    std::fs::create_dir_all(&next).unwrap();
    std::fs::write(
        next.join("component.toml"),
        manifest.replace("version = \"1.0.0\"", "version = \"1.1.0\""),
    )
    .unwrap();
    host.app.ui.library.catalog.reindex();
    crate::library_catalog::flush(&mut host.app);
    listed(&mut host, "the upgrade badges", |v| {
        v.iter()
            .filter(|i| matches!(i.dto.status, ComponentStatusDto::UpgradeAvailable { .. }))
            .count()
            == 2
    })
    .await;
    let page = painted_with(&mut host, "Review 2 updates").await;
    assert!(page.contains("Review 2 updates"), "{page}");
    let scope = scope(&host);
    host.app.doc.instances.open_review(&scope);
    until(&mut host, "the plan", |app| {
        app.doc
            .instances
            .review
            .as_ref()
            .is_some_and(|r| !r.loading)
    })
    .await;
    let review = host.app.doc.instances.review.as_ref().unwrap();
    assert_eq!(review.approved_all().len(), 2);
    assert!(
        review
            .items
            .iter()
            .all(|i| i.from_version == "1.0.0" && i.to_version == "1.1.0")
    );
    let approved = review.approved_all();
    let page = painted_with(&mut host, "Review component updates").await;
    assert!(
        page.contains("Review component updates") && page.contains("1.0.0 → 1.1.0"),
        "{page}"
    );
    let before = revision(&host.app);
    mutate(&mut host, "Update components", Op::Upgrade { approved });
    listed(&mut host, "the upgrade", |v| {
        v.iter()
            .filter(|i| i.dto.linked)
            .all(|i| i.dto.component_version == "1.1.0")
    })
    .await;
    assert_eq!(revision(&host.app), before + 1, "one history step");
    assert!(
        host.app.doc.instances.review.is_none(),
        "a committed update closes the review"
    );
    let copy = host.app.doc.instances.find(&copy_id).unwrap();
    assert!(!copy.dto.linked, "the copy is untouched");
    undo(&mut host).await;
    listed(&mut host, "the upgrade undone", |v| {
        v.iter()
            .filter(|i| i.dto.linked)
            .all(|i| i.dto.component_version == "1.0.0")
    })
    .await;
    frame(&mut host, vec![], egui::Modifiers::NONE);
    host.app.stop_discard();
    host.wait(|app| app.may_close).await;
}
