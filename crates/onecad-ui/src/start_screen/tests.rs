use super::*;

#[test]
fn home_actions_have_real_intents_except_assembly() {
    let specs = tile_specs();
    assert_eq!(specs[0].request, Some(Request::New));
    assert_eq!(specs[1].request, None);
    assert_eq!(specs.len(), 3);
    assert_eq!(specs[2].title, "Open or import");
    assert_eq!(specs[2].subtitle, ".onecad and STEP");
    assert_eq!(specs[2].request, Some(Request::Import));
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
