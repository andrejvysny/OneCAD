//! Data-driven ToolPill contents: which tools show for which selection kind.
//! Groups and order follow docs/design/native-ui/final/README.md.
use crate::app::App;
use onecad_core::document::record::{KnownOperation, Operation};
use onecad_render::HighlightKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    None,
    Face,
    Edge,
    Body,
    Region,
    SketchIdle,
    SketchEntity,
    Feature,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "Legacy gallery identities are not native creation commands"
)]
pub enum ToolId {
    Select,
    Sketch,
    DatumPlane,
    OffsetPlane,
    Generators,
    InsertComponent,
    Import,
    Measure,
    Section,
    Extrude,
    OffsetFace,
    Hole,
    Draft,
    Shell,
    Material,
    Isolate,
    Hide,
    More,
    Line,
    Rectangle,
    Circle,
    Arc,
    Slot,
    Polygon,
    Ellipse,
    SketchMirror,
    Boolean,
    LinearPattern,
    CircularPattern,
    FeaturePattern,
    Spline,
    Point,
    Dimension,
    Constraints,
    Trim,
    Extend,
    SketchFillet,
    SketchOffset,
    Project,
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Equal,
    Midpoint,
    Fix,
    Distance,
    Diameter,
    Construction,
    Delete,
    Revolve,
    Sweep,
    Loft,
    Fillet,
    Chamfer,
    Edit,
    RollBack,
    Suppress,
    Duplicate,
    MoveRotate,
    MakeComponent,
    Flange,
    Dogbone,
    CheckGeometry,
}

#[derive(Clone, Copy, Debug)]
pub struct ToolDef {
    pub id: ToolId,
    pub icon: &'static str,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    /// (extension, publisher) for tools an extension contributes.
    pub source: Option<(&'static str, &'static str)>,
}

type Groups = &'static [&'static [ToolDef]];

const fn t(id: ToolId, icon: &'static str, label: &'static str) -> ToolDef {
    ToolDef {
        id,
        icon,
        label,
        shortcut: None,
        source: None,
    }
}
const fn k(id: ToolId, icon: &'static str, label: &'static str, key: &'static str) -> ToolDef {
    ToolDef {
        id,
        icon,
        label,
        shortcut: Some(key),
        source: None,
    }
}
const SELECT: ToolDef = t(ToolId::Select, "select", "Select");
const SKETCH: ToolDef = t(ToolId::Sketch, "sketch", "Sketch");
const EXTRUDE: ToolDef = k(ToolId::Extrude, "extrude", "Extrude", "E");
const CREATE: &[ToolDef] = &[
    k(ToolId::Line, "line", "Line", "L"),
    t(ToolId::Rectangle, "rect", "Rectangle"),
    t(ToolId::Circle, "circle", "Circle"),
    t(ToolId::Arc, "arc", "Arc"),
    t(ToolId::Slot, "slot", "Slot"),
    t(ToolId::Point, "point", "Point"),
];
const DIMENSIONS: &[ToolDef] = &[
    k(ToolId::Dimension, "dimension", "Dimension", "D"),
    t(ToolId::Constraints, "coincident", "Constraints"),
];
const UTILITIES: &[ToolDef] = &[
    t(ToolId::Import, "import", "Import…"),
    t(ToolId::More, "overflow", "More tools"),
];
const MORE: &[ToolDef] = &[t(ToolId::More, "overflow", "More tools")];
const DIRECT_CONSTRAINTS: &[ToolDef] = &[
    t(ToolId::Horizontal, "horizontal", "Horizontal"),
    t(ToolId::Vertical, "vertical", "Vertical"),
    t(ToolId::Fix, "fix", "Fix"),
];
const NONE: Groups = &[&[SELECT], &[SKETCH], UTILITIES];
const FACE: Groups = &[&[SKETCH], UTILITIES];
const SKETCH_IDLE: Groups = &[&[SELECT], CREATE, DIMENSIONS, DIRECT_CONSTRAINTS, MORE];
const SKETCH_ENTITY: Groups = &[
    &[SELECT],
    CREATE,
    DIMENSIONS,
    DIRECT_CONSTRAINTS,
    MORE,
    &[
        t(ToolId::Construction, "construction", "Construction"),
        t(ToolId::Delete, "delete", "Delete"),
    ],
];
const REGION: Groups = &[&[EXTRUDE], UTILITIES];
const EDGE: Groups = &[UTILITIES];
const FEATURE: Groups = &[&[t(ToolId::Edit, "edit", "Edit")], UTILITIES];
const BODY: Groups = &[
    &[k(ToolId::MoveRotate, "move", "Move / Rotate", "M")],
    UTILITIES,
];

