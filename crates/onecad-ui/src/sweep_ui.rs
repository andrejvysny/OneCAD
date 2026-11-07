//! Explicit profile and path selection for the bounded circular-section Sweep.
use crate::app::App;
use onecad_app::dto::{FeatureStatus, SketchStatus};
use onecad_core::{
    document::{
        record::{KnownOperation, Operation, OperationRecord, SketchOpParams, SweepParams},
        refs::SketchRegionRef,
    },
    ids::{RecordId, SketchId},
};
use onecad_interaction::{ModelSession, ModelingOptions, modeling_records, operand_binding};
use onecad_widgets::components::overlays::FeatureCardRows;

fn one_curve(entities: &[serde_json::Value], ellipse_allowed: bool) -> bool {
    let mut count = 0;
    for entity in entities {
        let Some(kind) = entity.get("type").and_then(|value| value.as_str()) else {
            return false;
        };
        if entity
            .get("construction")
            .is_some_and(|value| !value.is_boolean())
        {
            return false;
        }
        if kind == "Point" {
            continue;
        }
        if entity
            .get("construction")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            || !(kind == "Circle" || ellipse_allowed && kind == "Ellipse")
        {
            return false;
        }
        count += 1;
    }
    count == 1
}

fn sketch(
    records: &[OperationRecord],
    id: SketchId,
) -> Option<(&OperationRecord, &SketchOpParams)> {
    let mut matches = records.iter().filter_map(|record| match &record.op {
        Operation::Known(KnownOperation::Sketch(params)) if params.sketch == id => {
            Some((record, params))
        }
        _ => None,
    });
    let (record, params) = matches.next()?;
    if record.suppressed || matches.next().is_some() {
        return None;
    }
    Some((record, params))
}

fn current_curve(
    record: &OperationRecord,
    row: &onecad_app::dto::SketchDto,
    status: Option<FeatureStatus>,
    has_profile: bool,
) -> bool {
    !record.suppressed
        && current_evidence(
            status,
            has_profile,
            row.status,
            row.solve_geometry_token
                .as_ref()
                .is_none_or(|token| token == &row.geometry_token),
        )
}

fn current_evidence(
    status: Option<FeatureStatus>,
    has_profile: bool,
    solve: Option<SketchStatus>,
    token_matches: bool,
) -> bool {
    status == Some(FeatureStatus::Ok)
        && has_profile
        && token_matches
        && !matches!(solve, Some(SketchStatus::Over | SketchStatus::Error))
}

fn same_region(captured: &SketchRegionRef, selected: &SketchRegionRef) -> bool {
    matches!(selected.region_identity_version, Some(2 | 3))
        && captured.sketch == selected.sketch
        && captured.region == selected.region
        && captured.region_identity_version == selected.region_identity_version
}

fn selected(app: &App) -> Option<SketchRegionRef> {
    if app.sketch_active() {
        return None;
    }
    let selected = app.doc.interaction.selection.as_ref()?;
    let context = app.doc.interaction.context.as_ref()?;
    if &selected.context != context || selected.capture.scene != context.scene {
        return None;
    }
    let display = app.doc.display.as_ref()?;
    let records = &display.records[..display.records.len().min(display.projection.applied_ops)];
    let profile = &selected.profile;
    let (record, params) = sketch(records, profile.sketch)?;
    let row = display
        .projection
        .sketches
        .get(&profile.sketch.to_string())?;
    let status = display
        .projection
        .features
        .iter()
        .find(|feature| feature.id == record.record_id.to_string())
        .map(|feature| feature.status);
    let has_profile = display
        .profiles
        .iter()
        .any(|p| same_region(&p.reference, profile));
    if !one_curve(&params.entities, false) || !current_curve(record, row, status, has_profile) {
        return None;
    }
    Some(profile.clone())
}

fn paths(app: &App, profile: SketchId) -> Vec<(SketchId, String)> {
    let Some(display) = app.doc.display.as_ref() else {
        return Vec::new();
    };
    display
        .records
        .iter()
        .take(display.projection.applied_ops)
        .filter_map(|record| {
            let Operation::Known(KnownOperation::Sketch(params)) = &record.op else {
                return None;
            };
            let records =
                &display.records[..display.records.len().min(display.projection.applied_ops)];
            if params.sketch == profile
                || sketch(records, params.sketch).is_none()
                || !one_curve(&params.entities, true)
            {
                return None;
            }
            let row = display
                .projection
                .sketches
                .get(&params.sketch.to_string())?;
            let status = display
                .projection
                .features
                .iter()
                .find(|feature| feature.id == record.record_id.to_string())
                .map(|feature| feature.status);
            let has_profile = display.profiles.iter().any(|p| {
                p.reference.sketch == params.sketch
                    && matches!(p.reference.region_identity_version, Some(2 | 3))
            });
            if !current_curve(record, row, status, has_profile) {
                return None;
            }
            Some((params.sketch, row.name.clone()))
        })
        .collect()
}

