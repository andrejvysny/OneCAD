//! Resolve chrome ownership before any document or CAD shortcut dispatch.
use crate::ui_state::UiState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owner {
    SketchEntry,
    Document,
    Customize,
    Popup,
    Layers,
    Extensions,
    Constraints,
    Placement,
}

pub(super) fn owner(ui: &UiState, entry_open: bool, document: bool, popup: bool) -> Option<Owner> {
    if document {
        Some(Owner::Document)
    } else if entry_open {
        Some(Owner::SketchEntry)
    } else if popup {
        Some(Owner::Popup)
    } else if ui.shell_facade.customize_open {
        Some(Owner::Customize)
    } else if ui.shell_facade.layers_open {
        Some(Owner::Layers)
    } else if ui.editor_facade.popover_open
        || ui.extensions.native.palette_open
        || ui.extensions.native.review.is_some()
        || ui.extensions.native.reset_confirmation
    {
        Some(Owner::Extensions)
    } else if ui.constraints_open {
        Some(Owner::Constraints)
    } else if ui.editor_facade.placing.is_some() {
        Some(Owner::Placement)
    } else {
        None
    }
}

pub(super) fn navigation_blocked(
    ui: &UiState,
    entry_open: bool,
    document: bool,
    popup: bool,
) -> bool {
    document
        || popup
        || owner(ui, entry_open, false, false).is_some_and(|owner| owner != Owner::SketchEntry)
}

pub(super) fn dismiss(
    ui: &mut UiState,
    entry: &mut crate::sketch_entry::Entry,
    owner: Owner,
    raw: &mut egui::RawInput,
    text: bool,
) {
    if !raw.focused || text || matches!(owner, Owner::Document | Owner::Popup) {
        return;
    }
    let escape = raw.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                repeat: false,
                ..
            }
        )
    });
    let finish_placement = owner == Owner::Placement
        && raw.events.iter().any(|event| {
            matches!(event,
        egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, repeat: false, .. }
            if !modifiers.command)
        });
    if !escape && !finish_placement {
        return;
    }
    match owner {
        Owner::SketchEntry => entry.dismiss(),
        Owner::Customize => ui.shell_facade.customize_open = false,
        Owner::Layers => ui.shell_facade.layers_open = false,
        Owner::Extensions => {
            ui.editor_facade.popover_open = false;
            ui.extensions.native.palette_open = false;
            ui.extensions.native.review = None;
            ui.extensions.native.reset_confirmation = false;
        }
        Owner::Constraints => ui.constraints_open = false,
        Owner::Placement => ui.editor_facade.placing = None,
        Owner::Document | Owner::Popup => unreachable!(),
    }
    // A dismissed owner must not expose another command to the rest of this frame.
    raw.events
        .retain(|event| !matches!(event, egui::Event::Key { .. }));
}

pub(super) fn text_menu(action: crate::menu::Action, text: bool, focused: bool) -> bool {
    text && focused
        && matches!(
            action,
            crate::menu::Action::Undo
                | crate::menu::Action::Redo
                | crate::menu::Action::Cut
                | crate::menu::Action::Copy
                | crate::menu::Action::Paste
                | crate::menu::Action::SelectAll
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_extensions_palette_owns_and_consumes_its_escape() {
        let mut state = UiState::default();
        state.extensions.native.open_palette();
        assert_eq!(owner(&state, false, false, false), Some(Owner::Extensions));
        assert!(navigation_blocked(&state, false, false, false));
        let mut raw = egui::RawInput {
            focused: true,
            ..Default::default()
        };
        raw.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        dismiss(
            &mut state,
            &mut Default::default(),
            Owner::Extensions,
            &mut raw,
            false,
        );
        assert!(!state.extensions.native.palette_open);
        assert!(raw.events.is_empty());
        assert_eq!(owner(&state, false, false, false), None);
    }
}
