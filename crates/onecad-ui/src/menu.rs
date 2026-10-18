//! macOS native menu bar (muda) with key equivalents. Menu items with key equivalents are
//! handled by AppKit before the key reaches winit, so egui never sees Cmd+Z: the app routes
//! the menu action itself (text field first, then document history) — see `DimInput::route_edit`.
//! Cut/Copy/Paste/Select All are custom items for the same reason: muda's predefined items
//! send `copy:` etc. to the first responder, which winit's view does not implement, and their
//! key equivalents still swallow the keys, so egui's own Cmd+C/V/A handling never runs.

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Save,
    Quit,
    Fit,
    Home,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Action;
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use std::sync::mpsc::{Receiver, channel};

    pub struct NativeMenu {
        _menu: Menu,
        rx: Receiver<Action>,
    }

    fn item(text: &str, mods: Modifiers, code: Code) -> MenuItem {
        MenuItem::new(text, true, Some(Accelerator::new(mods, code)))
    }

    pub fn install(ctx: &egui::Context) -> Result<NativeMenu, String> {
        let cmd = Modifiers::META;
        let undo = item("Undo", cmd, Code::KeyZ);
        let redo = item("Redo", Modifiers::META | Modifiers::SHIFT, Code::KeyZ);
        let save = item("Save", cmd, Code::KeyS);
        let quit = item("Quit OneCAD", cmd, Code::KeyQ);
        let fit = item("Fit All", cmd, Code::Digit0);
        let cut = item("Cut", cmd, Code::KeyX);
        let copy = item("Copy", cmd, Code::KeyC);
        let paste = item("Paste", cmd, Code::KeyV);
        let select_all = item("Select All", cmd, Code::KeyA);
        let home = item("Home View", Modifiers::META | Modifiers::SHIFT, Code::KeyH);
        let e = |e: muda::Error| e.to_string();
        let app = Submenu::with_items(
            "OneCAD",
            true,
            &[
                &PredefinedMenuItem::about(None, None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &quit,
            ],
        )
        .map_err(e)?;
        let file = Submenu::with_items("File", true, &[&save]).map_err(e)?;
        let edit = Submenu::with_items(
            "Edit",
            true,
            &[
                &undo,
                &redo,
                &PredefinedMenuItem::separator(),
                &cut,
                &copy,
                &paste,
                &select_all,
            ],
        )
        .map_err(e)?;
        let view = Submenu::with_items("View", true, &[&fit, &home]).map_err(e)?;
        let window = Submenu::with_items(
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::fullscreen(None),
            ],
        )
        .map_err(e)?;
        let menu = Menu::with_items(&[&app, &file, &edit, &view, &window]).map_err(e)?;
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();

        let ids = [
            (undo.id().clone(), Action::Undo),
            (redo.id().clone(), Action::Redo),
            (save.id().clone(), Action::Save),
            (quit.id().clone(), Action::Quit),
            (fit.id().clone(), Action::Fit),
            (home.id().clone(), Action::Home),
            (cut.id().clone(), Action::Cut),
            (copy.id().clone(), Action::Copy),
            (paste.id().clone(), Action::Paste),
            (select_all.id().clone(), Action::SelectAll),
        ];
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        // Menu events arrive outside winit's event stream; wake egui so the action is handled
        // on the next frame (rendering is on-demand, nothing else would repaint).
        MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
            if let Some((_, a)) = ids.iter().find(|(id, _)| *id == ev.id) {
                let _ = tx.send(*a);
                ctx.request_repaint();
            }
        }));
        Ok(NativeMenu { _menu: menu, rx })
    }

    impl NativeMenu {
        pub fn drain(&self) -> Vec<Action> {
            self.rx.try_iter().collect()
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Action;
    pub struct NativeMenu;
    pub fn install(_ctx: &egui::Context) -> Result<NativeMenu, String> {
        Err("no native menu on this platform".into())
    }
    impl NativeMenu {
        pub fn drain(&self) -> Vec<Action> {
            Vec::new()
        }
    }
}

pub use imp::{NativeMenu, install};

pub(crate) fn push_command(events: &mut Vec<egui::Event>, key: egui::Key, shift: bool) {
    for pressed in [true, false] {
        events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                mac_cmd: cfg!(target_os = "macos"),
                shift,
                ..Default::default()
            },
        });
    }
}

/// Some integrations may report both a native menu action and its keyboard event.
/// The menu owns that action; route it once, preserving menu event order.
pub(crate) fn remove_duplicate_events(actions: &[Action], events: &mut Vec<egui::Event>) {
    events.retain(|event| !actions.iter().any(|action| action.matches_event(event)));
}

impl Action {
    fn matches_event(self, event: &egui::Event) -> bool {
        match event {
            egui::Event::Copy => self == Self::Copy,
            egui::Event::Cut => self == Self::Cut,
            egui::Event::Paste(_) => self == Self::Paste,
            egui::Event::Key { key, modifiers, .. } => {
                let (expected, shift) = match self {
                    Self::Undo => (egui::Key::Z, false),
                    Self::Redo => (egui::Key::Z, true),
                    Self::Cut => (egui::Key::X, false),
                    Self::Copy => (egui::Key::C, false),
                    Self::Paste => (egui::Key::V, false),
                    Self::SelectAll => (egui::Key::A, false),
                    Self::Save => (egui::Key::S, false),
                    Self::Quit => (egui::Key::Q, false),
                    Self::Fit => (egui::Key::Num0, false),
                    Self::Home => (egui::Key::H, true),
                };
                *key == expected
                    && modifiers.matches_logically(egui::Modifiers {
                        command: true,
                        shift,
                        ..Default::default()
                    })
            }
            _ => false,
        }
    }
}
