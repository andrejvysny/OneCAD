//! Several open documents in one window: each tab is its own document host.
use super::*;

pub(super) async fn two_tabs(root: &std::path::Path) -> (App, egui::Context, eframe::Frame) {
    let (mut app, context) = create_app(root).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.document_open() && app.doc.display.is_some() && !app.busy()
    })
    .await;
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.tabs.parked.len() == 1 && app.document_open() && !app.busy()
    })
    .await;
    (app, context, frame)
}

fn session(doc: &crate::document_editor::DocumentEditor) -> String {
    doc.document_state
        .as_ref()
        .map(|state| state.runtime_session.clone())
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn new_while_a_document_is_open_opens_a_second_live_tab() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let tabs = app.tab_summaries();
    assert_eq!(tabs.len(), 2);
    assert!(
        tabs[1].active && !tabs[0].active,
        "the new document is active"
    );
    assert_ne!(session(&app.doc), session(&app.tabs.parked[0]));
    let hosts = (
        app.doc
            .bridge
            .shared
            .host
            .load(std::sync::atomic::Ordering::Acquire),
        app.tabs.parked[0]
            .bridge
            .shared
            .host
            .load(std::sync::atomic::Ordering::Acquire),
    );
    assert_ne!(hosts.0, hosts.1, "one document host per tab");

    // The parked document keeps being serviced: its state stays current.
    logic_tick(&mut app, &context, &mut frame, false);
    assert!(app.tabs.parked[0].document_state.is_some());
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
    assert!(
        app.all_tabs_stopped(),
        "the window closes only after every host"
    );
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn switching_back_re_prepares_the_same_scene_without_a_new_capture() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let first = app.tabs.parked[0].tab;
    let second = app.doc.tab;
    // Simulate the first tab's scene having been uploaded before it was parked.
    let parked = &app.tabs.parked[0];
    let display = parked.display.clone().expect("first tab captured a scene");
    display.prepared.lock().unwrap().take();
    let identity = display.scene.identity();
    let context_before = parked.interaction.context.clone();

    app.activate_tab(first);
    assert_eq!(app.doc.tab, first);
    assert!(
        app.doc.awaiting_scene,
        "no image or pick until its own scene"
    );
    assert!(app.doc.view.is_none());
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc
            .display
            .as_ref()
            .is_some_and(|display| display.prepared.lock().unwrap().is_some())
    })
    .await;
    let display = app.doc.display.as_ref().unwrap();
    assert_eq!(
        display.scene.identity(),
        identity,
        "same capture, same identity"
    );
    assert_eq!(
        app.doc.interaction.context, context_before,
        "the interaction context (and with it any active tool) is untouched"
    );
    assert_eq!(
        app.tab_summaries().iter().find(|t| t.active).unwrap().tab,
        first
    );

    app.activate_tab(second);
    assert_eq!(app.doc.tab, second);
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn closing_a_tab_stops_only_its_host_and_activates_a_neighbour() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let survivor = app.tabs.parked[0].tab;
    let survivor_session = session(&app.tabs.parked[0]);
    app.close_active_tab(None).unwrap();
    assert_eq!(app.doc.tab, survivor);
    assert_eq!(app.tab_summaries().len(), 1);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.tabs.all_closing_stopped()
    })
    .await;
    assert!(!app.may_close, "closing one tab never closes the window");
    assert_eq!(
        session(&app.doc),
        survivor_session,
        "the other document is untouched"
    );
    assert!(app.document_open());
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn each_restored_crash_document_gets_its_own_tab() {
    use super::document_lifecycle_tests::recovered_fixture_named;
    let area = tempfile::tempdir().unwrap();
    let (first, _) = recovered_fixture_named(area.path(), "First part");
    let (second, _) = recovered_fixture_named(area.path(), "Second part");
    let (mut app, context) = create_home_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.bridge.shared.recovery.lock().unwrap().offers.len() == 2
    })
    .await;
    for _ in [&first, &second] {
        let offer = app.doc.bridge.shared.recovery.lock().unwrap().offers[0].clone();
        let opened = app.doc.bridge.shared.recovery.lock().unwrap().opened;
        if opened {
            crate::startup_recovery::tests_hand_offers(&mut app).unwrap();
        }
        app.doc.bridge.shared.recovery.lock().unwrap().busy = true;
        app.doc
            .bridge
            .submit(crate::bridge::Command::Recovery {
                document_id: offer.document_id,
                choice: crate::startup_recovery::Choice::Restore,
            })
            .unwrap();
        pump_until(&mut app, &context, &mut frame, |app| {
            app.document_open() && !app.busy()
        })
        .await;
    }
    let ids: Vec<String> = std::iter::once(&app.doc)
        .chain(&app.tabs.parked)
        .filter_map(|doc| doc.document_state.as_ref().map(|s| s.document_id.clone()))
        .collect();
    assert_eq!(ids.len(), 2, "two tabs, one restored document each");
    assert!(ids.contains(&first) && ids.contains(&second));
    assert!(
        app.tab_summaries().iter().all(|tab| tab.dirty),
        "restored work stays unsaved"
    );
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn quit_asks_about_every_document_with_unsaved_work() {
    use crate::document_flow::{Pending, apply_close_choice};
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let parked = app.tabs.parked[0].tab;
    app.tabs.parked[0].authoring_dirty = true;
    // The active document is clean: quit brings the dirty one forward first.
    logic_tick(&mut app, &context, &mut frame, true);
    assert_eq!(app.doc.tab, parked, "the undecided document is shown");
    assert_eq!(app.doc.documents.confirm, Some(Pending::Quit));
    // Both now hold unsaved work: discarding one must ask about the other.
    app.tabs.parked[0].authoring_dirty = true;
    apply_close_choice(&mut app, Some(1));
    assert!(
        !app.stopping,
        "discarding one never stops the others unasked"
    );
    assert_eq!(app.tab_summaries().len(), 1);
    logic_tick(&mut app, &context, &mut frame, false);
    assert_eq!(
        app.doc.documents.confirm,
        Some(Pending::Quit),
        "asked again"
    );
    apply_close_choice(&mut app, Some(1));
    assert!(app.stopping);
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn a_close_decided_in_the_background_waits_for_its_tab() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let (first, second) = (app.tabs.parked[0].tab, app.doc.tab);
    // A parked document finishing "Save, then close" must not reshape the tabs
    // while its messages are being applied.
    app.parked = true;
    app.close_active_tab(None).unwrap();
    app.parked = false;
    assert_eq!(app.tab_summaries().len(), 2, "nothing closed while parked");
    assert_eq!(app.doc.tab, second);
    app.doc.close_when_active = None;
    app.tabs.parked[0].close_when_active = Some(None);
    app.activate_tab(first);
    assert_eq!(app.tab_summaries().len(), 1, "applied once shown again");
    assert_eq!(app.doc.tab, second);
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn tabs_keep_opening_order_and_cycle_through_it() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let first = app.tabs.parked[0].tab;
    app.activate_tab(first);
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.tabs.parked.len() == 2 && app.document_open() && !app.busy()
    })
    .await;
    let order: Vec<_> = app.tab_summaries().iter().map(|tab| tab.tab).collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(order, sorted, "tab order is opening order");
    let newest = *order.last().unwrap();
    assert_eq!(app.doc.tab, newest);
    app.cycle_tab(-1);
    assert_eq!(app.doc.tab, order[1], "previous tab is the left-hand one");
    app.cycle_tab(2);
    assert_eq!(app.doc.tab, order[0], "cycling wraps");
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn an_unpreparable_switch_falls_back_instead_of_blanking_forever() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let first = app.tabs.parked[0].tab;
    let parked = &app.tabs.parked[0];
    parked
        .display
        .as_ref()
        .unwrap()
        .prepared
        .lock()
        .unwrap()
        .take();
    // Its service refuses new work: the re-prepare cannot even be submitted.
    parked
        .bridge
        .shared
        .stopping
        .store(true, std::sync::atomic::Ordering::Release);
    app.activate_tab(first);
    assert!(
        !app.doc.awaiting_scene,
        "no wait on a request that never left"
    );
    assert!(
        app.doc.display.is_none(),
        "the undrawable capture is dropped"
    );
    assert!(app.doc.status.starts_with("Document display:"));
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

