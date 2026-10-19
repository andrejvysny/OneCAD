use super::*;

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn composition(text: &str) -> egui::Event {
    egui::Event::Ime(egui::ImeEvent::Preedit {
        text: text.into(),
        active_range_chars: None,
    })
}

fn editor_frame(ctx: &egui::Context, text: &mut String, events: Vec<egui::Event>, time: f64) {
    let raw = egui::RawInput {
        events,
        time: Some(time),
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| {
        ui.add(egui::TextEdit::singleline(text).id(egui::Id::unique("arbitrary-editor")))
            .request_focus();
    });
    output.textures_delta.clear();
}

#[test]
fn every_text_editor_owns_shortcuts_without_numeric_registration() {
    let ctx = egui::Context::default();
    let mut text = String::new();
    editor_frame(&ctx, &mut text, vec![], 0.0);
    assert!(text_owns_keyboard(&ctx));
    ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::unique("arbitrary-editor")));
    assert!(!text_owns_keyboard(&ctx));
}

#[test]
fn native_select_all_copy_and_cut_reach_focused_editor() {
    let ctx = egui::Context::default();
    let mut text = "filter 0.1234".to_owned();
    editor_frame(&ctx, &mut text, vec![], 0.0);
    let mut events = Vec::new();
    assert!(route_text_menu(&mut events, menu::Action::SelectAll));
    editor_frame(&ctx, &mut text, events, 0.1);
    let mut events = Vec::new();
    assert!(route_text_menu(&mut events, menu::Action::Copy));
    assert!(matches!(events.as_slice(), [egui::Event::Copy]));
    assert!(route_text_menu(&mut events, menu::Action::Cut));
    editor_frame(&ctx, &mut text, events, 0.2);
    assert!(text.is_empty());
    editor_frame(
        &ctx,
        &mut text,
        vec![egui::Event::Paste("section".into())],
        0.3,
    );
    assert_eq!(text, "section");
}

#[test]
fn native_undo_redo_edit_text() {
    let ctx = egui::Context::default();
    let mut text = "filter".to_owned();
    editor_frame(&ctx, &mut text, vec![], 0.0);
    editor_frame(&ctx, &mut text, vec![], 1.0);
    editor_frame(&ctx, &mut text, vec![egui::Event::Text("x".into())], 2.0);
    editor_frame(&ctx, &mut text, vec![], 3.0);
    let edited = text.clone();
    assert_ne!(edited, "filter");
    let mut events = Vec::new();
    assert!(route_text_menu(&mut events, menu::Action::Undo));
    editor_frame(&ctx, &mut text, events, 4.0);
    assert_eq!(text, "filter");
    let mut events = Vec::new();
    assert!(route_text_menu(&mut events, menu::Action::Redo));
    editor_frame(&ctx, &mut text, events, 5.0);
    assert_eq!(text, edited);
}

#[test]
fn ime_start_and_commit_protect_entire_frame_before_cad_routing() {
    for ime in [
        composition("候補"),
        egui::Event::Ime(egui::ImeEvent::Commit("確定".into())),
    ] {
        for text in [true, false] {
            let mut composing = false;
            let mut raw = egui::RawInput {
                events: vec![key(egui::Key::Enter), ime.clone(), key(egui::Key::Escape)],
                ..Default::default()
            };
            protect_composition(&mut composing, &mut raw, text);
            assert_eq!(raw.events, vec![ime.clone()]);
        }
    }
}

#[test]
fn active_composition_protects_later_frames_and_end_frame() {
    let mut composing = true;
    let mut raw = egui::RawInput {
        events: vec![key(egui::Key::Enter), key(egui::Key::Escape)],
        ..Default::default()
    };
    protect_composition(&mut composing, &mut raw, true);
    assert!(raw.events.is_empty());
    assert!(composing);
    raw.events = vec![composition(""), key(egui::Key::Escape)];
    protect_composition(&mut composing, &mut raw, true);
    assert_eq!(raw.events, vec![composition("")]);
    assert!(!composing);
    raw.events = vec![key(egui::Key::Escape)];
    protect_composition(&mut composing, &mut raw, true);
    assert_eq!(raw.events, vec![key(egui::Key::Escape)]);
}

#[test]
fn focus_loss_resets_composition_without_leaking_cancel_key() {
    let mut composing = true;
    let mut raw = egui::RawInput {
        focused: false,
        events: vec![key(egui::Key::Escape)],
        ..Default::default()
    };
    protect_composition(&mut composing, &mut raw, true);
    assert!(!composing);
    assert!(raw.events.is_empty());
}

#[test]
#[allow(deprecated)]
fn ime_disabled_protects_its_last_frame() {
    let mut composing = true;
    let mut raw = egui::RawInput {
        events: vec![
            egui::Event::Ime(egui::ImeEvent::Disabled),
            key(egui::Key::Enter),
        ],
        ..Default::default()
    };
    protect_composition(&mut composing, &mut raw, true);
    assert!(!composing);
    assert_eq!(raw.events.len(), 1);
}

#[test]
fn document_shortcuts_distinguish_save_as_and_reject_unclaimed_modifiers() {
    let command = egui::Modifiers::COMMAND;
    assert_eq!(document_shortcut(egui::Key::N, command), Some(Request::New));
    assert_eq!(
        document_shortcut(egui::Key::O, command),
        Some(Request::Open(None))
    );
    assert_eq!(
        document_shortcut(egui::Key::S, command),
        Some(Request::Save { save_as: false })
    );
    let shifted = egui::Modifiers {
        shift: true,
        ..command
    };
    assert_eq!(
        document_shortcut(egui::Key::S, shifted),
        Some(Request::Save { save_as: true })
    );
    assert_eq!(document_shortcut(egui::Key::N, shifted), None);
    assert_eq!(document_shortcut(egui::Key::N, egui::Modifiers::NONE), None);
    assert_eq!(
        document_shortcut(
            egui::Key::S,
            egui::Modifiers {
                alt: true,
                ..command
            }
        ),
        None
    );
}

