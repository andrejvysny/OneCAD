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
}

#[derive(Clone, Copy, Debug)]
pub struct ToolDef {
    pub id: ToolId,
    pub icon: &'static str,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
}

type Groups = &'static [&'static [ToolDef]];

const fn t(id: ToolId, icon: &'static str, label: &'static str) -> ToolDef {
    ToolDef {
        id,
        icon,
        label,
        shortcut: None,
    }
}
const fn k(id: ToolId, icon: &'static str, label: &'static str, key: &'static str) -> ToolDef {
    ToolDef {
        id,
        icon,
        label,
        shortcut: Some(key),
    }
}

const SELECT: ToolDef = t(ToolId::Select, "select", "Select");
const MEASURE: ToolDef = t(ToolId::Measure, "measure", "Measure");
const ISOLATE: ToolDef = t(ToolId::Isolate, "isolate", "Isolate");
const HIDE: ToolDef = t(ToolId::Hide, "eyeOff", "Hide");
const MORE: ToolDef = t(ToolId::More, "overflow", "More");
const SKETCH: ToolDef = t(ToolId::Sketch, "sketch", "Sketch (XY)");
const EXTRUDE: ToolDef = k(ToolId::Extrude, "extrude", "Extrude", "E");
const MATERIAL: ToolDef = t(ToolId::Material, "material", "Material");

const NONE: Groups = &[
    &[SELECT],
    &[
        SKETCH,
        t(ToolId::DatumPlane, "datum", "Datum plane"),
        t(ToolId::OffsetPlane, "offsetPlane", "Offset plane"),
    ],
    &[
        t(ToolId::Generators, "gear", "Generators"),
        t(ToolId::InsertComponent, "component", "Insert component"),
        t(ToolId::Import, "import", "Import"),
    ],
    &[MEASURE, t(ToolId::Section, "section", "Section")],
];

const FACE: Groups = &[
    &[
        SKETCH,
        EXTRUDE,
        t(ToolId::OffsetFace, "offsetFace", "Offset face"),
        t(ToolId::Hole, "hole", "Hole"),
        t(ToolId::Draft, "draft", "Draft"),
        t(ToolId::Shell, "shell", "Shell"),
    ],
    &[MATERIAL, MEASURE],
    &[ISOLATE, HIDE, MORE],
];

const SKETCH_IDLE: Groups = &[
    &[SELECT],
    &[
        k(ToolId::Line, "line", "Line", "L"),
        t(ToolId::Rectangle, "rect", "Rectangle"),
        t(ToolId::Circle, "circle", "Circle"),
        t(ToolId::Arc, "arc", "Arc"),
        t(ToolId::Slot, "slot", "Slot"),
        t(ToolId::Polygon, "polygon", "Polygon"),
        t(ToolId::Spline, "spline", "Spline"),
        t(ToolId::Point, "point", "Point"),
    ],
    &[
        k(ToolId::Dimension, "dimension", "Dimension", "D"),
        t(ToolId::Constraints, "coincident", "Constraints"),
    ],
    &[
        t(ToolId::Trim, "trim", "Trim"),
        t(ToolId::Extend, "extend", "Extend"),
        t(ToolId::SketchFillet, "sketchFillet", "Sketch fillet"),
        t(ToolId::SketchOffset, "sketchOffset", "Offset"),
        t(ToolId::Project, "project", "Project"),
    ],
];

const SKETCH_ENTITY: Groups = &[
    &[
        k(ToolId::Horizontal, "horizontal", "Horizontal", "H"),
        k(ToolId::Vertical, "vertical", "Vertical", "V"),
        k(ToolId::Parallel, "parallel", "Parallel", "⇧P"),
        k(
            ToolId::Perpendicular,
            "perpendicular",
            "Perpendicular",
            "⇧L",
        ),
        k(ToolId::Equal, "equal", "Equal", "="),
        k(ToolId::Midpoint, "midpoint", "Midpoint", "M"),
        k(ToolId::Fix, "fix", "Fix", "F"),
        k(ToolId::Distance, "distance", "Distance", "D"),
        t(ToolId::Constraints, "coincident", "Constraints"),
    ],
    &[
        t(ToolId::Construction, "construction", "Construction"),
        t(ToolId::Delete, "delete", "Delete"),
    ],
];