pub(crate) fn ready(app: &App) -> bool {
    selected(app).is_some_and(|profile| !paths(app, profile.sketch).is_empty())
}

pub(crate) fn begin(app: &mut App) {
    let Some(context) = app.doc.interaction.context.clone() else {
        return;
    };
    let Some(profile) = selected(app) else { return };
    let step = app
        .doc
        .display
        .as_ref()
        .map_or(0, |d| d.projection.applied_ops as u32);
    let record = modeling_records::sweep(profile, RecordId::new(), step);
    app.doc.gesture += 1;
    match operand_binding::begin_bound(record, app.doc.gesture, context) {
        Ok(action) => app.dispatch(action),
        Err(error) => app.doc.status = error,
    }
    app.draw_dirty = true;
}

pub(crate) fn rows(
    app: &App,
    rows: &mut FeatureCardRows,
    _tool: &ModelSession,
    p: &SweepParams,
) -> Option<ModelingOptions> {
    let candidates = p
        .profile
        .as_ref()
        .map_or_else(Vec::new, |profile| paths(app, profile.sketch));
    let mut path = p
        .path_sketch
        .filter(|id| candidates.iter().any(|candidate| candidate.0 == *id));
    rows.row("Path sketch", |ui| {
        let label = path
            .and_then(|id| candidates.iter().find(|candidate| candidate.0 == id))
            .map_or("Choose path…", |candidate| candidate.1.as_str());
        egui::ComboBox::from_id_salt("sweep-path-sketch")
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (id, name) in &candidates {
                    let response = ui.selectable_value(&mut path, Some(*id), name);
                    crate::automation::name(
                        ui.ctx(),
                        response.id,
                        &format!("onecad.option.sweepPath.{id}"),
                    );
                }
            });
    });
    if let Some(range) = p.path_range {
        rows.row("Path interval", |ui| {
            ui.label(format!(
                "{:.6}° to {:.6}°",
                range.start_radians.to_degrees(),
                range.end_radians.to_degrees()
            ))
            .on_hover_text("Zero follows the circle’s sketch X axis or the ellipse’s major axis; angles increase in the path sketch direction.");
        });
    }
    rows.row("", |ui| {
        ui.add(egui::Label::new(egui::RichText::new(
            "Circular section; one closed circle or ellipse path. Placement and clearance are verified by preview.",
        ).small()).wrap());
    });
    (path != p.path_sketch).then_some(ModelingOptions::SweepPath(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn refuses_failed_missing_or_stale_path_evidence() {
        assert!(current_evidence(
            Some(FeatureStatus::Ok),
            true,
            Some(SketchStatus::Under),
            true
        ));
        assert!(current_evidence(Some(FeatureStatus::Ok), true, None, true));
        assert!(!current_evidence(
            Some(FeatureStatus::Error),
            true,
            Some(SketchStatus::Ok),
            true
        ));
        assert!(!current_evidence(
            Some(FeatureStatus::Ok),
            false,
            Some(SketchStatus::Ok),
            true
        ));
        assert!(!current_evidence(
            Some(FeatureStatus::Ok),
            true,
            Some(SketchStatus::Error),
            true
        ));
        assert!(!current_evidence(
            Some(FeatureStatus::Ok),
            true,
            Some(SketchStatus::Ok),
            false
        ));
    }

    #[test]
    fn profile_requires_matching_captured_identity_version() {
        let selected = SketchRegionRef {
            sketch: SketchId::new(),
            region: onecad_core::ids::RegionId::new("r"),
            region_identity_version: Some(3),
            region_anchor: None,
            extra: Default::default(),
        };
        let mut captured = selected.clone();
        assert!(same_region(&captured, &selected));
        captured.region_identity_version = Some(2);
        assert!(!same_region(&captured, &selected));
        assert!(!current_evidence(
            Some(FeatureStatus::Error),
            true,
            None,
            true
        ));
        assert!(!current_evidence(
            Some(FeatureStatus::Ok),
            true,
            Some(SketchStatus::Ok),
            false
        ));
    }

    #[test]
    fn admits_only_one_explicit_supported_curve() {
        let circle = json!({"id":"c", "type":"Circle"});
        let ellipse = json!({"id":"e", "type":"Ellipse"});
        let point = json!({"id":"p", "type":"Point"});
        assert!(one_curve(&[point.clone(), circle.clone()], false));
        assert!(!one_curve(std::slice::from_ref(&ellipse), false));
        assert!(one_curve(&[point, ellipse.clone()], true));
        assert!(!one_curve(&[circle.clone(), ellipse], true));
        assert!(!one_curve(
            &[json!({"type":"Circle", "construction":true})],
            true
        ));
        assert!(!one_curve(
            &[json!({"type":"Circle", "construction":"false"})],
            true
        ));
        assert!(!one_curve(&[json!({"type":"Line"})], true));
        assert!(!one_curve(&[json!({"kind":"circle"})], true));
    }
}