#[test]
fn native_file_actions_remove_only_their_own_key_events() {
    for (action, key, shift) in [
        (menu::Action::New, egui::Key::N, false),
        (menu::Action::Open, egui::Key::O, false),
        (menu::Action::Save, egui::Key::S, false),
        (menu::Action::SaveAs, egui::Key::S, true),
    ] {
        let mut events = Vec::new();
        menu::push_command(&mut events, key, shift);
        menu::push_command(&mut events, key, !shift);
        menu::remove_duplicate_events(&[action], &mut events);
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| matches!(event,
            egui::Event::Key { modifiers, .. } if modifiers.shift != shift)));
    }
}

fn route_history(
    state: &mut RoutingState,
    pass: u64,
    raw: &mut egui::RawInput,
    text: bool,
) -> Vec<Command> {
    let mut commands = Vec::new();
    route_events(state, pass, raw, |raw| {
        raw.events.retain(|event| {
            if let Some(command) = history_shortcut(event, text, raw.focused) {
                commands.push(command);
                false
            } else {
                true
            }
        });
    });
    commands
}

#[test]
fn retained_text_undo_cannot_become_model_undo_before_an_egui_pass() {
    let mut state = RoutingState::default();
    let mut raw = egui::RawInput::default();
    menu::push_command(&mut raw.events, egui::Key::Z, false);
    let original = raw.events.clone();
    assert!(route_history(&mut state, 7, &mut raw, true).is_empty());
    assert_eq!(raw.events, original);
    assert!(route_history(&mut state, 7, &mut raw, false).is_empty());
    assert_eq!(raw.events, original);
}

#[test]
fn appended_fresh_history_shortcut_routes_once_while_old_text_input_is_preserved() {
    let mut state = RoutingState::default();
    let mut raw = egui::RawInput::default();
    menu::push_command(&mut raw.events, egui::Key::Z, false);
    let text_events = raw.events.clone();
    assert!(route_history(&mut state, 7, &mut raw, true).is_empty());
    menu::push_command(&mut raw.events, egui::Key::Z, true);
    assert!(matches!(
        route_history(&mut state, 7, &mut raw, false).as_slice(),
        [Command::Redo]
    ));
    assert_eq!(&raw.events[..text_events.len()], text_events.as_slice());
    assert!(route_history(&mut state, 7, &mut raw, false).is_empty());
}

#[test]
fn an_egui_pass_consumes_retained_input_so_new_events_can_route() {
    let ctx = egui::Context::default();
    let mut state = RoutingState::default();
    let mut raw = egui::RawInput::default();
    menu::push_command(&mut raw.events, egui::Key::Z, false);
    let old_pass = ctx.cumulative_pass_nr();
    assert!(route_history(&mut state, old_pass, &mut raw, true).is_empty());
    let mut output = ctx.run_ui(raw, |_ui| {});
    output.textures_delta.clear();
    assert!(ctx.cumulative_pass_nr() > old_pass);
    let mut raw = egui::RawInput::default();
    menu::push_command(&mut raw.events, egui::Key::Z, false);
    assert!(matches!(
        route_history(&mut state, ctx.cumulative_pass_nr(), &mut raw, false).as_slice(),
        [Command::Undo]
    ));
}

#[test]
fn native_menu_text_undo_injection_does_not_replay_as_model_undo() {
    let mut state = RoutingState::default();
    let mut raw = egui::RawInput::default();
    route_events(&mut state, 7, &mut raw, |raw| {
        assert!(route_text_menu(&mut raw.events, menu::Action::Undo));
    });
    let text_events = raw.events.clone();
    assert_eq!(text_events.len(), 2);
    assert!(route_history(&mut state, 7, &mut raw, false).is_empty());
    assert_eq!(raw.events, text_events);
}

#[test]
fn retained_ime_commit_is_preserved_but_does_not_protect_a_later_escape() {
    let mut state = RoutingState::default();
    let mut composing = true;
    let commit = egui::Event::Ime(egui::ImeEvent::Commit("確定".into()));
    let mut raw = egui::RawInput {
        events: vec![commit.clone()],
        ..Default::default()
    };
    route_events(&mut state, 7, &mut raw, |raw| {
        protect_composition(&mut composing, raw, true)
    });
    assert!(!composing);
    raw.events.push(key(egui::Key::Escape));
    route_events(&mut state, 7, &mut raw, |raw| {
        protect_composition(&mut composing, raw, true)
    });
    assert_eq!(raw.events, vec![commit, key(egui::Key::Escape)]);
}

#[test]
fn retained_events_are_restored_when_fresh_routing_returns_early() {
    let mut state = RoutingState::default();
    let mut raw = egui::RawInput {
        events: vec![egui::Event::Text("filter".into())],
        ..Default::default()
    };
    route_events(&mut state, 7, &mut raw, |_| {});
    raw.events.push(key(egui::Key::Enter));
    route_events(&mut state, 7, &mut raw, |raw| {
        assert_eq!(raw.events, vec![key(egui::Key::Enter)]);
        raw.events.clear();
    });
    assert_eq!(raw.events, vec![egui::Event::Text("filter".into())]);
}
