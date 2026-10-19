//! Baked-SVG icon renderer. SVGs come from `assets/icons/{light,dark}.json`
//! (scripts/bake-cad-icons.ts); they are rasterised with resvg at the target pixel size,
//! tinted, and cached as egui textures in the context's temp data.

use egui::{Color32, ColorImage, Rect, TextureHandle, TextureOptions, Vec2, pos2};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use crate::theme::look;

type Svgs = HashMap<String, HashMap<u32, String>>;

struct Registry {
    icons: Svgs,
    sizes: Vec<u32>,
    gap: String,
}

fn parse(json: &str) -> Result<Registry, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let sizes = v["sizes"]
        .as_array()
        .ok_or("sizes")?
        .iter()
        .filter_map(|s| s.as_u64().map(|n| n as u32))
        .collect();
    let gap = v["gap"].as_str().ok_or("gap")?.to_owned();
    let mut icons = Svgs::new();
    for (name, by_size) in v["icons"].as_object().ok_or("icons")? {
        let m = by_size.as_object().ok_or("icon sizes")?;
        let m = m
            .iter()
            .filter_map(|(k, s)| Some((k.parse().ok()?, s.as_str()?.to_owned())))
            .collect();
        icons.insert(name.clone(), m);
    }
    Ok(Registry { icons, sizes, gap })
}

static LIGHT: LazyLock<Result<Registry, String>> =
    LazyLock::new(|| parse(include_str!("../assets/icons/light.json")));
static DARK: LazyLock<Result<Registry, String>> =
    LazyLock::new(|| parse(include_str!("../assets/icons/dark.json")));

fn registry(dark: bool) -> Option<&'static Registry> {
    if dark { DARK.as_ref() } else { LIGHT.as_ref() }.ok()
}

/// Sorted icon names (light registry; both registries share names).
pub fn names() -> Vec<&'static str> {
    let mut v: Vec<&str> = registry(false)
        .map(|r| r.icons.keys().map(String::as_str).collect())
        .unwrap_or_default();
    v.sort_unstable();
    v
}

pub fn exists(name: &str) -> bool {
    registry(false).is_some_and(|r| r.icons.contains_key(name))
}

/// Baked size to use for a requested size: exact if baked, else nearest within the
/// small {14..20} or large {24,32} family.
pub fn bucket(size: u32) -> u32 {
    const ALL: [u32; 6] = [14, 16, 18, 20, 24, 32];
    if ALL.contains(&size) {
        return size;
    }
    let family: &[u32] = if size <= 20 { &ALL[..4] } else { &ALL[4..] };
    family
        .iter()
        .copied()
        .min_by_key(|b| b.abs_diff(size))
        .unwrap_or(24)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IconStyle {
    pub tint: Color32,
    pub bg: Color32,
    pub grey: bool,
}

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

fn grey_out(img: &mut ColorImage) {
    for p in &mut img.pixels {
        let l = (0.299 * f32::from(p.r()) + 0.587 * f32::from(p.g()) + 0.114 * f32::from(p.b()))
            .round() as u8;
        *p = Color32::from_rgba_premultiplied(l, l, l, p.a());
    }
}

/// Rasterise to `px`×`px`; `None` when the icon is unknown or the SVG does not parse.
pub(crate) fn rasterise(name: &str, dark: bool, px: u32, style: IconStyle) -> Option<ColorImage> {
    use resvg::{tiny_skia, usvg};
    let reg = registry(dark)?;
    let b = bucket(px);
    if !reg.sizes.contains(&b) {
        return None;
    }
    let svg = reg.icons.get(name)?.get(&b)?;
    let svg = svg
        .replace("currentColor", &hex(style.tint))
        .replace(&reg.gap, &hex(style.bg));
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let s = px as f32 / 24.0;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(s, s),
        &mut pixmap.as_mut(),
    );
    let mut img = ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data());
    if style.grey {
        grey_out(&mut img);
    }
    Some(img)
}

type Key = (String, bool, u32, u32, [u8; 4], [u8; 4], bool);

#[derive(Clone, Default)]
struct IconCache(Arc<Mutex<HashMap<Key, TextureHandle>>>);

