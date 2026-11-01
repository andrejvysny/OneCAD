//! The upgrade review dialog (design 15a/15b): what each update keeps, drops and refuses.
use super::{Op, model, review::Review};
use crate::{
    app::App,
    library_pages::kit::{self, ChipKind},
};
use egui::{FontFamily, Ui};
use onecad_app::dto::UpgradeItemDto;
use onecad_widgets::{
    components::overlays::{Dialog, DialogIntent},
    theme::look,
};

pub(crate) fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut review) = app.doc.instances.review.take() else {
        return;
    };
    let busy = app.doc.instances.blocks();
    let reason = super::unavailable(app);
    let names: Vec<(String, String)> = app
        .doc
        .instances
        .items()
        .iter()
        .map(|v| (v.dto.record_id.clone(), model::title(&v.dto)))
        .collect();
    let can_apply = !review.loading && reason.is_none();
    let selected = review.approved_selected().len();
    let all = review.approved_all().len();
    let out = Dialog::new(DialogIntent::Decide, "Review component updates")
        .icon("component")
        .primary("Update selected", can_apply && selected > 0)
        .secondary("Later", true)
        .tertiary("Update all applicable")
        .disabled(busy)
        .width(560.0)
        .show(ctx, egui::Id::unique("component-review"), |ui| {
            body(ui, &mut review, &names);
        });
    let scope = super::scope(app);
    let approved = if out.primary && can_apply {
        Some(review.approved_selected())
    } else if out.tertiary && can_apply && all > 0 {
        Some(review.approved_all())
    } else {
        None
    };
    if out.secondary || out.dismissed {
        return;
    }
    app.doc.instances.review = Some(review);
    if let (Some(approved), Some(scope)) = (approved, scope)
        && !approved.is_empty()
    {
        app.doc
            .instances
            .mutate(&scope, "Update components", Op::Upgrade { approved });
    }
}

fn body(ui: &mut Ui, review: &mut Review, names: &[(String, String)]) {
    let theme = look(ui.ctx()).theme;
    if let Some(notice) = &review.notice {
        kit::label(ui, notice, 12.5, FontFamily::Proportional, theme.warn);
        ui.add_space(6.0);
    }
    if let Some(error) = &review.error {
        kit::label(
            ui,
            &format!("Update failed: {error}"),
            12.5,
            FontFamily::Proportional,
            theme.warn,
        );
        ui.add_space(6.0);
    }
    if review.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            kit::muted(ui, "Planning updates…", 12.5);
        });
        return;
    }
    if review.items.is_empty() {
        kit::muted(ui, "Nothing to update.", 12.5);
    }
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            let items = review.items.clone();
            for item in &items {
                let name = names
                    .iter()
                    .find(|(id, _)| *id == item.record_id)
                    .map_or(item.record_id.as_str(), |(_, name)| name.as_str());
                item_card(ui, review, item, name);
                ui.add_space(8.0);
            }
        });
}

fn item_card(ui: &mut Ui, review: &mut Review, item: &UpgradeItemDto, name: &str) {
    let theme = look(ui.ctx()).theme;
    ui.horizontal(|ui| {
        let mut on = review.selected.contains(&item.record_id);
        let response = ui.add_enabled(item.applicable, egui::Checkbox::new(&mut on, name));
        if response.changed() {
            review.toggle(&item.record_id);
        }
        kit::chip(
            ui,
            &format!("{} → {}", item.from_version, item.to_version),
            ChipKind::Accent,
        );
    });
    if let Some(refusal) = &item.refusal {
        kit::label(
            ui,
            &format!("Not applicable: {refusal}"),
            12.0,
            FontFamily::Proportional,
            theme.ink5,
        );
        return;
    }
    ui.indent(("review-item", &item.record_id), |ui| {
        if !item.kept_overrides.is_empty() {
            let kept: Vec<String> = item
                .kept_overrides
                .iter()
                .map(|(k, v)| format!("{k} = {}", model::text(v)))
                .collect();
            kit::muted(ui, &format!("Keeps: {}", kept.join(", ")), 12.0);
        }
        for dropped in &item.dropped_overrides {
            let text = format!(
                "Drops {} = {}: {}",
                dropped.key,
                model::text(&dropped.value),
                dropped.reason
            );
            kit::label(ui, &text, 12.0, FontFamily::Proportional, theme.warn);
        }
        if let Some(mate) = &item.dropped_mate_attachment {
            let text = format!("Drops the mate attachment: {mate}");
            kit::label(ui, &text, 12.0, FontFamily::Proportional, theme.warn);
        }
    });
}
