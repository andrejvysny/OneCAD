use super::*;

#[test]
fn home_actions_have_real_intents() {
    let specs = tile_specs();
    assert_eq!(specs[0].request, Some(Request::New));
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[1].title, "Open or import");
    assert_eq!(specs[1].subtitle, ".onecad and STEP · ⌘O");
    assert_eq!(specs[1].request, Some(Request::Import));
}

#[test]
fn recent_metadata_reports_real_timestamp_and_recovery() {
    let mut recent = RecentProjectDto {
        id: "document".into(),
        name: "Actual part".into(),
        path: "/tmp/actual.onecad".into(),
        modified_at: "2026-10-20T12:00:00Z".into(),
        thumbnail: None,
        has_recovery: false,
    };
    assert_eq!(recent_metadata(&recent), recent.modified_at);
    recent.has_recovery = true;
    assert_eq!(
        recent_metadata(&recent),
        "Recovery available; saved file is older"
    );
}

#[test]
fn sidebar_leaves_content_room_at_minimum_window() {
    assert_eq!(sidebar_width(800.0), 200.0);
    assert_eq!(sidebar_width(1440.0), SIDEBAR_W);
}

#[test]
fn three_tiles_fit_minimum_window_with_narrow_fallback() {
    let content_width = 900.0 - sidebar_width(900.0) - 2.0 * MAIN_PAD_X;
    assert_eq!(tile_columns(content_width), 3);
    assert_eq!(tile_columns(400.0), 2);
    assert_eq!(tile_columns(300.0), 1);
}

#[test]
fn sidebar_navigation_routes_pages() {
    let mut state = crate::ui_state::UiState::default();
    navigate(&mut state, Nav::Library);
    assert_eq!(state.start_page, StartPage::Library);
    assert!(library_active(state.start_page));
    navigate(&mut state, Nav::Sub(0));
    assert_eq!(
        (state.start_page, state.library.source),
        (StartPage::Library, 1)
    );
    assert!(sub_active(&state, 0) && !sub_active(&state, 1));
    navigate(&mut state, Nav::Sub(1));
    assert_eq!(state.library.source, 2);
    assert!(sub_active(&state, 1));
    navigate(&mut state, Nav::Sub(2));
    assert_eq!(state.start_page, StartPage::StandardPart);
    assert!(sub_active(&state, 2) && library_active(state.start_page));
    navigate(&mut state, Nav::Extensions);
    assert_eq!(state.start_page, StartPage::Extensions);
    assert!(!library_active(state.start_page));
    navigate(&mut state, Nav::Home);
    assert_eq!(state.start_page, StartPage::Home);
}

#[test]
fn component_library_resets_source_tab() {
    let mut state = crate::ui_state::UiState::default();
    state.library.source = 3;
    navigate(&mut state, Nav::Library);
    assert_eq!(state.library.source, 0);
}

#[test]
fn brand_symbol_decodes_for_both_themes() {
    let ctx = egui::Context::default();
    for dark in [false, true] {
        let texture = brand_symbol(&ctx, dark).expect("embedded brand symbol decodes");
        assert_eq!(texture.size(), [56, 56]);
    }
}

/// The brand decode left eframe; desktop pixels must not change.
#[cfg(feature = "desktop")]
#[test]
fn brand_decode_matches_the_eframe_decoder() {
    for png in [
        &include_bytes!("../../../../assets/native/brand/png/symbol-dark-56.png")[..],
        &include_bytes!("../../../../assets/native/brand/png/symbol-56.png")[..],
    ] {
        let reference = eframe::icon_data::from_png_bytes(png).unwrap();
        let (size, rgba) = decode_rgba8(png).unwrap();
        assert_eq!(size, [reference.width as usize, reference.height as usize]);
        assert_eq!(rgba, reference.rgba);
    }
}
