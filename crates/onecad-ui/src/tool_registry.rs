//! Data-driven ToolPill contents: which tools show for which selection kind.
//! Groups and order follow the final native design reviewed at ed9ed33
//! (docs/design/native-ui/final/README.md, retired from the tree in 79eb923).
use crate::app::App;
use crate::shortcut_prefs::{KeyContext, ShortcutOverride};
use onecad_core::document::record::{KnownOperation, Operation};

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
    /// Several kinds at once; specialized tools validate their operand mix.
    Mixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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
    PenStroke,
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
    MirrorBody,
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
    /// (extension, publisher) for tools an extension contributes.
    pub source: Option<(&'static str, &'static str)>,
}

/// Single-key tool shortcuts per mode; the one source for binding and tooltips.
const MODEL_KEYS: &[(egui::Key, ToolId)] = &[
    (egui::Key::S, ToolId::Sketch),
    (egui::Key::E, ToolId::Extrude),
    (egui::Key::R, ToolId::Revolve),
    (egui::Key::F, ToolId::Fillet),
    (egui::Key::C, ToolId::Chamfer),
    (egui::Key::H, ToolId::Hole),
    (egui::Key::M, ToolId::MoveRotate),
    (egui::Key::B, ToolId::Boolean),
    (egui::Key::P, ToolId::DatumPlane),
    (egui::Key::O, ToolId::OffsetFace),
];
const SKETCH_KEYS: &[(egui::Key, ToolId)] = &[
    (egui::Key::L, ToolId::Line),
    (egui::Key::R, ToolId::Rectangle),
    (egui::Key::C, ToolId::Circle),
    (egui::Key::A, ToolId::Arc),
    (egui::Key::D, ToolId::Dimension),
    (egui::Key::X, ToolId::Construction),
];

pub fn default_bindings(context: KeyContext) -> &'static [(egui::Key, ToolId)] {
    if context.sketch() {
        SKETCH_KEYS
    } else {
        MODEL_KEYS
    }
}

/// Letters and digits only: the keys a user may record for a tool.
pub fn recordable(key: egui::Key) -> bool {
    let name = key.name();
    name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric()
}

fn context(sketch: bool) -> KeyContext {
    if sketch {
        KeyContext::Sketch
    } else {
        KeyContext::Modeling
    }
}

fn override_for(
    overrides: &[ShortcutOverride],
    id: ToolId,
    context: KeyContext,
) -> Option<&ShortcutOverride> {
    overrides
        .iter()
        .find(|o| o.context == context && o.tool == id)
}

pub fn effective_key(
    overrides: &[ShortcutOverride],
    id: ToolId,
    context: KeyContext,
) -> Option<egui::Key> {
    match override_for(overrides, id, context) {
        Some(o) => o.key.as_deref().and_then(egui::Key::from_name),
        None => default_bindings(context)
            .iter()
            .find(|(_, t)| *t == id)
            .map(|(k, _)| *k),
    }
}

pub fn shortcut_tool(
    overrides: &[ShortcutOverride],
    key: egui::Key,
    sketch: bool,
) -> Option<ToolId> {
    let context = context(sketch);
    default_bindings(context)
        .iter()
        .map(|(_, id)| *id)
        .find(|id| effective_key(overrides, *id, context) == Some(key))
}

pub fn shortcut_label(
    overrides: &[ShortcutOverride],
    id: ToolId,
    sketch: bool,
) -> Option<&'static str> {
    effective_key(overrides, id, context(sketch)).map(egui::Key::name)
}

type Groups = &'static [&'static [ToolDef]];

