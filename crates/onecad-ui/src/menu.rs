//! macOS native menu bar (muda) with key equivalents. Menu items with key equivalents are
//! handled by AppKit before the key reaches winit, so egui never sees Cmd+Z: the app routes
//! the menu action itself (text field first, then document history) — see `input::route`.
//! Cut/Copy/Paste/Select All are custom items for the same reason: muda's predefined items
//! send `copy:` etc. to the first responder, which winit's view does not implement, and their
//! key equivalents still swallow the keys, so egui's own Cmd+C/V/A handling never runs.

#[cfg_attr(not(all(target_os = "macos", feature = "desktop")), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    New,
    /// Home in focus, to start or open the next document.
    NewTab,
    Open,
    /// Index into the recents list the menu was last built from.
    OpenRecent(u8),
    CloseTab,
    SaveAll,
    Import,
    InsertStep,
    Save,
    SaveAs,
    Export(crate::document_flow::ExportFormat),
    Quit,
    Fit,
    Home,
    Appearance(crate::ui_state::Appearance),
}

#[cfg(all(target_os = "macos", feature = "desktop"))]
mod imp {
    use super::Action;
    use crate::ui_state::Appearance;
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Arc, Mutex};

    /// Menu item ids and their actions; Open Recent's entries are rebuilt in place.
    type Routes = Arc<Mutex<Vec<(MenuId, Action)>>>;

    pub struct NativeMenu {
        _menu: Menu,
        rx: Receiver<Action>,
        appearance: Choices,
        history: [MenuItem; 2],
        replacement: [MenuItem; 3],
        insert_step: MenuItem,
        saves: [MenuItem; 3],
        exports: [MenuItem; 4],
        recent: Submenu,
        recent_items: Mutex<(Vec<String>, Vec<MenuItem>)>,
        routes: Routes,
    }

    fn item(text: &str, mods: Modifiers, code: Code) -> MenuItem {
        MenuItem::new(text, true, Some(Accelerator::new(mods, code)))
    }

    type Choices = [(Appearance, CheckMenuItem); 3];

    /// View › Appearance: Light / Dark / System, Dark checked (the default).
    fn appearance_submenu() -> Result<(Submenu, Choices), muda::Error> {
        let choices = [
            (
                Appearance::Light,
                CheckMenuItem::new("Light", true, false, None),
            ),
            (
                Appearance::Dark,
                CheckMenuItem::new("Dark", true, true, None),
            ),
            (
                Appearance::System,
                CheckMenuItem::new("System", true, false, None),
            ),
        ];
        let submenu = Submenu::with_items(
            "Appearance",
            true,
            &[&choices[0].1, &choices[1].1, &choices[2].1],
        )?;
        Ok((submenu, choices))
    }

    pub fn install(ctx: &egui::Context) -> Result<NativeMenu, String> {
        let cmd = Modifiers::META;
        let undo = item("Undo", cmd, Code::KeyZ);
        let redo = item("Redo", Modifiers::META | Modifiers::SHIFT, Code::KeyZ);
        let new = item("New Part", cmd, Code::KeyN);
        let new_tab = item("New Tab", cmd, Code::KeyT);
        let open = item("Open…", cmd, Code::KeyO);
        let recent = Submenu::new("Open Recent", true);
        let close_tab = item("Close Tab", cmd, Code::KeyW);
        let import = item("Import…", cmd | Modifiers::SHIFT, Code::KeyI);
        let insert_step = MenuItem::new("Insert STEP…", true, None);
        let save = item("Save", cmd, Code::KeyS);
        let save_as = item("Save As…", cmd | Modifiers::SHIFT, Code::KeyS);
        let save_all = item("Save All", cmd | Modifiers::ALT, Code::KeyS);
        let exports =
            ["STEP…", "STL…", "OBJ…", "3MF…"].map(|label| MenuItem::new(label, true, None));
        let export = Submenu::with_items(
            "Export",
            true,
            &[&exports[0], &exports[1], &exports[2], &exports[3]],
        )
        .map_err(|error| error.to_string())?;
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
        let file = Submenu::with_items(
            "File",
            true,
            &[
                &new,
                &new_tab,
                &open,
                &recent,
                &PredefinedMenuItem::separator(),
                &close_tab,
                &save,
                &save_as,
                &save_all,
                &PredefinedMenuItem::separator(),
                &import,
                &insert_step,
                &export,
            ],
        )
        .map_err(e)?;
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
        let (appearance, choices) = appearance_submenu().map_err(e)?;
        let view = Submenu::with_items(
            "View",
            true,
            &[&fit, &home, &PredefinedMenuItem::separator(), &appearance],
        )
        .map_err(e)?;
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
            (
                choices[0].1.id().clone(),
                Action::Appearance(Appearance::Light),
            ),
            (
                choices[1].1.id().clone(),
                Action::Appearance(Appearance::Dark),
            ),
            (
                choices[2].1.id().clone(),
                Action::Appearance(Appearance::System),
            ),
            (undo.id().clone(), Action::Undo),
            (redo.id().clone(), Action::Redo),
            (new.id().clone(), Action::New),
            (new_tab.id().clone(), Action::NewTab),
            (open.id().clone(), Action::Open),
            (close_tab.id().clone(), Action::CloseTab),
            (save_all.id().clone(), Action::SaveAll),
            (import.id().clone(), Action::Import),
            (insert_step.id().clone(), Action::InsertStep),
            (save.id().clone(), Action::Save),
            (save_as.id().clone(), Action::SaveAs),
            (
                exports[0].id().clone(),
                Action::Export(crate::document_flow::ExportFormat::Step),
            ),
            (
                exports[1].id().clone(),
                Action::Export(crate::document_flow::ExportFormat::Stl),
            ),
            (
                exports[2].id().clone(),
                Action::Export(crate::document_flow::ExportFormat::Obj),
            ),
            (
                exports[3].id().clone(),
                Action::Export(crate::document_flow::ExportFormat::ThreeMf),
            ),
            (quit.id().clone(), Action::Quit),
            (fit.id().clone(), Action::Fit),
            (home.id().clone(), Action::Home),
            (cut.id().clone(), Action::Cut),
            (copy.id().clone(), Action::Copy),
            (paste.id().clone(), Action::Paste),
            (select_all.id().clone(), Action::SelectAll),
        ];
        let routes: Routes = Arc::new(Mutex::new(ids.to_vec()));
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let handler_routes = routes.clone();
        // Menu events arrive outside winit's event stream; wake egui so the action is handled
        // on the next frame (rendering is on-demand, nothing else would repaint).
        MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
            let action = handler_routes
                .lock()
                .ok()
                .and_then(|routes| routes.iter().find(|(id, _)| *id == ev.id).map(|(_, a)| *a));
            if let Some(action) = action {
                let _ = tx.send(action);
                ctx.request_repaint();
            }
        }));
        Ok(NativeMenu {
            _menu: menu,
            rx,
            appearance: choices,
            history: [undo, redo],
            replacement: [new, open, import],
            insert_step,
            saves: [save, save_as, save_all],
            exports,
            recent,
            recent_items: Mutex::new((Vec::new(), Vec::new())),
            routes,
        })
    }

    impl NativeMenu {
        pub fn set_history_enabled(&self, undo: bool, redo: bool) {
            self.history[0].set_enabled(undo);
            self.history[1].set_enabled(redo);
        }
        pub fn set_document_enabled(&self, replace: bool, insert: bool, save: bool, export: bool) {
            for item in &self.replacement {
                item.set_enabled(replace);
            }
            self.insert_step.set_enabled(insert);
            for item in &self.saves {
                item.set_enabled(save);
            }
            for item in &self.exports {
                item.set_enabled(export);
            }
        }

        /// Rebuilds File › Open Recent when the recents list changed.
        pub fn set_recents(&self, names: &[String]) {
            let Ok(mut current) = self.recent_items.lock() else {
                return;
            };
            if current.0 == names {
                return;
            }
            let Ok(mut routes) = self.routes.lock() else {
                return;
            };
            for item in current.1.drain(..) {
                routes.retain(|(id, _)| id != item.id());
                let _ = self.recent.remove(&item);
            }
            for (index, name) in names.iter().take(usize::from(u8::MAX)).enumerate() {
                let item = MenuItem::new(name, true, None);
                if self.recent.append(&item).is_ok() {
                    routes.push((item.id().clone(), Action::OpenRecent(index as u8)));
                    current.1.push(item);
                }
            }
            self.recent.set_enabled(!names.is_empty());
            current.0 = names.to_vec();
        }

        /// muda toggles a check item on click; restore the one-of-three invariant.
        pub fn set_appearance(&self, current: Appearance) {
            for (choice, item) in &self.appearance {
                item.set_checked(*choice == current);
            }
        }

        pub fn drain(&self) -> Vec<Action> {
            self.rx.try_iter().collect()
        }
    }
}

