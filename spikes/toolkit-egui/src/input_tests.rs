use super::*;
use egui::{Context, Event, ImeEvent, Modifiers, RawInput};

struct Harness {
    ctx: Context,
    dim: DimInput,
    status: String,
    time: f64,
    other_text: Option<String>,
    other_id: Option<egui::Id>,
    focus_other: bool,
}

impl Harness {
    fn new() -> Self {
        let mut this = Self {
            ctx: Context::default(),
            dim: DimInput {
                value_mm: 45.86,
                text: "45.86".into(),
                editing: false,
                error: None,
                undo: vec![12.0],
                redo: Vec::new(),
                anchor: [glam::Vec3::ZERO; 3],
                request_focus: true,
                id: None,
                composing: false,
            },
            status: "Ready".into(),
            time: 0.0,
            other_text: None,
            other_id: None,
            focus_other: false,
        };
        this.frame(Vec::new());
        this.frame(Vec::new());
        assert!(this.dim.editing);
        assert!(this.ctx.memory(|m| m.focused().is_some()));
        this
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.run(RawInput {
            events,
            ..Default::default()
        });
    }

    fn run(&mut self, mut raw: RawInput) {
        self.time += 0.1;
        raw.time = Some(self.time);
        self.dim.prepare_input(&self.ctx, &mut raw);
        self.ctx
            .run_ui(raw, |ui| {
                dimension_field(&mut self.dim, &mut self.status, ui, &crate::theme::dark());
                if let Some(text) = &mut self.other_text {
                    let response = ui.add(egui::TextEdit::singleline(text).id_salt("other-field"));
                    self.other_id = Some(response.id);
                    if std::mem::take(&mut self.focus_other) {
                        response.request_focus();
                    }
                }
            })
            .drop_without_applying_deltas();
    }

    fn menu_frame(&mut self, actions: &[menu::Action], mut events: Vec<Event>) {
        menu::remove_duplicate_events(actions, &mut events);
        for action in actions {
            self.dim
                .route_edit(&self.ctx, *action, &mut self.status, &mut events);
        }
        self.frame(events);
    }

    fn replace(&mut self, text: &str) {
        self.frame(vec![command(Key::A, false), Event::Text(text.into())]);
        assert_eq!(self.dim.text, text);
    }
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn command(key: Key, shift: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers {
            command: true,
            mac_cmd: true,
            shift,
            ..Default::default()
        },
    }
}

fn preedit(text: &str) -> Event {
    Event::Ime(ImeEvent::Preedit {
        text: text.into(),
        active_range_chars: None,
    })
}

#[test]
fn expression_enter_commits_and_blurs() {
    let mut h = Harness::new();
    h.replace("10 mm + 2*3");
    h.frame(vec![key(Key::Enter)]);
    assert_eq!(h.dim.value_mm, 16.0);
    assert!(!h.dim.editing);
    assert_eq!(h.ctx.memory(|m| m.focused()), None);
    assert_eq!(h.dim.undo, [12.0, 45.86]);
}

#[test]
fn invalid_enter_keeps_focus_escape_cancels() {
    let mut h = Harness::new();
    h.replace("10 parsecs");
    h.frame(vec![key(Key::Enter)]);
    assert_eq!(h.dim.error.as_deref(), Some("unknown unit 'parsecs'"));
    assert!(h.dim.editing);
    assert!(h.ctx.memory(|m| m.focused().is_some()));
    h.frame(vec![key(Key::Escape)]);
    assert_eq!(h.dim.value_mm, 45.86);
    assert_eq!(h.dim.text, "45.86");
    assert!(!h.dim.editing);
    assert!(h.dim.error.is_none());
}

#[test]
fn composition_enter_does_not_commit_model() {
    let mut h = Harness::new();
    h.replace("16");
    h.frame(vec![preedit("2")]);
    let text = h.dim.text.clone();
    h.frame(vec![key(Key::Enter)]);
    assert_eq!(h.dim.value_mm, 45.86);
    assert_eq!(h.dim.text, text);
    assert!(h.dim.editing);
    assert!(h.ctx.memory(|m| m.focused().is_some()));
    assert_eq!(h.dim.undo, [12.0]);
}