/// Discoverability uses the same availability and dispatch as the primary pill.
pub const PLANNED_SKETCH: &[ToolDef] = &[
    t(ToolId::Polygon, "polygon", "Polygon"),
    t(ToolId::Ellipse, "ellipse", "Ellipse"),
    t(ToolId::Trim, "trim", "Trim"),
    t(ToolId::Extend, "extend", "Extend"),
    t(ToolId::SketchFillet, "sketchFillet", "Sketch fillet"),
    t(ToolId::SketchOffset, "sketchOffset", "Sketch offset"),
    t(ToolId::SketchMirror, "sketchMirror", "Sketch mirror"),
    t(ToolId::Project, "project", "Project"),
    t(ToolId::Spline, "spline", "Spline"),
];
pub const PLANNED_MODEL: &[ToolDef] = &[
    t(ToolId::Extrude, "extrude", "Extrude"),
    t(ToolId::MoveRotate, "move", "Move / Rotate"),
    t(ToolId::Revolve, "revolve", "Revolve"),
    t(ToolId::Fillet, "fillet", "Fillet"),
    t(ToolId::Chamfer, "chamfer", "Chamfer"),
    t(ToolId::Shell, "shell", "Shell"),
    t(ToolId::Hole, "hole", "Hole"),
    t(ToolId::OffsetFace, "offsetFace", "Offset face"),
    t(ToolId::Boolean, "union", "Boolean"),
    t(ToolId::LinearPattern, "patternLinear", "Linear pattern"),
    t(
        ToolId::CircularPattern,
        "patternCircular",
        "Circular pattern",
    ),
    t(ToolId::FeaturePattern, "patternLinear", "Feature pattern"),
    t(ToolId::Sweep, "sweep", "Sweep"),
    t(ToolId::Loft, "loft", "Loft"),
    t(ToolId::DatumPlane, "datum", "Datum plane"),
    t(ToolId::OffsetPlane, "offsetPlane", "Offset plane"),
    t(ToolId::Generators, "gear", "Generators"),
    t(ToolId::Measure, "measure", "Measure"),
    t(ToolId::Isolate, "isolate", "Isolate"),
    t(ToolId::Hide, "eyeOff", "Hide body"),
    t(ToolId::InsertComponent, "component", "Insert component"),
    t(ToolId::MakeComponent, "component", "Make component"),
    t(ToolId::Draft, "draft", "Draft"),
    t(ToolId::Material, "material", "Material"),
    t(ToolId::RollBack, "rolledBack", "Roll back history"),
    t(ToolId::Suppress, "suppressed", "Suppress feature"),
    t(ToolId::Duplicate, "duplicate", "Duplicate feature"),
    t(ToolId::Flange, "extension", "Flange"),
    t(ToolId::Dogbone, "extension", "Dogbone"),
    t(ToolId::CheckGeometry, "check", "Check geometry"),
];

/// Implementation gaps are distinct from temporary selection/worker applicability.
pub fn unavailable_reason(id: ToolId) -> &'static str {
    match id {
        ToolId::Polygon | ToolId::Ellipse => {
            "Native polygon/ellipse creation is planned (ONECAD-292)"
        }
        ToolId::Trim | ToolId::Extend => "Native trim/extend workflow is planned (ONECAD-293)",
        ToolId::SketchFillet | ToolId::SketchOffset | ToolId::SketchMirror => {
            "Native sketch modification is planned (ONECAD-294)"
        }
        ToolId::Project => "Native projection workflow is planned (ONECAD-296)",
        ToolId::Spline => "Native spline creation is planned (ONECAD-298)",
        ToolId::Revolve => "Native revolve operand and preview flow is planned (ONECAD-315)",
        ToolId::Fillet => {
            "Native fillet creation flow is incomplete (ONECAD-220); saved features remain editable"
        }
        ToolId::Chamfer => {
            "Native chamfer creation flow is planned (ONECAD-316); saved features remain editable"
        }
        ToolId::Shell => "Native shell creation flow is planned (ONECAD-317)",
        ToolId::Hole => "Native hole creation flow is planned (ONECAD-318)",
        ToolId::OffsetFace => "Native offset-face creation flow is planned (ONECAD-319)",
        ToolId::Boolean => "Native target/tool selection and preview are planned (ONECAD-253)",
        ToolId::LinearPattern | ToolId::CircularPattern => {
            "Native body-pattern creation/preview guard is incomplete (ONECAD-221)"
        }
        ToolId::FeaturePattern => {
            "Native feature-pattern expansion is planned (ONECAD-222 / ONECAD-29)"
        }
        ToolId::Isolate | ToolId::Hide => {
            "Native temporary/persistent body visibility is planned (ONECAD-320)"
        }
        ToolId::Measure => "Native measurement workflow is planned (ONECAD-245)",
        ToolId::InsertComponent | ToolId::MakeComponent => {
            "Native library placement is unavailable (ONECAD-300–303)"
        }
        ToolId::Flange | ToolId::Dogbone | ToolId::CheckGeometry => {
            "Native extension execution is unavailable (ONECAD-92)"
        }
        ToolId::RollBack | ToolId::Suppress | ToolId::Duplicate => {
            "Native history action is planned (ONECAD-321)"
        }
        _ => "Native creation workflow is not connected (ONECAD-314)",
    }
}

