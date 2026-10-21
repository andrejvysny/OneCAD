//! Multi-document delivery with real worker and Metal: unfinished tools across a
//! switch, same-frame event order, Save All and background provisional tabs.
//! Synthetic input; native file dialogs have no seam here and are never opened.
use super::dimension_delivery_tests::{accept_capture, click, ids};
use super::document_tabs_tests::{ctrl_tab, raw, two_tabs};
use super::*;
use crate::document_flow::Outcome;
use onecad_core::{
    document::Document,
    ids::{DocumentId, RecordId},
    io::container::{ContainerCaches, ContainerWriter, SaveMeta},
    sketch::WorldPlane,
};
use onecad_interaction::{
    Action, CommitStatus, DraftStatus, FeatureOutcome, PreviewStatus, ProfileSelection,
    sketch::{SketchAction, SketchPhase, SketchTool},
};
use std::path::{Path, PathBuf};

fn titled_fixture(root: &Path, name: &str) -> PathBuf {
    let paths =
        QaPaths::from_args(["--qa-root".into(), root.join("qa").into_os_string()].into_iter())
            .unwrap();
    let destination = paths.root.join(format!("documents/{name}.onecad"));
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    ContainerWriter::save(
        &destination,
        &Document::new(DocumentId::new()),
        &ContainerCaches::none(),
        &SaveMeta {
            app_version: "tab-delivery".into(),
            created: "2026-10-21T00:00:00Z".into(),
            modified: "2026-10-21T00:00:00Z".into(),
            ..Default::default()
        },
    )
    .unwrap();
    destination
}

/// An untitled document, then `name` opened from a file in a tab of its own.
async fn untitled_and_titled(root: &Path, name: &str) -> (App, egui::Context, eframe::Frame) {
    let path = titled_fixture(root, name);
    let (mut app, context) = create_app(root).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.document_open() && app.doc.display.is_some() && !app.busy()
    })
    .await;
    app.document_request(Request::Open(Some(path)));
    pump_until(&mut app, &context, &mut frame, |app| {
        app.tabs.parked.len() == 1 && app.document_open() && !app.busy()
    })
    .await;
    assert!(has_file(&app.doc));
    (app, context, frame)
}

fn has_file(doc: &crate::document_editor::DocumentEditor) -> bool {
    doc.document_state
        .as_ref()
        .is_some_and(|state| state.path.is_some())
}

fn parked(app: &App, tab: crate::documents::TabId) -> &crate::document_editor::DocumentEditor {
    app.tabs.parked.iter().find(|doc| doc.tab == tab).unwrap()
}

fn parked_mut(
    app: &mut App,
    tab: crate::documents::TabId,
) -> &mut crate::document_editor::DocumentEditor {
    app.tabs
        .parked
        .iter_mut()
        .find(|doc| doc.tab == tab)
        .unwrap()
}

/// One raw frame through the production hook; returns what egui would still see.
fn frame_events(
    app: &mut App,
    context: &egui::Context,
    events: Vec<egui::Event>,
) -> Vec<egui::Event> {
    // Every call is a fresh frame, never a logic-tick replay of the last one.
    app.input_routing = Default::default();
    let mut input = raw(events);
    app.raw_input_hook(context, &mut input);
    input.events
}

fn command(key: egui::Key, shift: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command: true,
            mac_cmd: true,
            shift,
            ..Default::default()
        },
    }
}

