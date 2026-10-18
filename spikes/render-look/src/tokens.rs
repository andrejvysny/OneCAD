//! Viewport colours read at runtime from `src/styles/tokens.css`, which stays the single colour
//! source (no hex literal lives in this crate). The light theme is the `@theme { ... }` block; the
//! dark theme is the `:root[data-theme="dark"] { ... }` block, which overrides it.

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn name(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

/// sRGB 0..1 colour as authored in tokens.css.
#[derive(Clone, Copy, Debug)]
pub struct Srgb(pub [f32; 3]);

impl Srgb {
    pub fn linear(self) -> [f32; 3] {
        self.0.map(|c| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) })
    }
    pub fn linear4(self) -> [f32; 4] {
        let l = self.linear();
        [l[0], l[1], l[2], 1.0]
    }
}

#[derive(Clone)]
pub struct Palette {
    pub canvas: Srgb,
    pub grid_minor: Srgb,
    pub grid_major: Srgb,
    pub body_fill: Srgb,
    pub body_edge: Srgb,
}

fn block<'a>(css: &'a str, opener: &str) -> Result<&'a str, String> {
    let start = css.find(opener).ok_or_else(|| format!("tokens.css: no `{opener}` block"))?;
    let body = &css[start + opener.len()..];
    let end = body.find("\n}").ok_or_else(|| format!("tokens.css: unterminated `{opener}`"))?;
    Ok(&body[..end])
}

fn hex_vars(block: &str) -> HashMap<String, Srgb> {
    let mut out = HashMap::new();
    for line in block.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("--") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_end_matches(';').trim();
        let Some(hex) = value.strip_prefix('#') else {
            continue;
        };
        if hex.len() != 6 {
            continue;
        }
        let Ok(v) = u32::from_str_radix(hex, 16) else {
            continue;
        };
        let c = |s: u32| ((v >> s) & 0xff) as f32 / 255.0;
        out.insert(format!("--{}", name.trim()), Srgb([c(16), c(8), c(0)]));
    }
    out
}

impl Palette {
    pub fn load(tokens_css: &Path, theme: Theme) -> Result<Palette, String> {
        let css = std::fs::read_to_string(tokens_css).map_err(|e| format!("{}: {e}", tokens_css.display()))?;
        let mut vars = hex_vars(block(&css, "@theme {")?);
        if theme == Theme::Dark {
            vars.extend(hex_vars(block(&css, ":root[data-theme=\"dark\"] {")?));
        }
        let get = |n: &str| vars.get(n).copied().ok_or_else(|| format!("tokens.css: no {n} for {}", theme.name()));
        Ok(Palette {
            canvas: get("--color-canvas")?,
            grid_minor: get("--color-viewport-grid-minor")?,
            grid_major: get("--color-viewport-grid-major")?,
            body_fill: get("--color-body-fill")?,
            body_edge: get("--color-body-edge")?,
        })
    }
}
