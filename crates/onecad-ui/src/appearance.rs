//! View › Appearance: resolves Light / Dark / System and re-installs the theme.
use crate::{app::App, theme};

/// Re-install the theme when the resolved darkness differs from the current one.
pub fn sync(app: &mut App, ctx: &egui::Context) {
    let dark = app.ui.appearance.resolve_dark(ctx.system_theme());
    if dark == app.dark || app.ui.theme_failed == Some(dark) {
        return;
    }
    let loaded = match theme::load(dark) {
        Ok(loaded) => loaded,
        Err(error) => {
            app.ui.theme_failed = Some(dark);
            app.status = format!("Theme failed to load: {error}");
            return;
        }
    };
    app.ui.theme_failed = None;
    theme::install(ctx, loaded.clone(), dark);
    app.theme = loaded;
    app.dark = dark;
    app.draw_dirty = true;
}