/// Without a native menu bar the same actions come from in-app controls
/// (`touch_menu`): they queue here and `input::route` drains them exactly like
/// AppKit menu events, with the same enablement the router publishes.
#[cfg(not(all(target_os = "macos", feature = "desktop")))]
mod imp {
    use super::Action;
    use std::cell::{Cell, RefCell};

    #[derive(Clone, Copy, Default)]
    struct Enabled {
        undo: bool,
        redo: bool,
        replace: bool,
        insert: bool,
        save: bool,
        export: bool,
    }

    #[derive(Default)]
    pub struct NativeMenu {
        queue: RefCell<Vec<Action>>,
        enabled: Cell<Enabled>,
    }
    #[cfg(feature = "desktop")]
    pub fn install(_ctx: &egui::Context) -> Result<NativeMenu, String> {
        Err("no native menu on this platform".into())
    }
    impl NativeMenu {
        pub fn drain(&self) -> Vec<Action> {
            std::mem::take(&mut *self.queue.borrow_mut())
        }
        pub fn set_appearance(&self, _current: crate::ui_state::Appearance) {}
        pub fn set_history_enabled(&self, undo: bool, redo: bool) {
            let mut enabled = self.enabled.get();
            (enabled.undo, enabled.redo) = (undo, redo);
            self.enabled.set(enabled);
        }
        pub fn set_document_enabled(&self, replace: bool, insert: bool, save: bool, export: bool) {
            let Enabled { undo, redo, .. } = self.enabled.get();
            self.enabled.set(Enabled {
                undo,
                redo,
                replace,
                insert,
                save,
                export,
            });
        }
        pub fn set_recents(&self, _names: &[String]) {}

