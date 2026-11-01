use super::recording::{Filter, ToolRow, tool_rows};
use super::*;
use crate::{
    shortcut_prefs::{KeyContext, ShortcutOverride},
    tool_registry::{ToolId, effective_key},
};
use egui::{Event, Key, Modifiers};

fn press(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn key_of(overrides: &[ShortcutOverride], tool: ToolId, context: KeyContext) -> Option<Key> {
    effective_key(overrides, tool, context)
}

#[test]
fn recording_a_free_key_assigns_and_stops() {
    let (mut ui, mut overrides) = (ShortcutsUi::default(), Vec::new());
    ui.start(KeyContext::Modeling, ToolId::Chamfer);
    assert!(ui.capturing());
    ui.keys(&mut overrides, &[press(Key::K, Modifiers::NONE)]);
    assert_eq!(
        key_of(&overrides, ToolId::Chamfer, KeyContext::Modeling),
        Some(Key::K)
    );
    assert!(!ui.capturing() && ui.pending.is_none());
}

#[test]
fn recording_a_taken_key_waits_for_confirmation_then_unbinds_the_loser() {
    let (mut ui, mut overrides) = (ShortcutsUi::default(), Vec::new());
    ui.start(KeyContext::Modeling, ToolId::Chamfer);
    ui.keys(&mut overrides, &[press(Key::H, Modifiers::NONE)]);
    let pending = ui.pending.expect("H is held by Hole");
    assert_eq!(pending.loser, ToolId::Hole);
    assert_eq!(pending.tool, ToolId::Chamfer);
    assert!(overrides.is_empty(), "nothing changes before confirming");
    assert!(ui.capturing());
    ui.confirm(&mut overrides);
    assert_eq!(
        key_of(&overrides, ToolId::Chamfer, KeyContext::Modeling),
        Some(Key::H)
    );
    assert_eq!(key_of(&overrides, ToolId::Hole, KeyContext::Modeling), None);
    assert!(!ui.capturing() && ui.pending.is_none());
}

#[test]
fn escape_cancels_recording_and_a_pending_conflict() {
    let (mut ui, mut overrides) = (ShortcutsUi::default(), Vec::new());
    ui.start(KeyContext::Modeling, ToolId::Chamfer);
    ui.keys(&mut overrides, &[press(Key::Escape, Modifiers::NONE)]);
    assert!(!ui.capturing() && overrides.is_empty());
    ui.start(KeyContext::Modeling, ToolId::Chamfer);
    ui.keys(&mut overrides, &[press(Key::H, Modifiers::NONE)]);
    assert!(ui.pending.is_some());
    ui.keys(&mut overrides, &[press(Key::Escape, Modifiers::NONE)]);
    assert!(!ui.capturing() && ui.pending.is_none() && overrides.is_empty());
}

#[test]
fn shifted_repeated_released_and_unrecordable_keys_are_ignored() {
    let (mut ui, mut overrides) = (ShortcutsUi::default(), Vec::new());
    ui.start(KeyContext::Sketch, ToolId::Line);
    let raw = |pressed, repeat| Event::Key {
        key: Key::K,
        physical_key: None,
        pressed,
        repeat,
        modifiers: Modifiers::NONE,
    };
    let (repeat, release) = (raw(true, true), raw(false, false));
    ui.keys(
        &mut overrides,
        &[
            press(Key::K, Modifiers::SHIFT),
            press(Key::Space, Modifiers::NONE),
            press(Key::Comma, Modifiers::NONE),
            repeat,
            release,
        ],
    );
    assert!(ui.capturing() && ui.pending.is_none() && overrides.is_empty());
}

#[test]
fn a_new_free_key_after_a_conflict_assigns_it() {
    let (mut ui, mut overrides) = (ShortcutsUi::default(), Vec::new());
    ui.start(KeyContext::Modeling, ToolId::Chamfer);
    ui.keys(
        &mut overrides,
        &[
            press(Key::H, Modifiers::NONE),
            press(Key::K, Modifiers::NONE),
        ],
    );
    assert_eq!(
        key_of(&overrides, ToolId::Chamfer, KeyContext::Modeling),
        Some(Key::K)
    );
    assert_eq!(
        key_of(&overrides, ToolId::Hole, KeyContext::Modeling),
        Some(Key::H)
    );
}

#[test]
fn keys_are_ignored_when_nothing_is_recording() {
    let (mut ui, mut overrides) = (ShortcutsUi::default(), Vec::new());
    ui.keys(&mut overrides, &[press(Key::K, Modifiers::NONE)]);
    assert!(overrides.is_empty());
}

#[test]
fn category_search_filters_enabled_pages_by_label_and_row_keywords() {
    assert!(Category::General.matches(""));
    assert!(Category::General.matches("  THEME "));
    assert!(!Category::Navigation.matches("theme"));
    assert!(Category::Navigation.matches("Invert"));
    assert!(Category::Shortcuts.matches("keyboard"));
    assert!(Category::Viewport.matches("view"));
    assert!(!Category::Shortcuts.matches("decimals"));
    assert!(Category::General.available() && Category::Shortcuts.available());
    assert!(!Category::Files.available() && !Category::Advanced.available());
}

#[test]
fn every_shortcut_row_has_a_registry_icon_and_label() {
    let rows = tool_rows();
    assert_eq!(rows.len(), 16);
    for row in rows {
        let def = crate::tool_registry::definition(row.tool)
            .unwrap_or_else(|| panic!("{:?} has no definition", row.tool));
        assert!(onecad_widgets::icon::exists(def.icon), "{}", def.icon);
    }
}

#[test]
fn shortcut_filter_and_search_select_rows() {
    let chamfer = ToolRow {
        context: KeyContext::Modeling,
        tool: ToolId::Chamfer,
    };
    let line = ToolRow {
        context: KeyContext::Sketch,
        tool: ToolId::Line,
    };
    let none: &[ShortcutOverride] = &[];
    assert!(shortcuts::tool_visible(chamfer, Filter::All, "", none));
    assert!(shortcuts::tool_visible(chamfer, Filter::Modeling, "", none));
    assert!(!shortcuts::tool_visible(chamfer, Filter::Sketch, "", none));
    assert!(shortcuts::tool_visible(line, Filter::Sketch, "", none));
    assert!(!shortcuts::tool_visible(line, Filter::View, "", none));
    assert!(!shortcuts::tool_visible(chamfer, Filter::Changed, "", none));
    assert!(shortcuts::tool_visible(chamfer, Filter::All, "cham", none));
    assert!(shortcuts::tool_visible(chamfer, Filter::All, "c", none));
    assert!(!shortcuts::tool_visible(chamfer, Filter::All, "zzz", none));
    let remapped = [ShortcutOverride {
        context: KeyContext::Modeling,
        tool: ToolId::Chamfer,
        key: Some("K".into()),
    }];
    assert!(shortcuts::tool_visible(
        chamfer,
        Filter::Changed,
        "",
        &remapped
    ));
    assert!(shortcuts::tool_visible(
        chamfer,
        Filter::All,
        "k",
        &remapped
    ));
}

fn headless(state: &mut UiState, dark: bool, width: f32) {
    let ctx = egui::Context::default();
    onecad_widgets::fonts::install(&ctx);
    if let Ok(t) = onecad_widgets::theme::load(dark) {
        onecad_widgets::theme::install(&ctx, t, dark);
    }
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 900.0))),
        ..Default::default()
    };
    for _ in 0..2 {
        ctx.run_ui(raw.clone(), |ui| {
            content(ui, state);
        })
        .textures_delta
        .clear();
    }
}