fn texture(
    ctx: &egui::Context,
    name: &str,
    dark: bool,
    px: u32,
    style: IconStyle,
) -> Option<egui::TextureId> {
    let key: Key = (
        name.to_owned(),
        dark,
        bucket(px),
        px,
        style.tint.to_array(),
        style.bg.to_array(),
        style.grey,
    );
    let cache = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<IconCache>(egui::Id::unique("onecad_icon_cache"))
            .clone()
    });
    let mut map = cache.0.lock().ok()?;
    if let Some(t) = map.get(&key) {
        return Some(t.id());
    }
    let img = rasterise(name, dark, px, style)?;
    let handle = ctx.load_texture(format!("icon/{name}/{px}"), img, TextureOptions::LINEAR);
    let id = handle.id();
    map.insert(key, handle);
    Some(id)
}

/// Paint `name` into `rect`; unknown/invalid icons draw an outline in the tint.
pub fn paint(
    painter: &egui::Painter,
    rect: Rect,
    name: &str,
    dark: bool,
    style: IconStyle,
    opacity: f32,
) {
    let px = (rect.width() * painter.ctx().pixels_per_point())
        .round()
        .max(1.0) as u32;
    match texture(painter.ctx(), name, dark, px, style) {
        Some(id) => {
            painter.image(
                id,
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(opacity),
            );
        }
        None if exists(name) => {}
        None => {
            painter.rect_stroke(
                rect.shrink(1.0),
                1.0,
                egui::Stroke::new(1.0, style.tint.gamma_multiply(opacity)),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Icon widget; unset colours come from the installed [`crate::theme::Look`].
pub struct Icon {
    name: String,
    size: f32,
    dark: Option<bool>,
    tint: Option<Color32>,
    bg: Option<Color32>,
    disabled: bool,
}

impl Icon {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            size: 16.0,
            dark: None,
            tint: None,
            bg: None,
            disabled: false,
        }
    }
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
    pub fn dark(mut self, dark: bool) -> Self {
        self.dark = Some(dark);
        self
    }
    pub fn tint(mut self, tint: Color32) -> Self {
        self.tint = Some(tint);
        self
    }
    pub fn bg(mut self, bg: Color32) -> Self {
        self.bg = Some(bg);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl egui::Widget for Icon {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(self.size), egui::Sense::hover());
        let l = look(ui.ctx());
        let style = IconStyle {
            tint: self.tint.unwrap_or(l.theme.ink3),
            bg: self.bg.unwrap_or(l.theme.surface),
            grey: self.disabled,
        };
        let opacity = if self.disabled { 0.4 } else { 1.0 };
        paint(
            ui.painter(),
            rect,
            &self.name,
            self.dark.unwrap_or(l.dark),
            style,
            opacity,
        );
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: IconStyle = IconStyle {
        tint: Color32::BLACK,
        bg: Color32::WHITE,
        grey: false,
    };

    #[test]
    fn registries_complete() {
        for dark in [false, true] {
            let reg = registry(dark).is_some();
            assert!(reg, "registry parse failed (dark={dark})");
            if let Some(reg) = registry(dark) {
                assert_eq!(reg.sizes, [14, 16, 18, 20, 24, 32]);
                for (name, by) in &reg.icons {
                    for s in &reg.sizes {
                        assert!(by.contains_key(s), "{name} missing {s} (dark={dark})");
                    }
                }
            }
        }
        assert!(exists("extrude") && exists("panelLeft") && !exists("nope"));
        assert_eq!(names().len(), 154);
    }

    #[test]
    fn bucket_table() {
        for (i, o) in [
            (14, 14),
            (15, 14),
            (17, 16),
            (19, 18),
            (20, 20),
            (22, 24),
            (23, 24),
            (24, 24),
            (28, 24),
            (31, 32),
            (40, 32),
            (8, 14),
        ] {
            assert_eq!(bucket(i), o, "bucket({i})");
        }
    }

    #[test]
    fn rasterises_extrude() {
        let img = rasterise("extrude", false, 24, STYLE);
        assert!(img.is_some_and(|i| i.pixels.iter().any(|p| p.a() > 200)));
        assert!(rasterise("nope", false, 24, STYLE).is_none());
    }

    #[test]
    fn grey_removes_chroma() {
        let s = IconStyle {
            tint: Color32::from_rgb(200, 30, 30),
            grey: true,
            ..STYLE
        };
        let img = rasterise("extrude", false, 24, s);
        assert!(img.is_some_and(|i| i.pixels.iter().all(|p| p.r() == p.g() && p.g() == p.b())));
    }
}