pub(super) fn ctrl_tab(shift: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            shift,
            ..Default::default()
        },
    }
}

pub(super) fn raw(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        events,
        focused: true,
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn ctrl_tab_walks_most_recent_first_and_switches_on_release() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let (older, newer) = (app.tabs.parked[0].tab, app.doc.tab);
    assert_eq!(app.recent_tab_order(), vec![newer, older]);

    let mut input = raw(vec![ctrl_tab(false)]);
    crate::tab_switcher::route(&mut app, &mut input);
    assert!(
        input.events.is_empty(),
        "⌃Tab never reaches egui focus navigation"
    );
    let state = app.switcher.clone().expect("switcher open");
    assert_eq!(
        state.order,
        vec![
            crate::tab_switcher::Target::Tab(newer),
            crate::tab_switcher::Target::Tab(older),
            crate::tab_switcher::Target::Home
        ],
        "most recent first, Home last"
    );
    assert_eq!(
        state.selected, 1,
        "the first press moves off the current document"
    );

    let mut release = raw(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    crate::tab_switcher::route(&mut app, &mut release);
    assert!(app.switcher.is_none());
    assert_eq!(app.doc.tab, older, "release switches");
    assert_eq!(app.recent_tab_order(), vec![older, newer]);

    // Escape abandons the switch.
    crate::tab_switcher::route(&mut app, &mut raw(vec![ctrl_tab(false)]));
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
    };
    crate::tab_switcher::route(&mut app, &mut raw(vec![escape]));
    assert!(app.switcher.is_none());
    assert_eq!(app.doc.tab, older);

    // ⇧⌃Tab from a document reaches Home (last) directly.
    crate::tab_switcher::route(&mut app, &mut raw(vec![ctrl_tab(true)]));
    crate::tab_switcher::route(
        &mut app,
        &mut raw(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]),
    );
    assert!(app.ui.start_open, "Home in focus");
    assert_eq!(app.doc.tab, older, "Home keeps the last document loaded");
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn new_tab_shows_home_and_the_close_prompt_asks_what_changed() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, mut frame) = two_tabs(area.path()).await;
    let active = app.doc.tab;
    app.open_home();
    assert!(
        app.ui.start_open && app.doc.tab == active,
        "⌘T creates nothing"
    );
    app.show_tab(active);
    assert!(!app.ui.start_open);

    app.doc.authoring_dirty = true;
    app.request_close_tab();
    assert!(matches!(
        app.doc.documents.confirm,
        Some(crate::document_flow::Pending::CloseTab)
    ));
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.close_changes.is_some()
    })
    .await;
    assert_eq!(
        crate::documents::changes_text(app.doc.close_changes.as_ref()),
        "Your unsaved changes will be lost if you don't save them.",
        "work the backend cannot name is still warned about"
    );
    crate::document_flow::apply_close_choice(&mut app, Some(2));
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}