        /// Queues an in-app menu choice for the next frame's routing; a disabled
        /// action is dropped, as a disabled AppKit item would never fire.
        #[cfg_attr(feature = "desktop", allow(dead_code))]
        pub fn push(&self, action: Action) {
            if self.enabled(action) {
                self.queue.borrow_mut().push(action);
            }
        }

        /// Mirrors the AppKit item groups of `set_document_enabled`.
        #[cfg_attr(feature = "desktop", allow(dead_code))]
        pub fn enabled(&self, action: Action) -> bool {
            let enabled = self.enabled.get();
            match action {
                Action::Undo => enabled.undo,
                Action::Redo => enabled.redo,
                Action::New | Action::Open | Action::Import => enabled.replace,
                Action::InsertStep => enabled.insert,
                Action::Save | Action::SaveAs | Action::SaveAll => enabled.save,
                Action::Export(_) => enabled.export,
                _ => true,
            }
        }
    }
}

pub use imp::NativeMenu;
#[cfg(feature = "desktop")]
pub use imp::install;

/// The in-app menu queue for hosts without a native menu bar. Desktop builds
/// install their menu from the window instead (`App::new`).
pub fn host_menu() -> Option<NativeMenu> {
    #[cfg(feature = "desktop")]
    {
        None
    }
    #[cfg(not(feature = "desktop"))]
    {
        Some(NativeMenu::default())
    }
}

pub(crate) fn push_command(events: &mut Vec<egui::Event>, key: egui::Key, shift: bool) {
    for pressed in [true, false] {
        events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                mac_cmd: cfg!(any(target_os = "macos", target_os = "ios")),
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
                    Self::New => (egui::Key::N, false),
                    Self::NewTab => (egui::Key::T, false),
                    Self::Open => (egui::Key::O, false),
                    Self::CloseTab => (egui::Key::W, false),
                    Self::Import => (egui::Key::I, true),
                    Self::Save => (egui::Key::S, false),
                    Self::SaveAs => (egui::Key::S, true),
                    Self::Quit => (egui::Key::Q, false),
                    Self::Fit => (egui::Key::Num0, false),
                    Self::Home => (egui::Key::H, true),
                    // Save All carries Option, which the shift-only comparison cannot express.
                    Self::Appearance(_)
                    | Self::Export(_)
                    | Self::OpenRecent(_)
                    | Self::SaveAll
                    | Self::InsertStep => {
                        return false;
                    }
                };
                *key == expected
                    && modifiers.shift == shift
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
