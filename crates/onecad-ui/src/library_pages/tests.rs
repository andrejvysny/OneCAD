use super::browser::{free_count, icon_for, meta_line, source_label};
use crate::{
    library_catalog::{
        Reply, Source,
        testing::{component, iso_specs, load, open},
    },
    ui_state::{StartPage, UiState},
};
use std::collections::BTreeMap;

#[test]
fn grid_columns_follow_width() {
    assert_eq!(super::kit::columns(100.0, 200.0, 16.0), 1);
    assert_eq!(super::kit::columns(900.0, 200.0, 16.0), 4);
    assert_eq!(super::kit::columns(640.0, 200.0, 16.0), 3);
}

#[test]
fn card_text_comes_from_the_catalog_entry() {
    let c = component("onecad.std.iso4762", "1.0.0", iso_specs());
    assert_eq!(meta_line(&c), "ISO 4762 · v1.0.0");
    assert_eq!(free_count(&c), 3);
    assert_eq!(source_label(&c.source_kind), "Generated parts");
    assert_eq!(source_label("unlisted"), "unlisted");
    assert_eq!(icon_for(&c), "thread");
    let mut plain = component("x", "2.0.0", BTreeMap::new());
    plain.standard = None;
    plain.designation = None;
    plain.category.clear();
    assert_eq!(meta_line(&plain), "x · v2.0.0");
    assert_eq!(free_count(&plain), 0);
    assert_eq!(icon_for(&plain), "body");
}

#[test]
fn opening_a_card_selects_it_and_returns_to_the_browser() {
    let mut state = UiState::default();
    load(
        &mut state.library.catalog,
        vec![component("a", "1.0.0", iso_specs())],
    );
    state.library.detail_tab = 2;
    super::open_component(&mut state, "a", "1.0.0");
    assert_eq!(state.start_page, StartPage::ComponentDetail);
    assert_eq!(state.library.detail_tab, 0);
    let view = state.library.catalog.detail().expect("selected");
    assert_eq!((view.id.as_str(), view.version.as_str()), ("a", "1.0.0"));
    assert!(
        view.draft.is_some(),
        "the search hit fills the page at once"
    );
    super::go_library(&mut state);
    assert_eq!(state.start_page, StartPage::Library);
}

/// All text the frame painted, for asserting what the page says.
fn texts(output: &egui::FullOutput) -> String {
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
    let mut out = String::new();
    output.shapes.iter().for_each(|s| walk(&s.shape, &mut out));
    out
}

fn render(state: &mut UiState, dark: bool, width: f32) -> String {
    let ctx = egui::Context::default();
    onecad_widgets::fonts::install(&ctx);
    if let Ok(t) = onecad_widgets::theme::load(dark) {
        onecad_widgets::theme::install(&ctx, t, dark);
    }
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, 760.0),
        )),
        ..Default::default()
    };
    let mut painted = String::new();
    for _ in 0..2 {
        let mut output = ctx.run_ui(raw.clone(), |ui| {
            super::show_state(state, ui);
        });
        painted = texts(&output);
        output.textures_delta.clear();
    }
    painted
}

fn loaded_state() -> UiState {
    let mut state = UiState {
        start_page: StartPage::Library,
        ..UiState::default()
    };
    let screw = component("onecad.std.iso4762", "1.0.0", iso_specs());
    let mut other = component("other", "2.1.0", BTreeMap::new());
    other.name = "Bracket".into();
    other.source_kind = "embedded".into();
    other.generator_id = None;
    load(&mut state.library.catalog, vec![screw, other]);
    state
}

#[test]
fn browser_reports_the_real_count_and_no_mock_content() {
    let mut state = loaded_state();
    let page = render(&mut state, false, 1200.0);
    assert!(page.contains("2 components"), "{page}");
    assert!(page.contains("Part onecad.std.iso4762") && page.contains("Bracket"));
    for mock in [
        "128 components",
        "Mounting bracket",
        "Pi 5 standoff",
        "109 standard parts",
    ] {
        assert!(!page.contains(mock), "{mock} is mock content");
    }
    // The placeholder action says why it is off instead of pretending.
    assert!(page.contains("New component"));
}

#[test]
fn browser_states_are_honest() {
    let mut loading = UiState::default();
    loading.library.catalog.prepare();
    assert!(render(&mut loading, false, 1000.0).contains("Loading components…"));

    let mut empty = UiState::default();
    load(&mut empty.library.catalog, vec![]);
    let page = render(&mut empty, false, 1000.0);
    assert!(page.contains("The component library is empty."), "{page}");
    assert!(page.contains("0 components"));

    let mut filtered = UiState::default();
    filtered.library.catalog.query.text = "zzz".into();
    load(&mut filtered.library.catalog, vec![]);
    let page = render(&mut filtered, false, 1000.0);
    assert!(
        page.contains("No components match these filters."),
        "{page}"
    );
    assert!(page.contains("Clear filters"));

    let mut failed = UiState::default();
    failed.library.catalog.prepare();
    let sequence = failed.library.catalog.take_outbox()[0].sequence();
    failed.library.catalog.accept(
        sequence,
        Err("index.json unreadable".into()),
        &egui::Context::default(),
    );
    let page = render(&mut failed, false, 1000.0);
    assert!(
        page.contains("The component library could not be read."),
        "{page}"
    );
    assert!(page.contains("index.json unreadable"));
}

