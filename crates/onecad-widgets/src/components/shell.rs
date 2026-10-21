//! Shell chrome (g02): title bar, floating panel frame, panel toggles, workspace switcher
//! and status bar. Cards c-titlebar, c-panel, c-ptoggle, c-ws, c-status.

mod icon_button;
mod panel;
mod status;
mod tabs;
mod titlebar;
mod workspace;
mod workspace_menu;
mod workspace_selector;

pub(crate) use icon_button::{Backdrop, IconButton};
pub use panel::{FloatingPanel, PanelToggle, Side};
pub use status::{StatusBar, StatusItem, StatusOutput};
pub use tabs::{TabSpec, TabStrip};
pub use titlebar::{TitleBar, TitleBarOutput, WorkspaceSpec};
pub use workspace::{
    WorkspaceButton, WorkspaceItem, WorkspaceMenuOutput, workspace_menu, workspace_menu_with,
};
pub use workspace_menu::workspace_menu_compact;
pub use workspace_selector::WorkspaceSelector;

#[cfg(test)]
mod tabs_tests;
#[cfg(test)]
mod tests;
