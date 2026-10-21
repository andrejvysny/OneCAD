//! Pill selection label: kind plus count of the typed selection (ONECAD-315).
use crate::{app::App, tool_registry::SelectionKind};
use onecad_interaction::selection_set::SelectionItem;
use onecad_widgets::components::actions::SelectionVariant;

pub(crate) fn body_name(app: &App, id: onecad_core::ids::BodyId) -> String {
    app.doc
        .display
        .as_ref()
        .and_then(|d| {
            d.projection
                .bodies
                .values()
                .find(|b| b.id.parse().ok() == Some(id))
                .map(|b| b.name.clone())
        })
        .unwrap_or_default()
}

pub(crate) fn selection_label(
    app: &App,
    kind: SelectionKind,
) -> Option<(String, String, SelectionVariant)> {
    let model = SelectionVariant::Model;
    if let Some(tool) = &app.doc.interaction.tool
        && tool.editing_existing
    {
        return Some((tool.record.name.clone(), "feature".into(), model));
    }
    let count = app.doc.selection.len();
    let context = || match app.doc.selected_body() {
        Some(body) => body_name(app, body),
        None => {
            let items = app.doc.selection.items().iter();
            let bodies: std::collections::HashSet<_> =
                items.filter_map(SelectionItem::body).collect();
            format!("{} bodies", bodies.len())
        }
    };
    match kind {
        SelectionKind::Face => Some((counted(count, "Face", "Faces"), context(), model)),
        SelectionKind::Edge => Some((counted(count, "Edge", "Edges"), context(), model)),
        SelectionKind::Body if count == 1 => Some(("Body".into(), context(), model)),
        SelectionKind::Body => Some((counted(count, "Body", "Bodies"), "bodies".into(), model)),
        SelectionKind::Region => Some((
            counted(count.max(1), "Region", "Regions"),
            "profile".into(),
            model,
        )),
        SelectionKind::Mixed => Some((format!("{count} items"), "mixed".into(), model)),
        SelectionKind::Feature if count > 1 => Some((
            counted(count, "Feature", "Features"),
            "feature".into(),
            model,
        )),
        SelectionKind::Feature => {
            let id = app.doc.selected_feature()?.to_string();
            let label = app
                .doc
                .display
                .as_ref()?
                .projection
                .features
                .iter()
                .find(|f| f.id == id)
                .map_or_else(|| "Feature".into(), |f| f.label.clone());
            Some((label, "feature".into(), model))
        }
        SelectionKind::SketchEntity => {
            let session = app.doc.sketch.session.as_ref()?;
            let points = session
                .selection
                .iter()
                .filter(|target| {
                    matches!(
                        target,
                        onecad_interaction::sketch::SketchTarget::Point { .. }
                    )
                })
                .count();
            let entities = session.selection.len() - points;
            let label = if let [target] = session.selection.as_slice() {
                crate::sketch_view::target_name(*target, &session.authoritative)
            } else {
                match (points, entities) {
                    (_, 0) => "Points",
                    (0, _) => "Entities",
                    _ => "Sketch targets",
                }
                .into()
            };
            Some((
                label,
                format!("{} selected", session.selection.len()),
                SelectionVariant::Sketch,
            ))
        }
        SelectionKind::None | SelectionKind::SketchIdle => None,
    }
}

/// "Face" for one item, "3 Faces" for several: the existing label, counted.
fn counted(count: usize, one: &str, many: &str) -> String {
    if count > 1 {
        format!("{count} {many}")
    } else {
        one.into()
    }
}
