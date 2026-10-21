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
const NONE: Groups = &[&[SELECT], &[SKETCH]];
const FACE: Groups = &[&[SKETCH]];
const SKETCH_IDLE: Groups = &[&[SELECT], CREATE, DIMENSIONS];
const SKETCH_ENTITY: Groups = &[
    &[SELECT],
    CREATE,
    DIMENSIONS,
    &[
        t(ToolId::Construction, "construction", "Construction"),
        t(ToolId::Delete, "delete", "Delete"),
    ],
];
const REGION: Groups = &[&[EXTRUDE]];
const EDGE: Groups = &[];
const FEATURE: Groups = &[&[t(ToolId::Edit, "edit", "Edit")]];
const BODY: Groups = &[&[k(ToolId::MoveRotate, "move", "Move / Rotate", "M")]];

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
    let tool = app.interaction.tool.as_ref()?;
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
    let op = &app.interaction.tool.as_ref()?.record.op;
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
    if app.interaction.selection.is_some() {
        return SelectionKind::Region;
    }
    match app.topology.selected_kind() {
        Some((HighlightKind::Face, _)) => return SelectionKind::Face,
        Some((HighlightKind::Edge, _)) => return SelectionKind::Edge,
        None => {}
    }
    if app.selected_feature.is_some() {
        SelectionKind::Feature
    } else if app.selected_body.is_some() {
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
    fn normal_tools_have_existing_icons_and_no_extensions() {
        for kind in ALL {
            for def in pill_tools(kind).iter().flat_map(|group| group.iter()) {
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