#[test]
fn every_page_renders_in_both_themes_including_a_pending_conflict() {
    for dark in [false, true] {
        for category in [Category::General, Category::Navigation, Category::Shortcuts] {
            let mut state = UiState::default();
            state.settings.category = category;
            headless(&mut state, dark, 900.0);
            headless(&mut state, dark, 520.0);
        }
        let mut state = UiState::default();
        state.settings.category = Category::Shortcuts;
        let shortcuts = &mut state.settings.shortcuts;
        shortcuts.start(KeyContext::Modeling, ToolId::Chamfer);
        shortcuts.keys(
            &mut state.shortcut_overrides,
            &[press(Key::H, Modifiers::NONE)],
        );
        assert!(state.settings.shortcuts.pending.is_some());
        headless(&mut state, dark, 900.0);
    }
}

#[test]
fn command_control_and_option_chords_stop_recording_but_shift_is_ignored() {
    for modifiers in [Modifiers::COMMAND, Modifiers::CTRL, Modifiers::ALT] {
        let (mut ui, mut overrides) = (ShortcutsUi::default(), Vec::new());
        ui.start(KeyContext::Modeling, ToolId::Chamfer);
        ui.keys(&mut overrides, &[press(Key::K, modifiers)]);
        assert!(!ui.capturing() && overrides.is_empty(), "{modifiers:?}");
    }
    let (mut ui, mut overrides) = (ShortcutsUi::default(), Vec::new());
    ui.start(KeyContext::Modeling, ToolId::Chamfer);
    ui.keys(&mut overrides, &[press(Key::K, Modifiers::SHIFT)]);
    assert!(ui.capturing() && overrides.is_empty());
}

