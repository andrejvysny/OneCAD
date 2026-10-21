use super::{overlay::*, *};
use crate::ui_state::UiState;

fn escape() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn one_escape_dismisses_only_highest_owner_before_retained_input_replay() {
    let mut ui = UiState::default();
    ui.shell_facade.customize_open = true;
    ui.shell_facade.layers_open = true;
    ui.constraints_open = true;
    let mut state = RoutingState::default();
    let mut raw = egui::RawInput {
        events: vec![escape()],
        ..Default::default()
    };
    route_events(&mut state, 7, &mut raw, |raw| {
        assert_eq!(owner(&ui, false, false), Some(Owner::Customize));
        dismiss(&mut ui, Owner::Customize, raw, false);
    });
    assert!(!ui.shell_facade.customize_open);
    assert!(ui.shell_facade.layers_open && ui.constraints_open);
    assert!(raw.events.is_empty());
    route_events(&mut state, 7, &mut raw, |raw| {
        dismiss(&mut ui, Owner::Layers, raw, false)
    });
    assert!(ui.shell_facade.layers_open);
    raw.events.push(escape());
    route_events(&mut state, 7, &mut raw, |raw| {
        dismiss(&mut ui, Owner::Layers, raw, false)
    });
    assert!(!ui.shell_facade.layers_open);
    assert!(ui.constraints_open);
}

#[test]
fn modal_ownership_blocks_native_model_actions_but_keeps_text_history() {
    for action in [
        menu::Action::Undo,
        menu::Action::Redo,
        menu::Action::Save,
        menu::Action::New,
        menu::Action::Fit,
        menu::Action::Quit,
    ] {
        assert!(!text_menu(action, false, true));
        assert!(!text_menu(action, true, false));
        assert_eq!(
            text_menu(action, true, true),
            matches!(action, menu::Action::Undo | menu::Action::Redo)
        );
    }
    let mut ui = UiState::default();
    ui.shell_facade.customize_open = true;
    assert_eq!(owner(&ui, true, true), Some(Owner::Document));
    assert_eq!(owner(&ui, false, true), Some(Owner::Popup));
    ui.shell_facade.customize_open = false;
    ui.shell_facade.layers_open = true;
    assert_eq!(owner(&ui, false, true), Some(Owner::Popup));
}

#[test]
fn composition_text_focus_and_lost_focus_do_not_dismiss_overlay() {
    for (text, focused, ime) in [
        (true, true, false),
        (false, false, false),
        (true, true, true),
    ] {
        let mut ui = UiState::default();
        ui.shell_facade.customize_open = true;
        let mut raw = egui::RawInput {
            focused,
            events: vec![escape()],
            ..Default::default()
        };
        if ime {
            raw.events
                .push(egui::Event::Ime(egui::ImeEvent::Commit("確定".into())));
            protect_composition(&mut false, &mut raw, text);
        }
        dismiss(&mut ui, Owner::Customize, &mut raw, text);
        assert!(ui.shell_facade.customize_open);
    }
}

#[test]
fn placement_enter_is_consumed_without_reaching_document_commands() {
    let mut ui = UiState::default();
    ui.editor_facade.begin_placement("Test component");
    let mut enter = escape();
    if let egui::Event::Key { key, .. } = &mut enter {
        *key = egui::Key::Enter;
    }
    let mut raw = egui::RawInput {
        events: vec![enter],
        ..Default::default()
    };
    dismiss(&mut ui, Owner::Placement, &mut raw, false);
    assert!(ui.editor_facade.placing.is_none());
    assert!(raw.events.is_empty());
}

#[test]
fn dismissed_overlay_blocks_retained_pass_but_not_later_direct_ui_frames() {
    let ctx = egui::Context::default();
    let pass = ctx.cumulative_pass_nr();
    let mut state = RoutingState {
        pass,
        retained: 0,
        overlay_blocked: true,
    };
    let mut raw = egui::RawInput {
        events: vec![escape()],
        ..Default::default()
    };
    route_events(&mut state, pass, &mut raw, |raw| raw.events.clear());
    assert!(state.overlay_blocks(ctx.cumulative_pass_nr()));
    route_events(&mut state, pass, &mut raw, |_| {});
    assert!(state.overlay_blocks(ctx.cumulative_pass_nr()));
    let mut output = ctx.run_ui(raw, |ui| {
        assert!(state.overlay_blocks(ui.ctx().cumulative_pass_nr()));
    });
    output.textures_delta.clear();
    assert!(!state.overlay_blocks(ctx.cumulative_pass_nr()));
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        assert!(!state.overlay_blocks(ui.ctx().cumulative_pass_nr()));
    });
    output.textures_delta.clear();
}
