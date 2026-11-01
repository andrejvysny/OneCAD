//! Explorer COMPONENTS section: one row per placed instance with link and status badges.
use super::model::{self, Tone};
use crate::{
    app::App,
    library_pages::kit::{self, ChipKind},
};
use egui::Ui;
use onecad_app::dto::ComponentInstanceDto;
use onecad_widgets::components::lists::{RowState, SectionLabel, TreeRow};

fn kind(tone: Tone) -> ChipKind {
    match tone {
        Tone::Neutral => ChipKind::Plain,
        Tone::Accent => ChipKind::Accent,
    }
}

pub(crate) fn section(app: &mut App, ui: &mut Ui, filter: &str) {
    if app.doc.instances.items().is_empty() {
        if let Some(error) = app.doc.instances.list_error() {
            ui.add(SectionLabel::new("COMPONENTS"));
            kit::muted(ui, &format!("Components unavailable: {error}"), 11.5);
        }
        return;
    }
    let rows: Vec<ComponentInstanceDto> = app
        .doc
        .instances
        .items()
        .iter()
        .map(|view| view.dto.clone())
        .filter(|dto| {
            filter.is_empty()
                || model::title(dto)
                    .to_lowercase()
                    .contains(&filter.to_lowercase())
        })
        .collect();
    ui.add(SectionLabel::new("COMPONENTS"));
    review_action(app, ui);
    for dto in &rows {
        row(app, ui, dto);
    }
}

fn review_action(app: &mut App, ui: &mut Ui) {
    let count = app.doc.instances.upgradable().len();
    if count == 0 {
        return;
    }
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let noun = if count == 1 { "update" } else { "updates" };
        if kit::link(ui, &format!("Review {count} {noun}"), 12.0).clicked()
            && let Some(scope) = super::scope(app)
        {
            app.doc.instances.open_review(&scope);
        }
    });
}

fn row(app: &mut App, ui: &mut Ui, dto: &ComponentInstanceDto) {
    let selected = app
        .doc
        .selected_feature()
        .is_some_and(|id| id.to_string() == dto.record_id);
    let enabled = app.viewport_ready() && app.doc.bridge.valid() && !app.busy();
    let title = model::title(dto);
    let state = if selected {
        RowState::Selected
    } else {
        RowState::Normal
    };
    let response = ui.add(
        TreeRow::new("component", &title)
            .state(state)
            .enabled(enabled),
    );
    ui.horizontal(|ui| {
        ui.add_space(30.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        let (text, tone) = model::link_badge(dto);
        kit::chip(ui, text, kind(tone));
        if let Some(text) = model::status_badge(&dto.status) {
            kit::chip(ui, text, ChipKind::Warn);
        }
    });
    ui.add_space(2.0);
    if response.clicked()
        && enabled
        && let Ok(id) = dto.record_id.parse()
    {
        crate::pill::select_feature(app, id);
        app.ui.inspector_open = true;
        app.ui.inspector_tab = crate::ui_state::InspectorTab::Properties;
    }
}