fn recording_state() -> UiState {
    let mut state = UiState {
        start_open: true,
        start_page: StartPage::Settings,
        ..UiState::default()
    };
    state.settings.category = Category::Shortcuts;
    state
        .settings
        .shortcuts
        .start(KeyContext::Modeling, ToolId::Chamfer);
    state
}

#[test]
fn recording_continues_only_while_its_row_is_visible_on_the_shortcuts_page() {
    let mut state = recording_state();
    assert!(recording_allowed(&state, false, false));
    assert!(!recording_allowed(&state, true, false), "text focus");
    assert!(!recording_allowed(&state, false, true), "open dropdown");
    state.extensions.native.palette_open = true;
    assert!(!recording_allowed(&state, false, false), "palette");
    state.extensions.native.palette_open = false;
    state.settings.shortcuts.filter = Filter::Sketch;
    assert!(!recording_allowed(&state, false, false), "filtered out");
    state.settings.shortcuts.filter = Filter::All;
    state.settings.shortcuts.search = "zzz".into();
    assert!(!recording_allowed(&state, false, false), "searched out");
    state.settings.shortcuts.search.clear();
    state.settings.category = Category::General;
    assert!(!recording_allowed(&state, false, false), "other category");
    state.settings.category = Category::Shortcuts;
    state.start_page = StartPage::Home;
    assert!(!recording_allowed(&state, false, false), "other page");
    state.start_page = StartPage::Settings;
    state.start_open = false;
    assert!(!recording_allowed(&state, false, false), "document shown");
}

fn text_rects(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect)> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::epaint::Shape::Text(text) => out.push((
                text.galley.job.text.clone(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes
        .iter()
        .for_each(|clipped| walk(&clipped.shape, &mut out));
    out
}

/// (content right edge, column right edge, text cells) of the page at `width`.
fn laid_out(category: Category, width: f32) -> (f32, f32, Vec<(String, Rect)>) {
    let ctx = egui::Context::default();
    onecad_widgets::fonts::install(&ctx);
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1400.0, 900.0))),
        ..Default::default()
    };
    let mut state = UiState::default();
    state.settings.category = category;
    // A pending conflict draws the widest row of the shortcuts page.
    state
        .settings
        .shortcuts
        .start(KeyContext::Modeling, ToolId::Chamfer);
    state.settings.shortcuts.keys(
        &mut state.shortcut_overrides,
        &[press(Key::H, Modifiers::NONE)],
    );
    let inner = Rect::from_min_max(pos2(100.0, 28.0), pos2(100.0 + width, 860.0));
    let (mut right, mut texts) = (0.0, Vec::new());
    for _ in 0..3 {
        let mut out = ctx.run_ui(raw.clone(), |ui| {
            right = scroll_content(ui, inner, &mut state).1.right();
        });
        out.textures_delta.clear();
        texts = text_rects(&out.shapes);
    }
    (right, inner.right(), texts)
}

#[test]
fn pages_stay_inside_the_content_column_at_every_width() {
    for category in [Category::General, Category::Navigation, Category::Shortcuts] {
        for width in [288.0, 360.0, 480.0, 584.0, 1140.0] {
            let (right, bound, _) = laid_out(category, width);
            assert!(
                right <= bound + 0.5,
                "{category:?} at {width}: content ends at {right}, column at {bound}"
            );
        }
    }
}

#[test]
fn shortcut_text_cells_never_overlap_horizontally() {
    for width in [288.0, 360.0, 480.0, 584.0, 1140.0] {
        let (_, _, texts) = laid_out(Category::Shortcuts, width);
        for (i, (a, ra)) in texts.iter().enumerate() {
            for (b, rb) in &texts[i + 1..] {
                let rows_overlap = ra.top().max(rb.top()) < ra.bottom().min(rb.bottom()) - 2.0;
                let cols_overlap = ra.left().max(rb.left()) < ra.right().min(rb.right()) - 0.5;
                assert!(
                    !(rows_overlap && cols_overlap),
                    "{width}: {a:?} {ra:?} overlaps {b:?} {rb:?}"
                );
            }
        }
    }
}