#[test]
fn composition_escape_does_not_cancel_field() {
    let mut h = Harness::new();
    h.replace("16");
    h.frame(vec![preedit("2")]);
    let text = h.dim.text.clone();
    h.frame(vec![key(Key::Escape)]);
    assert_eq!(h.dim.text, text);
    assert!(h.dim.editing);
    assert!(h.ctx.memory(|m| m.focused().is_some()));
}

#[test]
fn composition_commit_and_enter_same_frame_does_not_commit_model() {
    let mut h = Harness::new();
    h.replace("16");
    h.frame(vec![preedit("2")]);
    h.frame(vec![
        Event::Ime(ImeEvent::Commit("2".into())),
        key(Key::Enter),
    ]);
    assert_eq!(h.dim.value_mm, 45.86);
    assert!(h.dim.editing);
    assert!(h.ctx.memory(|m| m.focused().is_some()));
}

#[test]
fn composition_dismissal_frame_is_protected_then_enter_commits() {
    let mut h = Harness::new();
    h.replace("16");
    h.frame(vec![preedit("2")]);
    h.frame(vec![key(Key::Escape), preedit("")]);
    assert!(h.dim.editing);
    assert_eq!(h.dim.text, "16");
    assert!(!h.dim.composing);
    h.frame(vec![key(Key::Enter)]);
    assert_eq!(h.dim.value_mm, 16.0);
    assert!(!h.dim.editing);
}

#[test]
fn enter_before_commit_in_same_frame_does_not_commit_model() {
    let mut h = Harness::new();
    h.replace("16");
    h.frame(vec![
        key(Key::Enter),
        Event::Ime(ImeEvent::Commit("2".into())),
    ]);
    assert_eq!(h.dim.value_mm, 45.86);
    assert!(h.dim.editing);
    assert!(!h.dim.composing);
}

#[test]
#[allow(deprecated)]
fn ime_enabled_without_preedit_does_not_block_enter() {
    let mut h = Harness::new();
    h.replace("16");
    h.frame(vec![Event::Ime(ImeEvent::Enabled)]);
    h.frame(vec![key(Key::Enter)]);
    assert_eq!(h.dim.value_mm, 16.0);
    assert!(!h.dim.editing);
}

#[test]
fn window_blur_cancels_composition_and_allows_fresh_edit() {
    let mut h = Harness::new();
    h.replace("16");
    h.frame(vec![preedit("2")]);
    h.run(RawInput {
        focused: false,
        ..Default::default()
    });
    assert!(!h.dim.editing);
    assert!(!h.dim.composing);
    assert_eq!(h.dim.text, "45.86");
    assert_eq!(h.ctx.memory(|m| m.focused()), None);
    h.dim.request_focus = true;
    h.frame(Vec::new());
    h.frame(Vec::new());
    h.replace("42");
    h.frame(vec![key(Key::Enter)]);
    assert_eq!(h.dim.value_mm, 42.0);
}

#[test]
fn focused_empty_text_history_never_undoes_or_redoes_model() {
    let mut h = Harness::new();
    h.dim.redo = vec![99.0];
    for action in [menu::Action::Undo, menu::Action::Redo] {
        h.menu_frame(&[action], Vec::new());
        assert_eq!(h.dim.value_mm, 45.86);
        assert_eq!(h.dim.text, "45.86");
        assert_eq!(h.dim.undo, [12.0]);
        assert_eq!(h.dim.redo, [99.0]);
        assert!(h.dim.editing);
    }
}

#[test]
fn exact_focus_owns_undo_before_editing_flag_catches_up() {
    let mut h = Harness::new();
    h.dim.editing = false;
    h.menu_frame(&[menu::Action::Undo], Vec::new());
    assert_eq!(h.dim.value_mm, 45.86);
    assert_eq!(h.dim.undo, [12.0]);
}

fn focus_second_field(h: &mut Harness) {
    h.other_text = Some("second".into());
    h.focus_other = true;
    h.frame(Vec::new());
    h.frame(Vec::new());
    assert_eq!(h.ctx.memory(|m| m.focused()), h.other_id);
    assert_ne!(h.other_id, h.dim.id);
    assert!(!h.dim.editing);
}

