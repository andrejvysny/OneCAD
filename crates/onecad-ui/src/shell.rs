//! Editor chrome orchestrator: title bar plus the floating panels and overlays.
use crate::{
    app::App, bars, explorer, feature_card, inspector, layout, overlays, pill, session_strip,
};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    crate::sketch_repair::set_constraint_hover(app, &ctx, None);
    if !crate::input::overlay_owns_keyboard(app, &ctx) {
        crate::ui_state::shortcuts(&mut app.ui, &ctx);
    }
    let chrome = layout::chrome(ctx.content_rect(), &app.ui);
    let chrome = if app
        .interaction
        .tool
        .as_ref()
        .is_some_and(|tool| !tool.editing_existing)
        && !app.sketch_active()
    {
        chrome.with_feature_card()
    } else {
        chrome
    };
    bars::title(app, ui);
    app.diagnostics.observe(&app.status);
    app.diagnostics.show(&ctx);
    crate::document_items::show(app, &ctx);
    if app.ui.start_open {
        return;
    }
    crate::workspace_ui::customize_dialog(app, &ctx);
    explorer::show(app, &ctx, &chrome);
    inspector::show(app, &ctx, &chrome);
    pill::show(app, &ctx, &chrome);
    crate::editor_facade::overlays(app, &ctx, &chrome);
    feature_card::show(app, &ctx, &chrome);
    if app.sketch_active() {
        session_strip::show(app, &ctx, &chrome);
    }
    overlays::nav_pill(app, &ctx, &chrome);
    overlays::view_cluster(app, &ctx, &chrome);
    crate::sketch_numeric::show(app, &ctx);
    app.diagnostics.observe(&app.status);
}

pub fn close_prompt(app: &mut App, ctx: &egui::Context) {
    crate::document_flow::close_prompt(app, ctx);
}