/// Tool groups for a selection kind, in README order.
pub fn pill_tools(kind: SelectionKind) -> &'static [&'static [ToolDef]] {
    match kind {
        SelectionKind::None => NONE,
        SelectionKind::Face => FACE,
        SelectionKind::Edge => EDGE,
        SelectionKind::Body => BODY,
        SelectionKind::Region => REGION,
        SelectionKind::SketchIdle => SKETCH_IDLE,
        SelectionKind::SketchEntity => SKETCH_ENTITY,
        SelectionKind::Feature => FEATURE,
    }
}

/// The running model tool, including saved-feature parameter edits.
pub fn running_tool(app: &App) -> Option<ToolId> {
    let tool = app.doc.interaction.tool.as_ref()?;
    match tool.record.op {
        Operation::Known(KnownOperation::Extrude(_)) => Some(ToolId::Extrude),
        Operation::Known(KnownOperation::TransformBody(_)) => Some(ToolId::MoveRotate),
        Operation::Known(KnownOperation::Revolve(_)) => Some(ToolId::Revolve),
        Operation::Known(KnownOperation::Fillet(_)) => Some(ToolId::Fillet),
        Operation::Known(KnownOperation::Chamfer(_)) => Some(ToolId::Chamfer),
        Operation::Known(KnownOperation::Shell(_)) => Some(ToolId::Shell),
        Operation::Known(KnownOperation::Loft(_)) => Some(ToolId::Loft),
        Operation::Known(KnownOperation::Sweep(_)) => Some(ToolId::Sweep),
        Operation::Known(KnownOperation::Hole(_)) => Some(ToolId::Hole),
        Operation::Known(KnownOperation::OffsetFace(_)) => Some(ToolId::OffsetFace),
        _ => Some(ToolId::Edit),
    }
}

pub fn operation_icon(op_type: &str) -> &'static str {
    match op_type {
        "Sketch" => "sketch",
        "Extrude" => "extrude",
        "Revolve" => "revolve",
        "Fillet" => "fillet",
        "Chamfer" => "chamfer",
        "Shell" => "shell",
        "LinearPattern" | "FeaturePattern" => "patternLinear",
        "CircularPattern" => "patternCircular",
        "Loft" => "loft",
        "Sweep" => "sweep",
        "MirrorBody" => "mirror",
        "ImportStep" => "import",
        "TransformBody" => "move",
        "Hole" => "hole",
        "OffsetFace" => "offsetFace",
        "Gear" => "gear",
        "PlaceComponent" | "DetachComponent" => "component",
        _ => "body",
    }
}

pub fn running_definition(app: &App) -> Option<ToolDef> {
    let op = &app.doc.interaction.tool.as_ref()?.record.op;
    let label = match op.op_type() {
        "Sketch" => "Sketch",
        "Extrude" => "Extrude",
        "Revolve" => "Revolve",
        "Fillet" => "Fillet",
        "Chamfer" => "Chamfer",
        "Shell" => "Shell",
        "Boolean" => "Boolean",
        "LinearPattern" => "Linear pattern",
        "CircularPattern" => "Circular pattern",
        "FeaturePattern" => "Feature pattern",
        "Loft" => "Loft",
        "Sweep" => "Sweep",
        "MirrorBody" => "Mirror body",
        "ImportStep" => "Import STEP",
        "TransformBody" => "Move / Rotate",
        "Hole" => "Hole",
        "OffsetFace" => "Offset face",
        "Gear" => "Gear",
        "PlaceComponent" => "Place component",
        "DetachComponent" => "Detach component",
        _ => "Unknown feature",
    };
    Some(t(running_tool(app)?, operation_icon(op.op_type()), label))
}

