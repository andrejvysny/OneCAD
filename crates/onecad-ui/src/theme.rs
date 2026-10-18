//! Colors come from OneCAD's `assets/native/tokens.css` (the sole color source), parsed at
//! build time via `include_str!`: the `@theme` block (light, defaults) overridden by the
//! `:root[data-theme="dark"]` block. No color literal lives in this crate.

use egui::Color32;
use std::collections::HashMap;

const TOKENS_CSS: &str = include_str!("../../../assets/native/tokens.css");

/// Parse `#rrggbb` or `rgb(r g b / a)`.
fn color(v: &str) -> Option<Color32> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let n = u32::from_str_radix(hex.get(..6)?, 16).ok()?;
        return Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8));
    }
    let inner = v.strip_prefix("rgb(")?.strip_suffix(')')?;
    let (rgb, a) = inner.split_once('/').unwrap_or((inner, "1"));
    let c: Vec<u8> = rgb
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    let a: f32 = a.trim().parse().ok()?;
    let [r, g, b] = c[..] else { return None };
    Some(Color32::from_rgba_unmultiplied(
        r,
        g,
        b,
        (a * 255.0).round() as u8,
    ))
}

fn block<'a>(css: &'a str, opener: &str) -> &'a str {
    let Some(start) = css.find(opener) else {
        return "";
    };
    let body = &css[start + opener.len()..];
    &body[..body.find("\n}").unwrap_or(body.len())]
}

fn parse_into(map: &mut HashMap<String, Color32>, body: &str) {
    for decl in body.split(';') {
        // Strip comments that precede the declaration.
        let decl = decl.rsplit("*/").next().unwrap_or(decl);
        if let Some((name, value)) = decl.trim().split_once(':')
            && let Some(name) = name.trim().strip_prefix("--")
            && let Some(c) = color(value)
        {
            map.insert(name.to_owned(), c);
        }
    }
}

pub fn tokens(dark: bool) -> HashMap<String, Color32> {
    let mut map = HashMap::new();
    parse_into(&mut map, block(TOKENS_CSS, "@theme {"));
    if dark {
        parse_into(&mut map, block(TOKENS_CSS, ":root[data-theme=\"dark\"] {"));
    }
    map
}

macro_rules! theme {
    ($($field:ident = $token:literal),* $(,)?) => {
        // A documented subset of tokens.css; not every token is painted yet.
        #[allow(dead_code)]
        #[derive(Clone, Debug)]
        pub struct Theme { $(pub $field: Color32,)* }
        impl Theme {
            pub fn from_tokens(map: &HashMap<String, Color32>) -> Result<Self, String> {
                Ok(Theme { $($field: *map.get($token).ok_or(concat!("missing token --", $token))?,)* })
            }
        }
    };
}

theme! {
    accent = "color-accent",
    accent_hover = "color-accent-hover",
    icon_accent = "color-icon-accent",
    on_accent = "color-on-accent",
    sel_bg = "color-sel-bg",
    sel_text = "color-sel-text",
    canvas = "color-canvas",
    canvas_model = "color-canvas-model",
    grid_minor = "color-viewport-grid-minor",
    grid_major = "color-viewport-grid-major",
    axis_x = "color-axis-x",
    axis_y = "color-axis-y",
    axis_z = "color-axis-z",
    panel = "color-panel",
    statusbar = "color-statusbar",
    titlebar = "color-titlebar",
    titlebar_text = "color-titlebar-text",
    surface = "color-surface",
    border = "color-border",
    border_strong = "color-border-strong",
    border_subtle = "color-border-subtle",
    ink = "color-ink",
    ink2 = "color-ink-2",
    tree_label = "color-tree-label",
    ink3 = "color-ink-3",
    ink4 = "color-ink-4",
    ink5 = "color-ink-5",
    ink6 = "color-ink-6",
    ink7 = "color-ink-7",
    hover = "color-hover",
    hover2 = "color-hover-2",
    hover3 = "color-hover-3",
    chip = "color-chip",
    well = "color-well",
    tooltip = "color-tooltip",
    tooltip_text = "color-tooltip-text",
    traffic_close = "color-traffic-close",
    dof_neutral = "color-dof-neutral",
    body_fill = "color-body-fill",
    body_edge = "color-body-edge",
    viewport_hover = "color-viewport-hover",
}

pub fn load(dark: bool) -> Result<Theme, String> {
    Theme::from_tokens(&tokens(dark))
}

/// Map the tokens onto egui's widget visuals.
pub fn apply(ctx: &egui::Context, t: &Theme, dark: bool) {
    use egui::{CornerRadius, Stroke, style::Selection};
    let theme = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    ctx.style_mut_of(theme, |s| {
        let v = &mut s.visuals;
        v.panel_fill = t.panel;
        v.window_fill = t.surface;
        v.extreme_bg_color = t.well;
        v.faint_bg_color = t.hover;
        v.override_text_color = Some(t.ink2);
        v.window_stroke = Stroke::new(1.0, t.border);
        v.selection = Selection {
            bg_fill: t.sel_bg,
            stroke: Stroke::new(1.0, t.sel_text),
        };
        v.hyperlink_color = t.accent;
        v.text_cursor.stroke = Stroke::new(1.5, t.accent);
        let r = CornerRadius::same(6);
        for (w, fill) in [
            (&mut v.widgets.noninteractive, t.panel),
            (&mut v.widgets.inactive, t.chip),
            (&mut v.widgets.hovered, t.hover2),
            (&mut v.widgets.active, t.hover3),
            (&mut v.widgets.open, t.hover2),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.corner_radius = r;
            w.bg_stroke = Stroke::new(1.0, t.border);
            w.fg_stroke = Stroke::new(1.0, t.ink3);
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn dark_tokens_override_light() {
        let light = super::tokens(false);
        let dark = super::tokens(true);
        assert_eq!(
            light["color-panel"],
            egui::Color32::from_rgb(0xfa, 0xfa, 0xfb)
        );
        assert_eq!(
            dark["color-panel"],
            egui::Color32::from_rgb(0x20, 0x23, 0x27)
        );
        assert!(super::Theme::from_tokens(&dark).is_ok());
    }
}
