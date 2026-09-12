//! The native application menu (WP-U1, decision D-3).
//!
//! Why this exists at all: without an explicit menu, macOS gives a Tauri app the
//! implicit AppKit default, whose Edit → Undo is the **NSResponder text** undo —
//! it can never reach the document history, and its ⌘Z key equivalent competes
//! with the webview's. The 2026-09-11 review lost every undo that way.
//!
//! Two rules shape this module:
//!
//! 1. **Rust decides nothing.** A menu item emits [`events::MENU_ACTION`] with a
//!    verb and stops. The frontend owns what "undo" means (sketch stack vs
//!    document history, and native text undo while a field has focus — see
//!    `src/features/shell/undoActions.ts`), and File → Save/Open/New reuse the
//!    exact bridges the ⌘-chords already call. A menu that reached into
//!    [`crate::document_runtime`] directly would be a second, divergent answer.
//! 2. **Ids are the contract.** [`action_for`] is the one id → verb map, and
//!    [`build`] labels every item from the same constants, so the two cannot
//!    drift.
//!
//! Only the application submenu is macOS-only; the menu itself is built on every
//! desktop OS.

#[cfg(target_os = "macos")]
use tauri::menu::AboutMetadata;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Runtime};

use crate::dto::MenuActionDto;
use crate::events;

/// File → New Project.
pub const ID_FILE_NEW: &str = "menu.file.new";
/// File → Open…
pub const ID_FILE_OPEN: &str = "menu.file.open";
/// File → Save.
pub const ID_FILE_SAVE: &str = "menu.file.save";
/// File → Save As…
pub const ID_FILE_SAVE_AS: &str = "menu.file.save_as";
/// Edit → Undo (document history, NOT the AppKit text undo).
pub const ID_EDIT_UNDO: &str = "menu.edit.undo";
/// Edit → Redo.
pub const ID_EDIT_REDO: &str = "menu.edit.redo";

/// Maps a menu item id onto the camelCase verb the frontend router understands.
///
/// `None` for every predefined item (cut/copy/paste/quit/…): those are handled
/// by the OS and must NOT reach the webview.
pub fn action_for(id: &str) -> Option<&'static str> {
    match id {
        ID_FILE_NEW => Some("new"),
        ID_FILE_OPEN => Some("open"),
        ID_FILE_SAVE => Some("save"),
        ID_FILE_SAVE_AS => Some("saveAs"),
        ID_EDIT_UNDO => Some("undo"),
        ID_EDIT_REDO => Some("redo"),
        _ => None,
    }
}

/// Builds the application menu.
///
/// Accelerators are declared HERE rather than in the frontend keymap because the
/// OS resolves a menu key equivalent before the webview ever sees the keystroke;
/// the `useShortcuts` chords stay in place for the browser / mock lane, where
/// there is no menu.
///
/// File deliberately carries NO predefined "Close Window": that item owns ⌘W at
/// the OS level, and in OneCAD ⌘W means "close the PROJECT and go back to the
/// start screen" (`fileActions.closeProject`), not "close the window", which the
/// `CloseRequested` guard in `run()` turns into a quit. Adding it would silently
/// re-point a chord this work package was not asked to change.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &MenuItem::with_id(app, ID_FILE_NEW, "New Project", true, Some("CmdOrCtrl+N"))?,
            &MenuItem::with_id(app, ID_FILE_OPEN, "Open…", true, Some("CmdOrCtrl+O"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, ID_FILE_SAVE, "Save", true, Some("CmdOrCtrl+S"))?,
            &MenuItem::with_id(
                app,
                ID_FILE_SAVE_AS,
                "Save As…",
                true,
                Some("Shift+CmdOrCtrl+S"),
            )?,
        ],
    )?;

    // Undo/Redo are OURS, not `PredefinedMenuItem::undo`: the predefined pair
    // sends the AppKit text-undo selector down the responder chain, which cannot
    // reach the document history. Cut/Copy/Paste/Select All stay predefined —
    // those genuinely belong to the focused text field.
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &MenuItem::with_id(app, ID_EDIT_UNDO, "Undo", true, Some("CmdOrCtrl+Z"))?,
            &MenuItem::with_id(app, ID_EDIT_REDO, "Redo", true, Some("Shift+CmdOrCtrl+Z"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
        ],
    )?;

    #[cfg(target_os = "macos")]
    let app_submenu = {
        let pkg = app.package_info();
        let config = app.config();
        let about = AboutMetadata {
            name: Some(pkg.name.clone()),
            version: Some(pkg.version.to_string()),
            copyright: config.bundle.copyright.clone(),
            authors: config.bundle.publisher.clone().map(|p| vec![p]),
            ..Default::default()
        };
        Submenu::with_items(
            app,
            pkg.name.clone(),
            true,
            &[
                &PredefinedMenuItem::about(app, None, Some(about))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, None)?,
            ],
        )?
    };

    Menu::with_items(
        app,
        &[
            #[cfg(target_os = "macos")]
            &app_submenu,
            &file,
            &edit,
            &window,
        ],
    )
}