const REGION: Groups = &[
    &[
        EXTRUDE,
        t(ToolId::Revolve, "revolve", "Revolve"),
        t(ToolId::Sweep, "sweep", "Sweep"),
        t(ToolId::Loft, "loft", "Loft"),
    ],
    &[MEASURE, MORE],
];

const EDGE: Groups = &[
    &[
        t(ToolId::Fillet, "fillet", "Fillet"),
        t(ToolId::Chamfer, "chamfer", "Chamfer"),
    ],
    &[MEASURE, ISOLATE, MORE],
];

const FEATURE: Groups = &[
    &[
        t(ToolId::Edit, "edit", "Edit"),
        t(ToolId::RollBack, "rolledBack", "Roll back to here"),
        t(ToolId::Suppress, "suppressed", "Suppress"),
        t(ToolId::Duplicate, "duplicate", "Duplicate"),
    ],
    &[t(ToolId::Delete, "delete", "Delete"), MORE],
];

const BODY: Groups = &[
    &[
        k(ToolId::MoveRotate, "move", "Move / Rotate", "M"),
        MATERIAL,
        MEASURE,
    ],
    &[ISOLATE, HIDE, MORE],
];

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

/// The running model tool: Move / Rotate or (for every other known operation) Extrude.
pub fn running_tool(app: &App) -> Option<ToolId> {
    let tool = app.interaction.tool.as_ref()?;
    Some(
        if matches!(
            tool.record.op,
            Operation::Known(KnownOperation::TransformBody(_))
        ) {
            ToolId::MoveRotate
        } else {
            ToolId::Extrude
        },
    )
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

    fn labels(kind: SelectionKind) -> Vec<Vec<&'static str>> {
        pill_tools(kind)
            .iter()
            .map(|g| g.iter().map(|d| d.label).collect())
            .collect()
    }

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
    fn every_icon_exists() {
        for kind in ALL {
            for def in pill_tools(kind).iter().flat_map(|g| g.iter()) {
                assert!(onecad_widgets::icon::exists(def.icon), "{}", def.icon);
            }
        }
    }

    #[test]
    fn idle_and_face_lists_match_readme() {
        assert_eq!(
            labels(SelectionKind::None),
            vec![
                vec!["Select"],
                vec!["Sketch (XY)", "Datum plane", "Offset plane"],
                vec!["Generators", "Insert component", "Import"],
                vec!["Measure", "Section"],
            ]
        );
        assert_eq!(
            labels(SelectionKind::Face),
            vec![
                vec![
                    "Sketch (XY)",
                    "Extrude",
                    "Offset face",
                    "Hole",
                    "Draft",
                    "Shell"
                ],
                vec!["Material", "Measure"],
                vec!["Isolate", "Hide", "More"],
            ]
        );
    }

    #[test]
    fn sketch_lists_match_readme() {
        assert_eq!(
            labels(SelectionKind::SketchIdle),
            vec![
                vec!["Select"],
                vec![
                    "Line",
                    "Rectangle",
                    "Circle",
                    "Arc",
                    "Slot",
                    "Polygon",
                    "Spline",
                    "Point"
                ],
                vec!["Dimension", "Constraints"],
                vec!["Trim", "Extend", "Sketch fillet", "Offset", "Project"],
            ]
        );
        assert_eq!(
            labels(SelectionKind::SketchEntity),
            vec![
                vec![
                    "Horizontal",
                    "Vertical",
                    "Parallel",
                    "Perpendicular",
                    "Equal",
                    "Midpoint",
                    "Fix",
                    "Distance",
                    "Constraints"
                ],
                vec!["Construction", "Delete"],
            ]
        );
    }

    #[test]
    fn region_edge_feature_body_lists_match_readme() {
        assert_eq!(
            labels(SelectionKind::Region),
            vec![
                vec!["Extrude", "Revolve", "Sweep", "Loft"],
                vec!["Measure", "More"]
            ]
        );
        assert_eq!(
            labels(SelectionKind::Edge),
            vec![
                vec!["Fillet", "Chamfer"],
                vec!["Measure", "Isolate", "More"]
            ]
        );
        assert_eq!(
            labels(SelectionKind::Feature),
            vec![
                vec!["Edit", "Roll back to here", "Suppress", "Duplicate"],
                vec!["Delete", "More"]
            ]
        );
        assert_eq!(
            labels(SelectionKind::Body),
            vec![
                vec!["Move / Rotate", "Material", "Measure"],
                vec!["Isolate", "Hide", "More"]
            ]
        );
    }
}