const fn t(id: ToolId, icon: &'static str, label: &'static str) -> ToolDef {
    ToolDef {
        id,
        icon,
        label,
        source: None,
    }
}
const SELECT: ToolDef = t(ToolId::Select, "select", "Select");
const SKETCH: ToolDef = t(ToolId::Sketch, "sketch", "Sketch");
const EXTRUDE: ToolDef = t(ToolId::Extrude, "extrude", "Extrude");
const CREATE: &[ToolDef] = &[
    t(ToolId::Line, "line", "Line"),
    t(ToolId::PenStroke, "spline", "Pencil Stroke"),
    t(ToolId::Rectangle, "rect", "Rectangle"),
    t(ToolId::Circle, "circle", "Circle"),
    t(ToolId::Arc, "arc", "Arc"),
    t(ToolId::Slot, "slot", "Slot"),
    t(ToolId::Point, "point", "Point"),
];
const DIMENSIONS: &[ToolDef] = &[
    t(ToolId::Dimension, "dimension", "Dimension"),
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
const FILLET: ToolDef = t(ToolId::Fillet, "fillet", "Fillet");
const CHAMFER: ToolDef = t(ToolId::Chamfer, "chamfer", "Chamfer");
const OFFSET_FACE: ToolDef = t(ToolId::OffsetFace, "offsetFace", "Offset face");
const NONE: Groups = &[&[SELECT], &[SKETCH], UTILITIES];
const FACE: Groups = &[&[SKETCH, OFFSET_FACE], UTILITIES];
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
const EDGE: Groups = &[&[FILLET, CHAMFER], UTILITIES];
const FEATURE: Groups = &[&[t(ToolId::Edit, "edit", "Edit")], UTILITIES];
const BODY: Groups = &[&[t(ToolId::MoveRotate, "move", "Move / Rotate")], UTILITIES];

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
    FILLET,
    CHAMFER,
    t(ToolId::Shell, "shell", "Shell"),
    t(ToolId::Hole, "hole", "Hole"),
    OFFSET_FACE,
    t(ToolId::Boolean, "union", "Boolean"),
    t(ToolId::LinearPattern, "patternLinear", "Linear pattern"),
    t(
        ToolId::CircularPattern,
        "patternCircular",
        "Circular pattern",
    ),
    t(ToolId::FeaturePattern, "patternLinear", "Feature pattern"),
    t(ToolId::MirrorBody, "mirror", "Mirror body"),
    t(ToolId::Sweep, "sweep", "Sweep"),
    t(ToolId::Loft, "loft", "Loft"),
    t(ToolId::DatumPlane, "datum", "Datum plane"),
    t(ToolId::OffsetPlane, "offsetPlane", "Offset plane"),
    t(ToolId::Generators, "gear", "Gear generator"),
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

/// Icon and label of any registered tool, for lists that show tools outside a pill.
pub fn definition(id: ToolId) -> Option<ToolDef> {
    const GROUPS: [Groups; 8] = [
        NONE,
        FACE,
        EDGE,
        BODY,
        REGION,
        SKETCH_IDLE,
        SKETCH_ENTITY,
        FEATURE,
    ];
    GROUPS
        .iter()
        .flat_map(|groups| groups.iter())
        .flat_map(|group| group.iter())
        .chain(PLANNED_MODEL)
        .chain(PLANNED_SKETCH)
        .find(|def| def.id == id)
        .copied()
}

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
        ToolId::Sweep => "Kernel executor is not implemented (ONECAD-314)",
        ToolId::FeaturePattern => "Native feature-pattern expansion is planned (ONECAD-29)",
        ToolId::Isolate | ToolId::Hide => {
            "Native temporary/persistent body visibility is planned (ONECAD-320)"
        }
        ToolId::Measure => "Native measurement workflow is planned (ONECAD-245)",
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
        SelectionKind::Mixed => NONE,
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
        Operation::Known(KnownOperation::Boolean(_)) => Some(ToolId::Boolean),
        Operation::Known(KnownOperation::LinearPattern(_)) => Some(ToolId::LinearPattern),
        Operation::Known(KnownOperation::CircularPattern(_)) => Some(ToolId::CircularPattern),
        Operation::Known(KnownOperation::MirrorBody(_)) => Some(ToolId::MirrorBody),
        Operation::Known(KnownOperation::Gear(_)) => Some(ToolId::Generators),
        Operation::Known(
            KnownOperation::PlaceComponent(_) | KnownOperation::DetachComponent(_),
        ) => Some(ToolId::InsertComponent),
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
        return running_operand_kind(running);
    }
    if app.doc.interaction.selection.is_some() {
        return SelectionKind::Region;
    }
    crate::item_selection::model_kind(&app.doc.selection)
}

