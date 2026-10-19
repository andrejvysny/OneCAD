//! Shell chrome (g02): title bar, floating panel frame, panel toggles, workspace switcher
//! and status bar. Cards c-titlebar, c-panel, c-ptoggle, c-ws, c-status.

mod icon_button;
mod panel;
mod status;
mod titlebar;
mod workspace;

pub(crate) use icon_button::{Backdrop, IconButton};
pub use panel::{FloatingPanel, PanelToggle, Side};
pub use status::{StatusBar, StatusItem, StatusOutput};
pub use titlebar::{TitleBar, TitleBarOutput};
pub use workspace::{WorkspaceButton, WorkspaceItem, WorkspaceMenuOutput, workspace_menu};

#[cfg(test)]
mod tests;
