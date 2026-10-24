//! Section order is authored data; display names never substitute for region identity.
use crate::app::App;
use onecad_core::{
    document::{
        record::{KnownOperation, LOFT_MAX_PROFILES, LoftParams, Operation},
        refs::SketchRegionRef,
    },
    ids::RecordId,
};
use onecad_interaction::{
    DocumentContext, ModelSession, ModelingOptions, SessionPhase, modeling_records,
    operand_binding, selection_set::SelectionItem,
};
use onecad_widgets::components::overlays::FeatureCardRows;

pub(crate) fn picking(app: &App) -> bool {
    app.doc.interaction.tool.as_ref().is_some_and(|tool| {
        tool.phase == SessionPhase::Editing
            && matches!(tool.record.op, Operation::Known(KnownOperation::Loft(_)))
    })
}

fn finalized(app: &App, profile: &SketchRegionRef) -> bool {
    app.doc.display.as_ref().is_some_and(|display| {
        display.records.iter().any(|record| {
            !record.suppressed && matches!(&record.op, Operation::Known(KnownOperation::Sketch(p)) if p.sketch == profile.sketch)
        }) && display.profiles.iter().any(|p| same(&p.reference, profile))
    })
}

fn selected(app: &App) -> Result<Vec<SketchRegionRef>, String> {
    if app.sketch_active() {
        return Err("Finish sketch editing before choosing Loft sections".into());
    }
    let context = app
        .doc
        .interaction
        .context
        .as_ref()
        .ok_or("No current document")?;
    selected_profiles(app.doc.selection.items(), context, |p| finalized(app, p))
}

fn selected_profiles(
    items: &[SelectionItem],
    context: &DocumentContext,
    finalized: impl Fn(&SketchRegionRef) -> bool,
) -> Result<Vec<SketchRegionRef>, String> {
    items
        .iter()
        .map(|item| match item {
            SelectionItem::Profile(p)
                if &p.context == context
                    && p.capture.scene == context.scene
                    && finalized(&p.profile) =>
            {
                Ok(p.profile.clone())
            }
            _ => Err("Select current finalized closed sketch profiles only".into()),
        })
        .collect()
}

pub(crate) fn ready(app: &App) -> bool {
    selected(app).is_ok_and(|profiles| (2..=LOFT_MAX_PROFILES).contains(&profiles.len()))
}

#[cfg(test)]
#[path = "loft_ui_tests.rs"]
mod tests;

pub(crate) fn begin(app: &mut App) {
    let Some(context) = app.doc.interaction.context.clone() else {
        return;
    };
    let step = app
        .doc
        .display
        .as_ref()
        .map_or(0, |d| d.projection.applied_ops as u32);
    let result =
        selected(app).and_then(|profiles| modeling_records::loft(profiles, RecordId::new(), step));
    app.doc.gesture += 1;
    match result.and_then(|record| operand_binding::begin_bound(record, app.doc.gesture, context)) {
        Ok(action) => app.dispatch(action),
        Err(error) => app.doc.status = error,
    }
    app.draw_dirty = true;
}

fn same(a: &SketchRegionRef, b: &SketchRegionRef) -> bool {
    a.sketch == b.sketch && a.region == b.region
}

fn label(app: &App, profile: &SketchRegionRef) -> String {
    let Some(display) = &app.doc.display else {
        return "Unavailable section".into();
    };
    let name = display
        .projection
        .sketches
        .get(&profile.sketch.to_string())
        .map_or("Missing sketch", |s| s.name.as_str());
    let ordinal = display
        .profiles
        .iter()
        .filter(|p| p.reference.sketch == profile.sketch)
        .position(|p| same(&p.reference, profile));
    ordinal.map_or_else(
        || format!("{name}: missing region"),
        |i| format!("{name}: region {}", i + 1),
    )
}

pub(crate) fn rows(
    app: &App,
    rows: &mut FeatureCardRows,
    _tool: &ModelSession,
    p: &LoftParams,
) -> Option<ModelingOptions> {
    let labels: Vec<_> = p.profiles.iter().map(|p| label(app, p)).collect();
    section_fields(rows, p, &labels, &selected(app).unwrap_or_default())
}

fn section_fields(
    rows: &mut FeatureCardRows,
    p: &LoftParams,
    labels: &[String],
    selection: &[SketchRegionRef],
) -> Option<ModelingOptions> {
    let mut profiles = p.profiles.clone();
    let mut ruled = p.is_ruled;
    rows.row("Surface", |ui| {
        let response = ui.checkbox(&mut ruled, "Ruled");
        crate::automation::name(ui.ctx(), response.id, "onecad.option.loftRuled");
    });
    let replace = (selection.len() == 1).then(|| &selection[0]);
    for (index, label) in labels.iter().enumerate() {
        rows.row(&format!("Section {}", index + 1), |ui| {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(label).wrap());
                section_controls(ui, &mut profiles, index, replace);
            });
        });
    }
    add_selected(rows, &mut profiles, selection);
    for caption in [
        "Pick profiles to add or replace. Sections require parallel distinct planes in forward or reverse order.",
        "Closed regions only. Holes, points, guide rails and open curves are unsupported.",
    ] {
        rows.row("", |ui| {
            ui.add(egui::Label::new(egui::RichText::new(caption).small()).wrap());
        });
    }
    (profiles != p.profiles || ruled != p.is_ruled).then_some(ModelingOptions::Loft {
        profiles,
        is_ruled: ruled,
    })
}

fn add_selected(
    rows: &mut FeatureCardRows,
    profiles: &mut Vec<SketchRegionRef>,
    selection: &[SketchRegionRef],
) {
    rows.row("Sections", |ui| {
        let can_add = !selection.is_empty()
            && profiles.len() + selection.len() <= LOFT_MAX_PROFILES
            && selection
                .iter()
                .all(|s| !profiles.iter().any(|p| same(p, s)));
        let response = ui.add_enabled(can_add, egui::Button::new("Add selected profiles"));
        crate::automation::name(ui.ctx(), response.id, "onecad.option.loftAddSelected");
        if response.clicked() {
            profiles.extend_from_slice(selection);
        }
    });
}

fn section_controls(
    ui: &mut egui::Ui,
    profiles: &mut Vec<SketchRegionRef>,
    index: usize,
    replacement: Option<&SketchRegionRef>,
) {
    if index >= profiles.len() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (suffix, text, enabled) in [
            ("up", "Up", index > 0),
            ("down", "Down", index + 1 < profiles.len()),
            ("remove", "Remove", true),
        ] {
            let response = ui.add_enabled(enabled, egui::Button::new(text));
            crate::automation::name(
                ui.ctx(),
                response.id,
                &format!("onecad.option.loftSection.{index}.{suffix}"),
            );
            if response.clicked() {
                match suffix {
                    "up" => profiles.swap(index, index - 1),
                    "down" => profiles.swap(index, index + 1),
                    _ => {
                        profiles.remove(index);
                    }
                }
                return;
            }
        }
        let allowed = replacement.is_some_and(|r| {
            profiles
                .iter()
                .enumerate()
                .all(|(i, p)| i == index || !same(p, r))
        });
        let response = ui.add_enabled(allowed, egui::Button::new("Use selected"));
        crate::automation::name(
            ui.ctx(),
            response.id,
            &format!("onecad.option.loftSection.{index}.replace"),
        );
        if response.clicked()
            && let Some(replacement) = replacement
        {
            profiles[index] = replacement.clone();
        }
    });
}
