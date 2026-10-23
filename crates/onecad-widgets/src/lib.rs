//! Presentation-only crate: design tokens, fonts, icons and UI components. No onecad
//! dependencies; components take a `&Theme` or read the installed [`theme::Look`].
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
pub mod components;
pub mod fonts;
pub mod frame;
pub(crate) mod glyph;
pub mod icon;
pub mod metrics;
pub mod text_session;
pub mod theme;

pub use theme::Theme;
