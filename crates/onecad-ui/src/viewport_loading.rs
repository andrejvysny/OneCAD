//! Project feedback stays visible before a drawable scene is available.
use crate::app::App;

pub(crate) const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

pub(crate) fn repaint(app: &App, context: &egui::Context) {
    let hidden = context.input(|input| {
        input.viewport().occluded == Some(true) || input.viewport().minimized == Some(true)
    });
    if pending(app) && !hidden {
        // Background delivery must progress before the first drawable frame.
        context.request_repaint_after(POLL_INTERVAL);
    }
}

pub(crate) fn pending(app: &App) -> bool {
    !app.ui.start_open
        && !app.stopping
        && !app.doc.stopped
        && app.doc.display_error.is_none()
        && app.viewport_failure.is_none()
        && app.resize_refusal.is_none()
        && (app.doc.display.is_none() || app.doc.awaiting_scene || app.texture.is_none())
}

fn message(app: &App, preparing_view: bool) -> (&str, &str, bool) {
    let error = app
        .doc
        .display_error
        .as_deref()
        .or(app.viewport_failure.as_deref())
        .or(app.resize_refusal.as_ref().map(|(_, error)| error.as_str()));
    match error {
        Some(error) => ("Model unavailable", error, false),
        None if app.stopping => (
            "Closing project…",
            "Finishing pending project operations.",
            false,
        ),
        None if !pending(app) => ("Project closed", "Open a project to continue.", false),
        None if preparing_view => (
            "Preparing view…",
            "Your project is open. Preparing its display.",
            true,
        ),
        None => (
            "Opening project…",
            "Preparing your model for editing. Larger projects can take a moment.",
            true,
        ),
    }
}

pub(crate) fn show(app: &App, ui: &mut egui::Ui, rect: egui::Rect, preparing_view: bool) {
    let (title, detail, animated) = message(app, preparing_view);
    let theme = &app.theme;
    let content = egui::Rect::from_center_size(
        rect.center(),
        egui::vec2(rect.width().min(420.0), rect.height().min(180.0)),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            if animated {
                ui.add(egui::Spinner::new().size(24.0).color(theme.accent));
            }
            ui.label(egui::RichText::new(title).size(20.0).color(theme.ink));
            ui.add(
                egui::Label::new(egui::RichText::new(detail).color(theme.ink3))
                    .wrap()
                    .halign(egui::Align::Center),
            );
        },
    );
}