#[test]
fn raw_chords_stop_recording_without_changing_overrides() {
    for modifiers in [Modifiers::COMMAND, Modifiers::MAC_CMD, Modifiers::ALT] {
        let mut ui = ShortcutsUi::default();
        let overrides: Vec<ShortcutOverride> = Vec::new();
        ui.start(KeyContext::Modeling, ToolId::Chamfer);
        let events = [press(Key::S, modifiers), press(Key::Comma, modifiers)];
        assert!(recording::chord_pressed(&events), "{modifiers:?}");
        ui.cancel();
        assert!(!ui.capturing() && overrides.is_empty());
    }
    assert!(!recording::chord_pressed(&[press(
        Key::K,
        Modifiers::SHIFT
    )]));
    let released = Event::Key {
        key: Key::S,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    };
    assert!(!recording::chord_pressed(&[released]));
}

#[test]
fn recording_row_must_be_drawn_inside_the_scroll_clip() {
    let row = Rect::from_min_size(pos2(0.0, 500.0), vec2(300.0, 38.0));
    let clip = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 400.0));
    assert!(!recording::row_on_screen(None));
    assert!(!recording::row_on_screen(Some((row, clip))));
    assert!(recording::row_on_screen(Some((
        row,
        clip.translate(vec2(0.0, 200.0))
    ))));
}

#[test]
fn category_column_width_follows_the_host_within_the_design_range() {
    assert_eq!(column_width(768.0), 768.0 * 0.22);
    assert_eq!(column_width(1032.0), 208.0);
    assert_eq!(column_width(1280.0), 208.0);
    assert_eq!(column_width(400.0), 150.0);
}

/// Labels of every AccessKit node produced by `add`.
fn accessible_labels(add: impl FnMut(&mut egui::Ui)) -> Vec<String> {
    let mut add = add;
    let ctx = egui::Context::default();
    onecad_widgets::fonts::install(&ctx);
    ctx.enable_accesskit();
    let raw = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 900.0))),
        ..Default::default()
    };
    ctx.run_ui(raw(), &mut add).textures_delta.clear();
    let mut out = ctx.run_ui(raw(), &mut add);
    out.textures_delta.clear();
    let update = out.platform_output.accesskit_update.expect("AccessKit on");
    update
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

#[test]
fn painted_controls_expose_accessible_labels() {
    let mut state = UiState::default();
    state.shortcut_overrides.push(ShortcutOverride {
        context: KeyContext::Modeling,
        tool: ToolId::Chamfer,
        key: Some("K".into()),
    });
    state.settings.category = Category::Shortcuts;
    let labels = accessible_labels(|ui| {
        let column = Rect::from_min_size(pos2(0.0, 0.0), vec2(208.0, 800.0));
        category_column(&mut state, ui, column);
        let inner = Rect::from_min_max(pos2(250.0, 20.0), pos2(890.0, 880.0));
        scroll_content(ui, inner, &mut state);
    });
    for expected in [
        "General",
        "Keyboard shortcuts",
        "Chamfer shortcut, Modeling: K",
        "Hole shortcut, Modeling: H",
        "Reset Chamfer shortcut",
    ] {
        assert!(
            labels.iter().any(|l| l == expected),
            "{expected:?} missing from {labels:?}"
        );
    }
}

#[test]
fn theme_cards_and_recording_are_labelled() {
    let mut state = UiState::default();
    let labels = accessible_labels(|ui| {
        let inner = Rect::from_min_max(pos2(20.0, 20.0), pos2(700.0, 880.0));
        scroll_content(ui, inner, &mut state);
    });
    assert!(labels.iter().any(|l| l == "Light theme"), "{labels:?}");
    assert!(
        labels.iter().any(|l| l == "Match system theme"),
        "{labels:?}"
    );
    state.settings.category = Category::Shortcuts;
    state
        .settings
        .shortcuts
        .start(KeyContext::Modeling, ToolId::Chamfer);
    let labels = accessible_labels(|ui| {
        let inner = Rect::from_min_max(pos2(20.0, 20.0), pos2(700.0, 880.0));
        scroll_content(ui, inner, &mut state);
    });
    assert!(labels.iter().any(|l| l == "Recording"), "{labels:?}");
}
