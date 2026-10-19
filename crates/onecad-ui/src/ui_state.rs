//! Chrome-only UI state: panel visibility, tabs and menus. Never touches the document.
use egui::{Context, Key, Modifiers};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InspectorTab {
    #[default]
    Properties,
    History,
}

/// View › Appearance choice; `System` follows the OS and defaults to dark when unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Appearance {
    Light,
    #[default]
    Dark,
    System,
}

impl Appearance {
    pub fn resolve_dark(self, system: Option<egui::Theme>) -> bool {
        match self {
            Self::Light => false,
            Self::Dark => true,
            Self::System => system != Some(egui::Theme::Light),
        }
    }
}

/// Full-window page shown while the Start screen is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StartPage {
    #[default]
    Home,
    Library,
    ComponentDetail,
    StandardPart,
    Extensions,
    ExtensionDetail,
}

#[derive(Clone, Debug)]
pub struct UiState {
    pub explorer_open: bool,
    pub inspector_open: bool,
    pub inspector_tab: InspectorTab,
    pub workspace_menu_open: bool,
    pub start_open: bool,
    pub explorer_filter: String,
    /// Anchor of the open Display menu; `None` while closed.
    pub display_menu: Option<egui::Rect>,
    /// Sketch Constraints menu (display-only rows).
    pub constraints_open: bool,
    pub rectangle_centered: bool,
    pub arc_three_point: bool,
    pub sketch_snap: bool,
    pub sketch_grid_snap: bool,
    pub appearance: Appearance,
    /// Target darkness whose theme failed to load; not retried until another succeeds.
    pub theme_failed: Option<bool>,
    /// Start screen Recent layout: 0 Grid, 1 List.
    pub recent_view: usize,
    pub start_page: StartPage,
    pub library: crate::library_pages::LibraryUi,
    pub extensions: crate::extension_pages::ExtensionsUi,
    pub shell_facade: crate::workspace_ui::ShellFacade,
    pub editor_facade: crate::editor_facade::EditorFacade,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            explorer_open: true,
            inspector_open: true,
            inspector_tab: InspectorTab::default(),
            workspace_menu_open: false,
            start_open: false,
            explorer_filter: String::new(),
            display_menu: None,
            constraints_open: false,
            rectangle_centered: false,
            arc_three_point: false,
            sketch_snap: true,
            sketch_grid_snap: false,
            appearance: Appearance::default(),
            theme_failed: None,
            recent_view: 0,
            start_page: StartPage::default(),
            library: Default::default(),
            extensions: Default::default(),
            shell_facade: Default::default(),
            editor_facade: Default::default(),
        }
    }
}

/// Cmd+1 toggles the Explorer, Cmd+2 the Inspector; ignored while a text field has focus.
pub fn shortcuts(state: &mut UiState, ctx: &Context) {
    if crate::input::text_owns_keyboard(ctx) || crate::input::ui_owns_keyboard(state, ctx) {
        return;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Num1)) {
        state.explorer_open = !state.explorer_open;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Num2)) {
        state.inspector_open = !state.inspector_open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, modifiers: Modifiers) -> egui::RawInput {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        raw
    }

    fn run(ctx: &Context, state: &mut UiState, raw: egui::RawInput) {
        let mut out = ctx.run_ui(raw, |ui| shortcuts(state, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn command_digits_toggle_panels() {
        let ctx = Context::default();
        let mut state = UiState::default();
        assert!(state.explorer_open && state.inspector_open);
        run(&ctx, &mut state, press(Key::Num1, Modifiers::COMMAND));
        assert!(!state.explorer_open && state.inspector_open);
        run(&ctx, &mut state, press(Key::Num2, Modifiers::COMMAND));
        assert!(!state.explorer_open && !state.inspector_open);
        run(&ctx, &mut state, press(Key::Num1, Modifiers::COMMAND));
        assert!(state.explorer_open);
    }

    #[test]
    fn ignored_while_text_edit_focused() {
        let ctx = Context::default();
        let mut state = UiState::default();
        let mut text = String::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let r = ui.text_edit_singleline(&mut text);
            r.request_focus();
        });
        out.textures_delta.clear();
        run(&ctx, &mut state, press(Key::Num1, Modifiers::COMMAND));
        assert!(state.explorer_open);
    }

    #[test]
    fn appearance_resolution() {
        let (l, d) = (Some(egui::Theme::Light), Some(egui::Theme::Dark));
        assert!(!Appearance::Light.resolve_dark(d));
        assert!(Appearance::Dark.resolve_dark(l));
        assert!(!Appearance::System.resolve_dark(l));
        assert!(Appearance::System.resolve_dark(d));
        assert!(Appearance::System.resolve_dark(None));
        assert_eq!(UiState::default().appearance, Appearance::Dark);
    }

    #[test]
    fn plain_digit_does_nothing() {
        let ctx = Context::default();
        let mut state = UiState::default();
        run(&ctx, &mut state, press(Key::Num1, Modifiers::NONE));
        assert!(state.explorer_open);
    }
}
