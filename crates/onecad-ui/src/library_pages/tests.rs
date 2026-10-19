use super::{
    data::{COMPONENTS, columns, sections},
    sheet::SheetState,
    standard_data::*,
};
use crate::ui_state::{StartPage, UiState};

#[test]
fn source_tabs_filter_the_grid() {
    let all = sections(0, 0, "");
    assert_eq!(all.recent, vec![0, 1, 2, 3]);
    assert!(all.tiles);
    assert!(all.own.iter().all(|i| !all.recent.contains(i)));
    let mine = sections(1, 0, "");
    assert!(mine.recent.is_empty() && !mine.tiles);
    assert!(mine.own.contains(&0) && mine.own.contains(&7));
    let project = sections(2, 0, "");
    assert_eq!(project.own_title, "This project");
    assert!(
        project
            .own
            .iter()
            .all(|i| COMPONENTS[*i].meta.starts_with("This project"))
    );
    let standard = sections(3, 0, "");
    assert!(standard.tiles && standard.recent.is_empty());
}

#[test]
fn type_tabs_and_search_narrow_cards() {
    let parametric = sections(1, 1, "");
    assert!(
        parametric
            .own
            .iter()
            .all(|i| COMPONENTS[*i].name != "Cable clip 6 mm")
    );
    let fixed = sections(1, 2, "");
    assert_eq!(fixed.own.len(), 1);
    assert!(!sections(0, 2, "").tiles);
    assert_eq!(sections(1, 0, "hinge").own, vec![5]);
    assert!(sections(1, 0, "zzz").own.is_empty());
}

#[test]
fn grid_columns_follow_width() {
    assert_eq!(columns(100.0, 200.0, 16.0), 1);
    assert_eq!(columns(900.0, 200.0, 16.0), 4);
    assert_eq!(columns(640.0, 200.0, 16.0), 3);
}

#[test]
fn opening_cards_routes_by_source() {
    let mut state = UiState::default();
    super::browser::open_component(&mut state, 4);
    assert_eq!(state.start_page, StartPage::ComponentDetail);
    assert_eq!(state.library.selected, 4);
    super::browser::open_component(&mut state, 1);
    assert_eq!(state.start_page, StartPage::StandardPart);
    super::go_library(&mut state, 2);
    assert_eq!(
        (state.start_page, state.library.source),
        (StartPage::Library, 2)
    );
}

#[test]
fn preset_edits_switch_to_custom() {
    let mut s = SheetState::from_preset(1);
    assert_eq!(s.summary(), "60 × 40 × 4 mm · 31.8 g");
    s.set_width(72.0, false);
    assert_eq!(s.preset, 3);
    assert_eq!(s.summary(), "72 × 40 × 4 mm · 38.2 g");
    assert_eq!(s.changed(), vec!["width changed"]);
    s.apply_preset(2);
    assert_eq!((s.width, s.height, s.thickness), (90.0, 60.0, 6.0));
    s.apply_preset(3);
    assert_eq!(s.width, 90.0);
    s.set_width(500.0, true);
    assert_eq!(s.width, 120.0);
}

#[test]
fn mock_mass_matches_design_values() {
    let mut s = SheetState::from_preset(1);
    s.set_width(72.0, false);
    assert_eq!(format!("{:.1}", s.mass_g()), "38.2");
    assert_eq!(format!("{:.1}", mass_g(3, 12, 1)), "1.9");
}

#[test]
fn standard_part_text_follows_selection() {
    let mut std = StdState::default();
    assert_eq!(
        part_code(std.family, std.size, std.length),
        "ISO 4762 M4×12"
    );
    assert_eq!(
        part_summary(std.size, std.length, std.material, false),
        "A2 stainless · cosmetic thread · 1.9 g"
    );
    assert_eq!(derived(std.size)[1].1, "Ø 8 × 4.4 mm");
    assert_eq!(derived(3)[0].1, "Ø 4.5 mm");
    assert_eq!(derived(3)[2].1, "Ø 3.3 mm");
    assert_eq!(
        lengths_for(3),
        vec![5, 6, 8, 10, 12, 16, 20, 25, 30, 35, 40, 45]
    );
    std.select_size(0);
    assert_eq!(std.length, 12);
    std.length = 45;
    std.select_size(1);
    assert_eq!(std.length, 25);
    assert_eq!(part_code(0, 0, 3), "ISO 4762 M2×3");
}

fn render(state: &mut UiState, dark: bool, width: f32) {
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
    for _ in 0..2 {
        ctx.run_ui(raw.clone(), |ui| {
            super::show_state(state, ui);
            crate::component_wizard::show_state(state, ui.ctx());
        })
        .textures_delta
        .clear();
    }
}

#[test]
fn every_library_page_renders_in_both_themes_and_widths() {
    for dark in [false, true] {
        for width in [760.0, 1200.0] {
            let mut state = UiState {
                start_page: StartPage::Library,
                ..UiState::default()
            };
            for source in [0, 2, 3] {
                state.library.source = source;
                render(&mut state, dark, width);
            }
            state.start_page = StartPage::ComponentDetail;
            for tab in 0..4 {
                state.library.detail_tab = tab;
                render(&mut state, dark, width);
            }
            state.start_page = StartPage::StandardPart;
            render(&mut state, dark, width);
            state.start_page = StartPage::ComponentDetail;
            super::sheet::open(&mut state.library);
            render(&mut state, dark, width);
            state.library.sheet = None;
            for step in 0..4 {
                crate::component_wizard::open(&mut state);
                if let Some(w) = state.library.wizard.as_mut() {
                    w.step = step;
                    w.saved = step == 3;
                }
                render(&mut state, dark, width);
            }
        }
    }
}
