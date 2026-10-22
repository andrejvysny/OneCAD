use super::{PresentationContext, State, ViewAction};
use crate::{app::App, ui_state::StartPage};
use egui::Ui;

pub(crate) fn show(app: &mut App, ui: &mut Ui) {
    if !app.ui.extensions.native.attempted && !app.busy() {
        app.extension_request(super::Request::Refresh);
    }
    let context = PresentationContext {
        idle: !app.busy() && !app.ui.extensions.native.pending,
        document: crate::bars::export_allowed(app),
        selection: app
            .doc
            .selection
            .items()
            .iter()
            .any(|item| item.body().is_some()),
    };
    let action = super::render(
        ui,
        &mut app.ui.extensions.native,
        app.ui.start_page,
        context,
    );
    if let Some(page) = action.page {
        app.ui.start_page = page;
    }
    if let Some(request) = action.request {
        app.extension_request(request);
    }
    if let Some(command) = action.command {
        app.run_extension_command(&command);
    }
}

/// The production page renderer, also used by headless visual and interaction tests.
pub(crate) fn render(
    ui: &mut Ui,
    state: &mut State,
    page: StartPage,
    context: PresentationContext,
) -> ViewAction {
    let mut action = ViewAction::default();
    if page == StartPage::ExtensionDetail {
        super::detail::show(ui, state, context, &mut action);
    } else {
        super::manager::show(ui, state, context, &mut action);
    }
    if let Some(request) = super::consent::show(ui.ctx(), state, context.idle) {
        action.request = Some(request);
    }
    action
}