pub fn selection_kind(app: &App) -> SelectionKind {
    if app.sketch_active() {
        let entity = app
            .doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| !session.selection.is_empty());
        return if entity {
            SelectionKind::SketchEntity
        } else {
            SelectionKind::SketchIdle
        };
    }
    if let Some(running) = running_tool(app) {
        return if running == ToolId::MoveRotate {
            SelectionKind::Body
        } else {
            SelectionKind::Region
        };
    }
    if app.doc.interaction.selection.is_some() {
        return SelectionKind::Region;
    }
    match app.doc.topology.selected_kind() {
        Some((HighlightKind::Face, _)) => return SelectionKind::Face,
        Some((HighlightKind::Edge, _)) => return SelectionKind::Edge,
        None => {}
    }
    if app.doc.selected_feature.is_some() {
        SelectionKind::Feature
    } else if app.doc.selected_body.is_some() {
        SelectionKind::Body
    } else {
        SelectionKind::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [SelectionKind; 8] = [
        SelectionKind::None,
        SelectionKind::Face,
        SelectionKind::Edge,
        SelectionKind::Body,
        SelectionKind::Region,
        SelectionKind::SketchIdle,
        SelectionKind::SketchEntity,
        SelectionKind::Feature,
    ];

    #[test]
    fn restored_catalog_is_discoverable_with_owned_implementation_gaps() {
        for tool in PLANNED_SKETCH.iter().chain(PLANNED_MODEL) {
            if !matches!(tool.id, ToolId::Extrude | ToolId::MoveRotate) {
                assert!(
                    unavailable_reason(tool.id).contains("ONECAD-"),
                    "{:?}",
                    tool.id
                );
            }
        }
        for kind in ALL {
            assert!(
                pill_tools(kind)
                    .iter()
                    .flat_map(|group| group.iter())
                    .any(|tool| tool.id == ToolId::More)
            );
        }
        const { assert!(!crate::editor_facade::PREVIEW_FACADES) };
    }

    #[test]
    fn normal_tools_have_existing_icons_and_no_extensions() {
        for kind in ALL {
            for def in pill_tools(kind)
                .iter()
                .flat_map(|group| group.iter())
                .chain(PLANNED_SKETCH)
                .chain(PLANNED_MODEL)
            {
                assert!(onecad_widgets::icon::exists(def.icon), "{}", def.icon);
                assert!(def.source.is_none());
            }
        }
    }

    #[test]
    fn running_operation_icons_exist_and_match_operation_families() {
        for operation in [
            "Sketch",
            "Extrude",
            "Revolve",
            "Fillet",
            "Chamfer",
            "Shell",
            "Boolean",
            "LinearPattern",
            "CircularPattern",
            "FeaturePattern",
            "Loft",
            "Sweep",
            "MirrorBody",
            "ImportStep",
            "TransformBody",
            "Hole",
            "OffsetFace",
            "Gear",
            "PlaceComponent",
            "DetachComponent",
        ] {
            assert!(
                onecad_widgets::icon::exists(operation_icon(operation)),
                "{operation}"
            );
        }
        assert_eq!(operation_icon("TransformBody"), "move");
        assert_eq!(operation_icon("OffsetFace"), "offsetFace");
        assert_ne!(operation_icon("Revolve"), operation_icon("Extrude"));
    }

    #[test]
    fn sketch_selection_preserves_creation_tools() {
        assert_eq!(
            SKETCH_IDLE[1]
                .iter()
                .map(|tool| tool.id)
                .collect::<Vec<_>>(),
            SKETCH_ENTITY[1]
                .iter()
                .map(|tool| tool.id)
                .collect::<Vec<_>>()
        );
        assert!(
            SKETCH_ENTITY
                .iter()
                .flat_map(|group| group.iter())
                .any(|tool| tool.id == ToolId::Construction)
        );
    }

    #[test]
    fn normal_model_surfaces_offer_implemented_creation_and_editing() {
        assert_eq!(REGION[0].len(), 1);
        assert_eq!(REGION[0][0].id, ToolId::Extrude);
        assert_eq!(BODY[0][0].id, ToolId::MoveRotate);
        assert_eq!(FEATURE[0][0].id, ToolId::Edit);
    }
}
