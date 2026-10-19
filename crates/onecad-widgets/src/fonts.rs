//! System SF fonts, resolved at runtime through CoreText (never bundled). The SF file is a
//! variable font; egui 0.36 can pin `wght` per FontData, so one file gives the three weights
//! tokens.css uses (400 / medium 510 / semibold 590 — Apple's SF weight values).

use egui::{FontData, FontDefinitions, FontFamily, FontTweak};
use std::path::PathBuf;
use std::sync::Arc;

pub const MEDIUM: &str = "sf-medium";
pub const SEMIBOLD: &str = "sf-semibold";

#[cfg(target_os = "macos")]
fn ui_font_path(kind: objc2_core_text::CTFontUIFontType) -> Option<PathBuf> {
    use objc2_core_foundation::CFURL;
    use objc2_core_text::{CTFont, kCTFontURLAttribute};
    // SAFETY: plain CoreText getters on a font CoreText itself created; no pointers escape.
    unsafe {
        let font = CTFont::new_ui_font_for_language(kind, 13.0, None)?;
        let url = font
            .attribute(kCTFontURLAttribute)?
            .downcast::<CFURL>()
            .ok()?;
        url.to_file_path()
    }
}

#[cfg(not(target_os = "macos"))]
fn ui_font_path(_kind: ()) -> Option<PathBuf> {
    None
}

pub struct Resolved {
    pub ui: Option<PathBuf>,
    pub mono: Option<PathBuf>,
}

pub fn resolve() -> Resolved {
    #[cfg(target_os = "macos")]
    let (ui, fixed) = {
        use objc2_core_text::CTFontUIFontType;
        (
            ui_font_path(CTFontUIFontType::System),
            ui_font_path(CTFontUIFontType::UserFixedPitch),
        )
    };
    #[cfg(not(target_os = "macos"))]
    let (ui, fixed) = (ui_font_path(()), None);
    // CoreText has no UI-font type for SF Mono (AppKit's monospacedSystemFont); prefer the
    // system SF Mono file when the user fixed-pitch font is something else.
    let sf_mono = PathBuf::from("/System/Library/Fonts/SFNSMono.ttf");
    let mono = if sf_mono.is_file() {
        Some(sf_mono)
    } else {
        fixed
    };
    Resolved { ui, mono }
}

/// One leaked copy of the font file shared by every weight (the file is ~8 MB; three owned
/// copies would triple that in RSS).
fn weighted(bytes: &'static [u8], wght: f32) -> FontData {
    let mut tweak = FontTweak::default();
    tweak.coords.push(b"wght", wght);
    // SFNS.ttf defaults to opsz 28 (the Display cut). CoreText picks the optical size from
    // the point size, and every UI size here (11-15 px) clamps to the axis minimum, 17 (the
    // Text cut). Without this, egui text sets about 10% narrower than WebKit's.
    tweak.coords.push(b"opsz", 17.0);
    FontData::from_static(bytes).tweak(tweak)
}

pub fn install(ctx: &egui::Context) -> Resolved {
    let r = resolve();
    let mut defs = FontDefinitions::default();
    if let Some(bytes) = r.ui.as_ref().and_then(|p| std::fs::read(p).ok()) {
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        defs.font_data
            .insert("sf-regular".into(), Arc::new(weighted(bytes, 400.0)));
        defs.font_data
            .insert(MEDIUM.into(), Arc::new(weighted(bytes, 510.0)));
        defs.font_data
            .insert(SEMIBOLD.into(), Arc::new(weighted(bytes, 590.0)));
        defs.families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "sf-regular".into());
        for name in [MEDIUM, SEMIBOLD] {
            let mut list = vec![name.to_owned()];
            list.extend(
                defs.families[&FontFamily::Proportional]
                    .iter()
                    .skip(1)
                    .cloned(),
            );
            defs.families.insert(FontFamily::Name(name.into()), list);
        }
    } else {
        for name in [MEDIUM, SEMIBOLD] {
            defs.families.insert(
                FontFamily::Name(name.into()),
                defs.families[&FontFamily::Proportional].clone(),
            );
        }
    }
    if let Some(bytes) = r.mono.as_ref().and_then(|p| std::fs::read(p).ok()) {
        defs.font_data
            .insert("sf-mono".into(), Arc::new(FontData::from_owned(bytes)));
        defs.families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "sf-mono".into());
    }
    ctx.set_fonts(defs);
    r
}

pub fn medium() -> FontFamily {
    FontFamily::Name(MEDIUM.into())
}

pub fn semibold() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

pub fn mono() -> FontFamily {
    FontFamily::Monospace
}