fn running_operand_kind(tool: ToolId) -> SelectionKind {
    match tool {
        ToolId::MoveRotate
        | ToolId::LinearPattern
        | ToolId::CircularPattern
        | ToolId::MirrorBody
        | ToolId::Boolean => SelectionKind::Body,
        ToolId::OffsetFace | ToolId::Shell | ToolId::Hole => SelectionKind::Face,
        ToolId::Fillet | ToolId::Chamfer => SelectionKind::Edge,
        ToolId::Extrude | ToolId::Revolve | ToolId::Loft => SelectionKind::Region,
        _ => SelectionKind::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymaps_have_unique_keys_and_tools() {
        for sketch in [false, true] {
            let map = default_bindings(context(sketch));
            for (i, (key, id)) in map.iter().enumerate() {
                assert!(map[i + 1..].iter().all(|(k, t)| k != key && t != id));
                assert_eq!(shortcut_tool(&[], *key, sketch), Some(*id));
                assert_eq!(shortcut_label(&[], *id, sketch), Some(key.name()));
            }
        }
    }

    #[test]
    fn keymap_tools_exist_in_registry_and_modes_do_not_leak() {
        let known: Vec<ToolId> = [
            NONE,
            FACE,
            SKETCH_IDLE,
            SKETCH_ENTITY,
            REGION,
            EDGE,
            FEATURE,
            BODY,
        ]
        .iter()
        .flat_map(|g| g.iter().flat_map(|g| g.iter().map(|d| d.id)))
        .chain(PLANNED_SKETCH.iter().chain(PLANNED_MODEL).map(|d| d.id))
        .collect();
        for (_, id) in MODEL_KEYS.iter().chain(SKETCH_KEYS) {
            assert!(known.contains(id), "{id:?} not in registry");
        }
        assert_eq!(shortcut_tool(&[], egui::Key::E, true), None);
        assert_eq!(shortcut_tool(&[], egui::Key::L, false), None);
        assert_eq!(shortcut_label(&[], ToolId::Line, false), None);
        assert_eq!(shortcut_label(&[], ToolId::Extrude, true), None);
    }

    #[test]
    fn running_operations_name_their_operand_kind() {
        for (tool, kind) in [
            (ToolId::LinearPattern, SelectionKind::Body),
            (ToolId::CircularPattern, SelectionKind::Body),
            (ToolId::Boolean, SelectionKind::Body),
            (ToolId::MirrorBody, SelectionKind::Body),
            (ToolId::OffsetFace, SelectionKind::Face),
            (ToolId::Shell, SelectionKind::Face),
            (ToolId::Hole, SelectionKind::Face),
            (ToolId::Fillet, SelectionKind::Edge),
            (ToolId::Chamfer, SelectionKind::Edge),
            (ToolId::Revolve, SelectionKind::Region),
            (ToolId::Generators, SelectionKind::None),
        ] {
            assert_eq!(running_operand_kind(tool), kind);
        }
    }

    const ALL: [SelectionKind; 9] = [
        SelectionKind::None,
        SelectionKind::Face,
        SelectionKind::Edge,
        SelectionKind::Body,
        SelectionKind::Region,
        SelectionKind::SketchIdle,
        SelectionKind::SketchEntity,
        SelectionKind::Feature,
        SelectionKind::Mixed,
    ];

    #[test]
    fn restored_catalog_is_discoverable_with_owned_implementation_gaps() {
        for tool in PLANNED_SKETCH.iter().chain(PLANNED_MODEL) {
            if !matches!(
                tool.id,
                ToolId::Extrude
                    | ToolId::MoveRotate
                    | ToolId::Fillet
                    | ToolId::Chamfer
                    | ToolId::OffsetFace
                    | ToolId::Revolve
                    | ToolId::Loft
                    | ToolId::DatumPlane
                    | ToolId::OffsetPlane
                    | ToolId::Shell
                    | ToolId::Hole
                    | ToolId::Boolean
                    | ToolId::LinearPattern
                    | ToolId::CircularPattern
                    | ToolId::MirrorBody
                    | ToolId::Generators
                    | ToolId::InsertComponent
            ) {
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
        let ids = |group: &[ToolDef]| group.iter().map(|tool| tool.id).collect::<Vec<_>>();
        assert_eq!(ids(EDGE[0]), [ToolId::Fillet, ToolId::Chamfer]);
        assert_eq!(ids(FACE[0]), [ToolId::Sketch, ToolId::OffsetFace]);
    }
}