#[test]
fn second_text_field_empty_undo_and_redo_never_touch_model_history() {
    let mut h = Harness::new();
    focus_second_field(&mut h);
    h.dim.redo = vec![99.0];
    let state = egui::text_edit::TextEditState::load(&h.ctx, h.other_id.unwrap()).unwrap();
    let current = (state.cursor.char_range().unwrap(), "second".to_owned());
    assert!(!state.undoer().has_undo(&current));
    assert!(!state.undoer().has_redo(&current));
    for action in [menu::Action::Undo, menu::Action::Redo] {
        h.menu_frame(&[action], Vec::new());
        assert_eq!(h.other_text.as_deref(), Some("second"));
        assert_eq!(h.ctx.memory(|m| m.focused()), h.other_id);
        assert_eq!(h.dim.value_mm, 45.86);
        assert_eq!(h.dim.text, "45.86");
        assert_eq!(h.dim.undo, [12.0]);
        assert_eq!(h.dim.redo, [99.0]);
    }
}

#[test]
fn native_edit_keys_target_second_focused_text_field() {
    let mut h = Harness::new();
    focus_second_field(&mut h);
    h.frame(vec![Event::Text("9".into())]);
    assert_eq!(h.other_text.as_deref(), Some("second9"));
    h.menu_frame(&[menu::Action::Undo], Vec::new());
    assert_eq!(h.other_text.as_deref(), Some("second"));
    h.menu_frame(&[menu::Action::Redo], Vec::new());
    assert_eq!(h.other_text.as_deref(), Some("second9"));
    assert_eq!(h.dim.value_mm, 45.86);
    assert_eq!(h.dim.text, "45.86");
    assert_eq!(h.dim.undo, [12.0]);
    assert!(h.dim.redo.is_empty());
}

#[test]
fn native_and_keyboard_undo_perform_one_text_undo() {
    let mut h = Harness::new();
    h.replace("16");
    h.time += 2.0;
    h.frame(Vec::new());
    h.replace("42");
    h.menu_frame(&[menu::Action::Undo], vec![command(Key::Z, false)]);
    assert_eq!(h.dim.text, "16");
    assert_eq!(h.dim.value_mm, 45.86);
    assert_eq!(h.dim.undo, [12.0]);
    h.menu_frame(&[menu::Action::Redo], vec![command(Key::Z, true)]);
    assert_eq!(h.dim.text, "42");
}

#[test]
fn native_menu_order_preserves_multiple_text_history_actions() {
    let mut h = Harness::new();
    h.replace("16");
    h.menu_frame(&[menu::Action::Undo, menu::Action::Redo], Vec::new());
    assert_eq!(h.dim.text, "16");
    assert_eq!(h.dim.value_mm, 45.86);
}

#[test]
fn model_history_runs_after_field_escape() {
    let mut h = Harness::new();
    h.replace("16");
    h.frame(vec![key(Key::Escape)]);
    h.menu_frame(&[menu::Action::Undo], Vec::new());
    assert_eq!(h.dim.value_mm, 12.0);
    assert_eq!(h.dim.text, "12.00");
    h.menu_frame(&[menu::Action::Redo], Vec::new());
    assert_eq!(h.dim.value_mm, 45.86);
}

#[test]
fn refocus_after_model_undo_starts_new_text_history() {
    let mut h = Harness::new();
    h.replace("16");
    h.frame(vec![key(Key::Enter)]);
    h.menu_frame(&[menu::Action::Undo], Vec::new());
    assert_eq!(h.dim.value_mm, 45.86);
    h.dim.request_focus = true;
    h.frame(Vec::new());
    h.frame(Vec::new());
    h.frame(vec![Event::Text("9".into())]);
    h.menu_frame(&[menu::Action::Undo], Vec::new());
    assert_eq!(h.dim.text, "45.86");
    assert_eq!(h.dim.value_mm, 45.86);
    assert_eq!(h.dim.redo, [16.0]);
}
