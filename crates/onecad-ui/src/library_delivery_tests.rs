//! Real host delivery of the library catalog: Start page -> service lane -> store -> page.
use super::*;
use crate::ui_state::StartPage;

fn tick(app: &mut App, context: &egui::Context, frame: &mut eframe::Frame) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 820.0),
        )),
        ..Default::default()
    };
    let mut output = context.run_ui(raw, |ui| eframe::App::ui(app, ui, frame));
    output.textures_delta.clear();
    logic_tick(app, context, frame, false);
}

async fn tick_until(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    done: impl Fn(&App) -> bool,
) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            tick(app, context, frame);
            if done(app) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("library did not progress: {}", app.doc.status));
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn start_page_lists_the_seeded_library_from_the_real_service() {
    assert!(
        std::env::var_os("ONECAD_LIBRARY_ROOT").is_none(),
        "unset ONECAD_LIBRARY_ROOT: this test seeds the QA profile's own library"
    );
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_home_app(area.path()).await;
    let library = area.path().join("qa").join("data").join("library");
    onecad_app::library::seed_and_reindex_at(&library).unwrap();
    let mut frame = eframe::Frame::_new_kittest();

    app.ui.start_open = true;
    app.ui.start_page = StartPage::Library;
    tick_until(&mut app, &context, &mut frame, |app| {
        app.ui.library.catalog.loaded()
    })
    .await;
    let catalog = &app.ui.library.catalog;
    let screw = catalog
        .results()
        .iter()
        .find(|c| c.id == "onecad.std.iso4762")
        .expect("the seeded ISO 4762 entry comes from the real service");
    assert_eq!(screw.name, "Socket Head Cap Screw");
    let (id, version) = (screw.id.clone(), screw.version.clone());

    crate::library_pages::open_component(&mut app.ui, &id, &version);
    tick_until(&mut app, &context, &mut frame, |app| {
        app.ui
            .library
            .catalog
            .detail()
            .is_some_and(|view| !view.versions.is_empty())
    })
    .await;
    let view = app.ui.library.catalog.detail().unwrap();
    assert_eq!(view.versions, [version]);
    let draft = view.draft.as_ref().unwrap();
    assert_eq!(draft.designation().as_deref(), Some("ISO 4762 M6x20"));
    assert!(draft.resolve().is_ok());

    // A real preview of the default draft reaches the warm worker and reports its size.
    app.ui.library.catalog.request_preview();
    tick_until(&mut app, &context, &mut frame, |app| {
        !matches!(
            app.ui.library.catalog.preview(),
            crate::library_catalog::PreviewState::Running
        )
    })
    .await;
    match app.ui.library.catalog.preview() {
        crate::library_catalog::PreviewState::Done(summary) => {
            assert!(
                summary.size.iter().all(|extent| *extent > 0.0),
                "{summary:?}"
            );
        }
        other => panic!("preview did not complete: {other:?}"),
    }
}