fn button(pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: egui::pos2(300.0, 200.0),
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

async fn rectangle_profile(app: &mut App, ctx: &egui::Context, frame: &mut eframe::Frame) {
    app.new_sketch_on_plane(WorldPlane::XY);
    pump_until(app, ctx, frame, |app| {
        app.doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
    let capture = accept_capture(app);
    app.sketch_dispatch(SketchAction::SetTool(SketchTool::Rectangle));
    click(app, capture, 0.0, 0.0, ids());
    let rectangle = ids();
    click(app, capture, 40.0, 30.0, rectangle.clone());
    pump_until(app, ctx, frame, |app| {
        app.doc.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active
                && session.authoritative.contains_entity(rectangle.entities[7])
        })
    })
    .await;
    app.sketch_dispatch(SketchAction::Finish);
    pump_until(app, ctx, frame, |app| {
        !app.sketch_active()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|display| !display.profiles.is_empty())
    })
    .await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn a_draft_left_mid_preview_republishes_on_return_and_confirms_once() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let (other, drafting) = (app.tabs.parked[0].tab, app.doc.tab);
    rectangle_profile(&mut app, &context, &mut frame).await;
    let display = app.doc.display.clone().unwrap();
    let records = display.records.len();
    let document = crate::sketch_services::document_context(&display);
    let view = onecad_scene::ViewSnapshot::new(
        document.scene,
        app.view_number + 1,
        app.doc.camera,
        onecad_scene::Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    app.dispatch(Action::SelectProfile(Box::new(ProfileSelection {
        profile: display.profiles[0].reference.clone(),
        context: document,
        capture: view.identity(),
    })));
    let record = RecordId::new();
    app.dispatch(Action::BeginExtrude {
        record_id: record,
        gesture: 1,
        step_index: display.projection.applied_ops as u32,
    });
    app.dispatch(Action::DraftChanged("5".into()));
    // Leave before its evaluation or preview can answer: parking cancels both.
    app.activate_tab(other);
    for _ in 0..20 {
        logic_tick(&mut app, &context, &mut frame, false);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let tool = parked(&app, drafting).interaction.tool.as_ref().unwrap();
    assert_eq!(tool.draft, "5");
    assert_eq!(tool.draft_status, DraftStatus::Pending, "nothing answers");

    // Shown while its file action runs (simulated): the draft is held, as a
    // draft change would be, and re-issued once the action ends.
    parked_mut(&mut app, drafting).documents.running = true;
    app.activate_tab(drafting);
    for _ in 0..10 {
        logic_tick(&mut app, &context, &mut frame, false);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let tool = app.doc.interaction.tool.as_ref().unwrap();
    assert_eq!(
        tool.draft_status,
        DraftStatus::Pending,
        "held during the save"
    );
    assert_ne!(
        app.doc.status,
        "Finish the file action before editing the document"
    );
    app.doc.documents.running = false;
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.interaction.tool.as_ref().is_some_and(|tool| {
            matches!(&tool.draft_status, DraftStatus::Valid(value) if value.value == 5.0)
                && tool.preview == PreviewStatus::Ready
        })
    })
    .await;
    app.dispatch(Action::Confirm);
    pump_until(&mut app, &context, &mut frame, |app| {
        matches!(
            app.doc.interaction.status,
            CommitStatus::Completed {
                outcome: FeatureOutcome::Published,
                ..
            }
        )
    })
    .await;
    let display = app.doc.display.as_ref().unwrap();
    assert_eq!(display.records.len(), records + 1, "one record committed");
    assert!(display.records.iter().any(|op| op.record_id == record));
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn ctrl_release_commits_at_its_position_and_a_later_ctrl_tab_reopens() {
    use crate::tab_switcher::Target;
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let (older, newer) = (app.tabs.parked[0].tab, app.doc.tab);
    let modifiers = egui::Event::ModifiersChanged;

    // A release observed before this frame's ⌃Tab cannot commit what it opens.
    frame_events(
        &mut app,
        &context,
        vec![
            modifiers(egui::Modifiers::SHIFT),
            modifiers(egui::Modifiers::CTRL),
            ctrl_tab(false),
        ],
    );
    assert_eq!(app.doc.tab, newer);
    let state = app.switcher.clone().expect("switcher stays open");
    assert_eq!(state.order[state.selected], Target::Tab(older));
    app.switcher = None;

    // ⌃Tab, release, Control, Tab: one switch at the release, then a new switcher
    // walking from the document that is now shown.
    frame_events(
        &mut app,
        &context,
        vec![
            ctrl_tab(false),
            modifiers(egui::Modifiers::NONE),
            modifiers(egui::Modifiers::CTRL),
            ctrl_tab(false),
        ],
    );
    assert_eq!(app.doc.tab, older, "switched once, at the release");
    let state = app.switcher.clone().expect("reopened by the later ⌃Tab");
    assert_eq!(state.order[0], Target::Tab(older));
    assert_eq!(state.order[state.selected], Target::Tab(newer));
    frame_events(&mut app, &context, vec![modifiers(egui::Modifiers::NONE)]);
    assert_eq!(app.doc.tab, newer);
    assert!(app.switcher.is_none());
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn same_frame_shortcuts_reach_the_document_active_at_their_position() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = untitled_and_titled(area.path(), "Ordered").await;
    let (untitled, titled) = (app.tabs.parked[0].tab, app.doc.tab);
    app.activate_tab(untitled);

    // Switch, then save: the save reaches the newly shown document. Pointer
    // input in a frame that switched documents reaches neither, except releases.
    let left = frame_events(
        &mut app,
        &context,
        vec![
            egui::Event::PointerMoved(egui::pos2(300.0, 200.0)),
            command(egui::Key::CloseBracket, true),
            command(egui::Key::S, false),
            button(true),
            button(false),
        ],
    );
    assert_eq!(app.doc.tab, titled);
    assert!(app.doc.documents.running, "saved where it was typed");
    assert!(!parked(&app, untitled).documents.running);
    assert_eq!(left, vec![button(false)]);
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
    })
    .await;
    assert!(app.doc.status.starts_with("Saved "));

    // Save, then switch: the save stays with the document it was typed in, and
    // holds the switch while it runs.
    frame_events(
        &mut app,
        &context,
        vec![
            command(egui::Key::S, false),
            command(egui::Key::OpenBracket, true),
        ],
    );
    assert_eq!(app.doc.tab, titled);
    assert!(app.doc.documents.running);
    assert!(!parked(&app, untitled).documents.running);
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
    })
    .await;

    // Save, then release Control over an open switcher: the release is refused
    // like ⌘⇧[, so the saving document is never parked.
    frame_events(&mut app, &context, vec![ctrl_tab(false)]);
    assert!(app.switcher.is_some());
    frame_events(
        &mut app,
        &context,
        vec![
            command(egui::Key::S, false),
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
        ],
    );
    assert_eq!(app.doc.tab, titled, "no switch while saving");
    assert!(app.switcher.is_none());
    assert!(app.doc.documents.running);
    assert_eq!(
        app.doc.status,
        "Wait for the current command before switching documents"
    );
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
    })
    .await;

    // Undo, then New: the undo is submitted to the document it was typed in,
    // before New opens and shows a tab of its own.
    frame_events(
        &mut app,
        &context,
        vec![command(egui::Key::Z, false), command(egui::Key::N, false)],
    );
    assert_eq!(app.tabs.parked.len(), 2, "New opened a third tab");
    assert!(app.doc.tab != titled && app.doc.documents.running);
    assert_eq!(
        parked(&app, titled).status,
        "Undo pending…",
        "undo reached it"
    );
    assert_ne!(app.doc.status, "Undo pending…");
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn save_all_saves_titled_in_place_and_continues_through_every_untitled() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = untitled_and_titled(area.path(), "Saved part").await;
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.tabs.parked.len() == 2 && app.document_open() && !app.busy()
    })
    .await;
    let titled = app.tabs.parked[1].tab;
    let untitled = [app.tabs.parked[0].tab, app.doc.tab];
    app.activate_tab(titled);
    app.doc.authoring_dirty = true;
    for tab in untitled {
        let doc = parked_mut(&mut app, tab);
        doc.authoring_dirty = true;
        // Each save request fails at submission, standing in for a failed save
        // without opening a native dialog.
        doc.bridge
            .shared
            .stopping
            .store(true, std::sync::atomic::Ordering::Release);
    }

    app.save_all();
    assert_eq!(app.doc.tab, untitled[1], "the last untitled one is shown");
    assert!(parked(&app, titled).documents.running, "saved in place");
    let first = parked(&app, untitled[0]);
    assert_eq!(first.status, "Native host is shutting down", "asked first");
    assert_eq!(
        app.doc.status,
        "Save All left unsaved: Untitled (Native host is shutting down), \
         Untitled (Native host is shutting down)"
    );
    assert!(app.tabs.save_all.prompted.is_none() && app.tabs.save_all.queue.is_empty());
    pump_until(&mut app, &context, &mut frame, |app| {
        !parked(app, titled).documents.running
    })
    .await;
    assert!(parked(&app, titled).status.starts_with("Saved "));
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn save_all_stops_at_a_cancelled_dialog_reports_failures_and_busy_documents() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let (first, second) = (app.tabs.parked[0].tab, app.doc.tab);
    app.activate_tab(first);
    app.doc.authoring_dirty = true;
    {
        let doc = parked_mut(&mut app, second);
        doc.authoring_dirty = true;
        doc.bridge
            .shared
            .stopping
            .store(true, std::sync::atomic::Ordering::Release);
    }
    // The first document's save dialog is open (simulated: no request was sent).
    let prompt = |app: &mut App| {
        app.doc.documents.running = true;
        app.tabs.save_all.prompted = Some(first);
        app.tabs.save_all.queue = [second].into();
    };

    let before = parked(&app, second).status.clone();
    prompt(&mut app);
    app.document_finished(Ok(Outcome::Cancelled));
    assert_eq!(app.doc.tab, first, "Cancel stops the sequence");
    assert!(app.tabs.save_all.queue.is_empty());
    assert_eq!(parked(&app, second).status, before, "never asked");

    prompt(&mut app);
    app.document_finished(Err("disk full".into()));
    assert_eq!(app.doc.tab, second, "a failure moves on to the next one");
    assert_eq!(
        app.doc.status,
        "Save All left unsaved: Untitled (disk full), Untitled (Native host is shutting down)"
    );

    // A busy document is skipped and named, never shown.
    app.activate_tab(first);
    app.doc.authoring_dirty = false;
    parked_mut(&mut app, second).documents.confirm = Some(Pending::CloseTab);
    app.save_all();
    assert_eq!(app.doc.tab, first);
    assert_eq!(app.doc.status, "Save All left unsaved: Untitled (busy)");
    parked_mut(&mut app, second).documents.confirm = None;
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn a_provisional_tab_answered_in_the_background_closes_its_host() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.document_open() && !app.busy()
    })
    .await;
    let origin = app.doc.tab;
    let missing = area.path().join("qa/documents/missing.onecad");
    app.document_request(Request::Open(Some(missing)));
    let provisional = app.doc.tab;
    assert_ne!(provisional, origin, "the open got a tab of its own");
    // The user leaves before the open answers; it fails while parked.
    app.activate_tab(origin);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.tabs.parked.is_empty() && app.tabs.all_closing_stopped()
    })
    .await;
    assert_eq!(app.doc.tab, origin);
    assert!(app.editor(provisional).is_none(), "no orphan host or tab");
    assert!(app.document_open());
    // Its failure is not lost with its tab: reported where the user is.
    let status = app.doc.status.clone();
    assert!(
        status.starts_with("Open (background tab): ") && status.len() > 23,
        "{status}"
    );
    assert!(app.doc.diagnostics.contains(
        crate::diagnostics::Source::File(crate::diagnostics::FileAction::Open),
        crate::diagnostics::State::Failed,
        &status,
    ));
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