#[test]
fn detail_shows_the_component_and_never_claims_a_placement() {
    let mut state = loaded_state();
    super::open_component(&mut state, "onecad.std.iso4762", "1.0.0");
    let page = render(&mut state, false, 1200.0);
    assert!(page.contains("Part onecad.std.iso4762"));
    assert!(page.contains("ISO 4762 M6x20"), "live designation: {page}");
    for control in [
        "Insert into open model",
        "Edit source",
        "Duplicate",
        "Standard table",
    ] {
        assert!(page.contains(control), "{control}");
    }
    for fake in ["Inserted", "preview only", "Duplicated to", "added to"] {
        assert!(!page.contains(fake), "{fake}");
    }
    for key in ["thread", "length", "thread_detail", "free"] {
        assert!(page.contains(key), "{key}");
    }
}

#[test]
fn an_edited_invalid_draft_is_explained_on_the_page() {
    let mut state = loaded_state();
    open(
        &mut state.library.catalog,
        component("onecad.std.iso4762", "1.0.0", iso_specs()),
    );
    state.start_page = StartPage::ComponentDetail;
    let draft = state
        .library
        .catalog
        .detail_mut()
        .and_then(|v| v.draft.as_mut())
        .expect("draft");
    draft.set_text("length", "1".into());
    draft.set_text("thread", "M8".into());
    let page = render(&mut state, false, 1200.0);
    assert!(page.contains("Must be at least 2"), "{page}");
    assert!(page.contains("ISO 4762 M8x{length}"), "{page}");
    assert!(page.contains("incomplete until the highlighted fields are fixed"));
}

#[test]
fn the_iso_table_page_is_only_for_iso4762_components() {
    let mut state = loaded_state();
    super::open_component(&mut state, "onecad.std.iso4762", "1.0.0");
    state.start_page = StartPage::StandardPart;
    let page = render(&mut state, false, 1200.0);
    for row in ["M2", "M12", "1.75", "ISO 4762 · socket head cap screws"] {
        assert!(page.contains(row), "{row}");
    }
    assert_eq!(state.start_page, StartPage::StandardPart);

    let mut bracket = loaded_state();
    open(&mut bracket.library.catalog, {
        let mut c = component("other", "2.1.0", BTreeMap::new());
        c.generator_id = None;
        c
    });
    bracket.start_page = StartPage::StandardPart;
    render(&mut bracket, false, 1200.0);
    assert_eq!(bracket.start_page, StartPage::ComponentDetail);

    let mut nothing = UiState {
        start_page: StartPage::StandardPart,
        ..UiState::default()
    };
    render(&mut nothing, false, 1200.0);
    assert_eq!(nothing.start_page, StartPage::Library);
}

#[test]
fn the_table_row_sets_the_drafts_thread() {
    let mut state = loaded_state();
    super::open_component(&mut state, "onecad.std.iso4762", "1.0.0");
    super::standard::set_thread(&mut state.library.catalog, "M10");
    let draft = state
        .library
        .catalog
        .detail()
        .and_then(|v| v.draft.as_ref())
        .expect("draft");
    assert_eq!(draft.designation().as_deref(), Some("ISO 4762 M10x20"));
}

#[test]
fn every_library_page_renders_in_both_themes_and_widths() {
    for dark in [false, true] {
        for width in [760.0, 1200.0] {
            let mut state = loaded_state();
            state.library.catalog.query.source = Source::Generator;
            render(&mut state, dark, width);
            super::open_component(&mut state, "onecad.std.iso4762", "1.0.0");
            for tab in 0..4 {
                state.library.detail_tab = tab;
                render(&mut state, dark, width);
            }
            state.start_page = StartPage::StandardPart;
            render(&mut state, dark, width);
        }
    }
}

#[test]
fn unexpected_reply_kinds_fail_instead_of_corrupting_state() {
    let mut state = UiState::default();
    state.library.catalog.prepare();
    let sequence = state.library.catalog.take_outbox()[0].sequence();
    let reply = Reply::Reindexed(onecad_app::dto::ReindexReportDto {
        total: 0,
        indexed: 0,
        skipped: vec![],
    });
    state
        .library
        .catalog
        .accept(sequence, Ok(reply), &egui::Context::default());
    let page = render(&mut state, false, 1000.0);
    assert!(page.contains("unexpected library reply"), "{page}");
}

#[test]
fn a_failed_detail_load_shows_its_error_instead_of_loading_text() {
    let mut state = loaded_state();
    super::open_component(&mut state, "onecad.std.iso4762", "1.0.0");
    let sequence = state.library.catalog.take_outbox()[0].sequence();
    state.library.catalog.accept(
        sequence,
        Err("component.toml unreadable".into()),
        &egui::Context::default(),
    );
    state.library.detail_tab = 1;
    let page = render(&mut state, false, 1200.0);
    assert!(
        page.contains("Versions unavailable: component.toml unreadable"),
        "{page}"
    );
    assert!(!page.contains("Version list is loading"));
    assert!(page.contains("Could not load full details"));
    assert!(page.contains("No preview image in this package."));
    assert!(!page.contains("Try parameters to build"));
}

#[test]
fn insert_needs_an_open_editor_and_a_valid_draft() {
    let mut state = loaded_state();
    open(
        &mut state.library.catalog,
        component("onecad.std.iso4762", "1.0.0", iso_specs()),
    );
    assert_eq!(
        super::insert_blocker(&state.library),
        Some("Open a model first")
    );
    state.library.insert_gate = super::InsertGate(None);
    assert_eq!(super::insert_blocker(&state.library), None);
    state
        .library
        .catalog
        .detail_mut()
        .and_then(|view| view.draft.as_mut())
        .unwrap()
        .set_text("thread", "M99".into());
    assert_eq!(
        super::insert_blocker(&state.library),
        Some("Fix the invalid parameters first")
    );
}
