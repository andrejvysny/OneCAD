//! Layout and type constants from docs/design/native-ui/final/README.md ("Design tokens").

use egui::{Color32, epaint::Shadow};

use crate::theme::Theme;

pub const RADIUS_SM: u8 = 6;
pub const RADIUS_MD: u8 = 8;
pub const RADIUS_LG: u8 = 10;
pub const RADIUS_XL: u8 = 12;

pub const H_TOOLTIP: f32 = 24.0;
pub const H_CHIP: f32 = 28.0;
pub const H_FIELD: f32 = 30.0;
pub const H_ROW: f32 = 32.0;
pub const H_TOOL: f32 = 34.0;
pub const H_SQUARE: f32 = 36.0;
pub const H_TITLEBAR: f32 = 44.0;
pub const H_STATUSBAR: f32 = 28.0;

pub const S4: f32 = 4.0;
pub const S6: f32 = 6.0;
pub const S8: f32 = 8.0;
pub const S10: f32 = 10.0;
pub const S12: f32 = 12.0;
pub const S16: f32 = 16.0;
pub const S20: f32 = 20.0;
pub const S24: f32 = 24.0;

pub const EDGE_INSET: f32 = 10.0;
pub const CHROME_GAP: f32 = 8.0;

pub const TITLE: f32 = 15.0;
pub const PANEL_TITLE: f32 = 14.0;
pub const BODY: f32 = 13.0;
pub const CONTROL: f32 = 12.5;
pub const SECONDARY: f32 = 12.0;
pub const SECTION: f32 = 11.0;
/// 0.07em of `SECTION`.
pub const SECTION_TRACKING: f32 = 0.77;
pub const MONO_VALUE: f32 = 12.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elevation {
    Pill,
    Panel,
    Card,
    Tooltip,
    /// Start-screen recent card hover: blue-tinted `--shadow-card-hover`.
    CardHover,
}

/// Drop shadow per elevation, tinted by the theme's `shadow` token; dark themes use
/// alpha 0.55 throughout (tokens.css).
pub fn shadow(kind: Elevation, t: &Theme, dark: bool) -> Shadow {
    let (y, blur, a) = match kind {
        Elevation::Pill => (2, 10, 0.08),
        Elevation::Panel => (6, 20, 0.08),
        Elevation::Card => (14, 40, 0.16),
        Elevation::Tooltip => (2, 6, 0.2),
        Elevation::CardHover => (3, 10, 0.1),
    };
    let alpha: f32 = if dark { 0.55 } else { a };
    let color = if kind == Elevation::CardHover && !dark {
        Color32::from_rgba_unmultiplied(30, 60, 120, (alpha * 255.0).round() as u8)
    } else {
        t.shadow.gamma_multiply(alpha)
    };
    Shadow {
        offset: [0, y],
        blur,
        spread: 0,
        color,
    }
}

/// Focus ring: 3px accent at 15% alpha.
pub fn focus_ring(t: &Theme) -> (f32, Color32) {
    (3.0, t.accent.gamma_multiply(0.15))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_shadows_are_stronger() {
        let t = Theme::filled(Color32::BLACK);
        for k in [
            Elevation::Pill,
            Elevation::Panel,
            Elevation::Card,
            Elevation::Tooltip,
            Elevation::CardHover,
        ] {
            assert!(shadow(k, &t, true).color.a() > shadow(k, &t, false).color.a());
        }
    }
}