/// Builds and installs the menu, then wires every OneCAD item to
/// [`events::MENU_ACTION`].
///
/// Emitted to the whole app (not one window): the webview is the only listener,
/// and a failed emit is not worth tearing anything down over — the keydown lane
/// still serves the same verbs.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = build(app)?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let Some(action) = action_for(event.id().as_ref()) else {
            return; // a predefined item — the OS already handled it
        };
        if let Err(e) = app.emit(events::MENU_ACTION, MenuActionDto { action }) {
            tracing::warn!("menu-action emit failed ({e}); action={action} dropped");
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // `build()` itself is NOT unit-testable, under `tauri::test::mock_app()` or
    // otherwise: `muda` asserts a main-thread marker before it creates any menu
    // child (`muda-0.19.3/src/platform_impl/macos/mod.rs:328` — measured: a
    // `mock_app()` build panics with "`muda::MenuChild` can only be created on
    // the main thread"), and libtest runs every test on a spawned thread, so such
    // a test would panic rather than fail, on one thread or many. The id → verb
    // map is the part that can drift, and `build()` labels its items from these
    // same constants, so that is what is pinned here. The rendered menu is an
    // owed user-run check on the bundled app.

    #[test]
    fn every_onecad_id_maps_to_its_wire_verb() {
        assert_eq!(action_for(ID_FILE_NEW), Some("new"));
        assert_eq!(action_for(ID_FILE_OPEN), Some("open"));
        assert_eq!(action_for(ID_FILE_SAVE), Some("save"));
        assert_eq!(action_for(ID_FILE_SAVE_AS), Some("saveAs"));
        assert_eq!(action_for(ID_EDIT_UNDO), Some("undo"));
        assert_eq!(action_for(ID_EDIT_REDO), Some("redo"));
    }

    #[test]
    fn predefined_and_unknown_ids_emit_nothing() {
        // Predefined items carry muda's own numeric/native ids; nothing outside
        // the six may reach the webview.
        assert_eq!(action_for("1"), None);
        assert_eq!(action_for("menu.edit.cut"), None);
        assert_eq!(action_for(""), None);
    }

    #[test]
    fn the_six_ids_are_distinct_and_namespaced() {
        let ids = [
            ID_FILE_NEW,
            ID_FILE_OPEN,
            ID_FILE_SAVE,
            ID_FILE_SAVE_AS,
            ID_EDIT_UNDO,
            ID_EDIT_REDO,
        ];
        let unique: std::collections::BTreeSet<&str> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "duplicate menu id");
        assert!(ids.iter().all(|id| id.starts_with("menu.")));
    }

    #[test]
    fn the_payload_serialises_camel_case() {
        let v = serde_json::to_value(MenuActionDto { action: "saveAs" }).unwrap();
        assert_eq!(v["action"], "saveAs");
    }
}