fn saved(title: &str) -> Result<Outcome, String> {
    Ok(Outcome::Saved(onecad_app::dto::SaveOutcomeDto {
        document_id: "saved".into(),
        saved_revision: 1,
        current_revision: 1,
        clean: true,
        path: format!("/tmp/{title}.onecad"),
        title: title.into(),
    }))
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn save_all_moves_on_after_a_saved_dialog_even_when_that_document_closes() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let (first, second) = (app.tabs.parked[0].tab, app.doc.tab);
    app.activate_tab(first);
    {
        let doc = parked_mut(&mut app, second);
        doc.authoring_dirty = true;
        // Its save fails at submission instead of opening a native dialog.
        doc.bridge
            .shared
            .stopping
            .store(true, std::sync::atomic::Ordering::Release);
    }
    // The first document's dialog was answered with a location (simulated).
    let prompt = |app: &mut App| {
        app.doc.authoring_dirty = true;
        app.doc.documents.running = true;
        app.tabs.save_all.prompted = Some(first);
        app.tabs.save_all.queue = [second].into();
    };

    prompt(&mut app);
    app.document_finished(saved("First"));
    assert_eq!(app.doc.tab, second, "the next untitled one is shown");
    assert_eq!(parked(&app, first).status, "Saved First");
    assert_eq!(
        app.doc.status, "Save All left unsaved: Untitled (Native host is shutting down)",
        "and its save was requested"
    );
    assert!(app.tabs.save_all.prompted.is_none() && app.tabs.save_all.queue.is_empty());

    // Saved, then closed (Save chosen in its close prompt): Save All still moves
    // on from it rather than waiting on a tab that is gone.
    app.activate_tab(first);
    prompt(&mut app);
    app.doc.documents.after_save = Some(Pending::CloseTab);
    app.document_finished(saved("First"));
    assert!(app.editor(first).is_none(), "closed after saving");
    assert_eq!(app.doc.tab, second);
    assert!(app.tabs.save_all.prompted.is_none() && app.tabs.save_all.queue.is_empty());
    assert_eq!(
        app.doc.status,
        "Save All left unsaved: Untitled (Native host is shutting down)"
    );
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}
