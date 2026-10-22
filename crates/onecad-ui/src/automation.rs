//! Stable AccessKit author ids (`onecad.*`) that the iPadOS bridge exports as
//! automation identifiers (docs/ios/TESTING.md). They are never localized and
//! never follow a label, so UI tests survive copy changes.
use crate::tool_registry::ToolId;
use onecad_core::sketch::WorldPlane;
use onecad_interaction::{NumericParameter, ParameterAxis, sketch::CreationField};

/// The primary value entry, whatever field currently has focus: the running
/// operation's first declared field (fillet radius, offset distance,
/// translation X…), a sketch creation chip's first field (line length,
/// rectangle width, circle Ø…) or the sketch dimension editor's value.
pub(crate) const DIMENSION_INPUT: &str = "onecad.dimension.input";
pub(crate) const OP_CONFIRM: &str = "onecad.op.confirm";
pub(crate) const OP_CANCEL: &str = "onecad.op.cancel";
pub(crate) const ADDITIVE_SELECT: &str = "onecad.select.additive";
pub(crate) const VIEWPORT: &str = "onecad.viewport";
pub(crate) const TAB_NEW: &str = "onecad.tab.new";
pub(crate) const FINISH_SKETCH: &str = "onecad.tool.finishSketch";
pub(crate) const DISCARD_SKETCH: &str = "onecad.tool.discardSketch";

/// Names the widget `id`; a no-op while AccessKit is off.
pub(crate) fn name(ctx: &egui::Context, id: egui::Id, name: &str) {
    ctx.accesskit_node_builder(id, |node| node.set_author_id(name));
}

pub(crate) fn tool(id: ToolId) -> String {
    format!("onecad.tool.{}", id.automation_name())
}

impl ToolId {
    /// Stable, unlocalized registry name; the automation id is `onecad.tool.<name>`.
    pub fn automation_name(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Sketch => "sketch",
            Self::DatumPlane => "datumPlane",
            Self::OffsetPlane => "offsetPlane",
            Self::Generators => "generators",
            Self::InsertComponent => "insertComponent",
            Self::Import => "import",
            Self::Measure => "measure",
            Self::Section => "section",
            Self::Extrude => "extrude",
            Self::OffsetFace => "offsetFace",
            Self::Hole => "hole",
            Self::Draft => "draft",
            Self::Shell => "shell",
            Self::Material => "material",
            Self::Isolate => "isolate",
            Self::Hide => "hide",
            Self::More => "more",
            Self::Line => "line",
            Self::Rectangle => "rectangle",
            Self::Circle => "circle",
            Self::Arc => "arc",
            Self::Slot => "slot",
            Self::Polygon => "polygon",
            Self::Ellipse => "ellipse",
            Self::SketchMirror => "sketchMirror",
            Self::Boolean => "boolean",
            Self::LinearPattern => "linearPattern",
            Self::CircularPattern => "circularPattern",
            Self::MirrorBody => "mirrorBody",
            Self::FeaturePattern => "featurePattern",
            Self::Spline => "spline",
            Self::PenStroke => "penStroke",
            Self::Point => "point",
            Self::Dimension => "dimension",
            Self::Constraints => "constraints",
            Self::Trim => "trim",
            Self::Extend => "extend",
            Self::SketchFillet => "sketchFillet",
            Self::SketchOffset => "sketchOffset",
            Self::Project => "project",
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
            Self::Parallel => "parallel",
            Self::Perpendicular => "perpendicular",
            Self::Equal => "equal",
            Self::Midpoint => "midpoint",
            Self::Fix => "fix",
            Self::Distance => "distance",
            Self::Diameter => "diameter",
            Self::Construction => "construction",
            Self::Delete => "delete",
            Self::Revolve => "revolve",
            Self::Sweep => "sweep",
            Self::Loft => "loft",
            Self::Fillet => "fillet",
            Self::Chamfer => "chamfer",
            Self::Edit => "edit",
            Self::RollBack => "rollBack",
            Self::Suppress => "suppress",
            Self::Duplicate => "duplicate",
            Self::MoveRotate => "moveRotate",
            Self::MakeComponent => "makeComponent",
            Self::Flange => "flange",
            Self::Dogbone => "dogbone",
            Self::CheckGeometry => "checkGeometry",
        }
    }
}

/// 0-based position among the open document tabs.
pub(crate) fn tab(index: usize) -> String {
    format!("onecad.tab.{index}")
}

/// 0-based position in the feature history.
pub(crate) fn history(index: usize) -> String {
    format!("onecad.history.{index}")
}

pub(crate) fn plane(plane: WorldPlane) -> &'static str {
    match plane {
        WorldPlane::XY => "onecad.plane.xy",
        WorldPlane::XZ => "onecad.plane.xz",
        WorldPlane::YZ => "onecad.plane.yz",
    }
}

pub(crate) fn param(parameter: NumericParameter) -> String {
    format!("onecad.param.{}", parameter_name(parameter))
}

/// A sketch creation chip follows the operation card's rule: the first field
/// is the dimension input and the others are named by field.
pub(crate) fn creation_field(index: usize, field: CreationField) -> String {
    if index == 0 {
        return DIMENSION_INPUT.to_owned();
    }
    let name = match field {
        CreationField::X => "x",
        CreationField::Y => "y",
        CreationField::Length => "length",
        CreationField::Width => "width",
        CreationField::Height => "height",
        CreationField::Diameter => "diameter",
        CreationField::Radius => "radius",
        CreationField::Sweep => "sweep",
    };
    format!("onecad.param.{name}")
}

fn parameter_name(parameter: NumericParameter) -> &'static str {
    match parameter {
        NumericParameter::ExtrudeDistance => "extrudeDistance",
        NumericParameter::ExtrudeSecondDistance => "extrudeSecondDistance",
        NumericParameter::ExtrudeDraftAngle => "extrudeDraftAngle",
        NumericParameter::RevolveAngle => "revolveAngle",
        NumericParameter::FilletRadius => "filletRadius",
        NumericParameter::ChamferDistance => "chamferDistance",
        NumericParameter::ChamferSecondDistance => "chamferSecondDistance",
        NumericParameter::ChamferAngle => "chamferAngle",
        NumericParameter::ShellThickness => "shellThickness",
        NumericParameter::HoleDiameter => "holeDiameter",
        NumericParameter::HoleDepth => "holeDepth",
        NumericParameter::HoleCounterboreDiameter => "holeCounterboreDiameter",
        NumericParameter::HoleCounterboreDepth => "holeCounterboreDepth",
        NumericParameter::HoleCountersinkDiameter => "holeCountersinkDiameter",
        NumericParameter::HoleCountersinkAngle => "holeCountersinkAngle",
        NumericParameter::HoleThreadPitch => "holeThreadPitch",
        NumericParameter::HoleThreadDepth => "holeThreadDepth",
        NumericParameter::OffsetFaceDistance => "offsetFaceDistance",
        NumericParameter::LinearPatternSpacing => "linearPatternSpacing",
        NumericParameter::CircularPatternAngle => "circularPatternAngle",
        NumericParameter::FeaturePatternSpacing => "featurePatternSpacing",
        NumericParameter::FeaturePatternAngle => "featurePatternAngle",
        NumericParameter::TransformTranslation(ParameterAxis::X) => "translationX",
        NumericParameter::TransformTranslation(ParameterAxis::Y) => "translationY",
        NumericParameter::TransformTranslation(ParameterAxis::Z) => "translationZ",
        NumericParameter::TransformRotationAngle => "rotationAngle",
        NumericParameter::GearModule => "gearModule",
        NumericParameter::GearHeight => "gearHeight",
        NumericParameter::GearPressureAngle => "gearPressureAngle",
        NumericParameter::GearAxleDiameter => "gearAxleDiameter",
        NumericParameter::GearOffsetDiameter => "gearOffsetDiameter",
        NumericParameter::GearOffsetDistance => "gearOffsetDistance",
    }
}

/// One accessible egui frame, GPU-free, for asserting automation ids.
#[cfg(test)]
pub(crate) mod testing {
    use egui::accesskit::{Role, Toggled};

    /// A named node: author id, role and toggle state.
    pub(crate) type Named = (String, Role, Option<Toggled>);

    /// Runs `add` for a sizing frame and then an accessible frame; returns
    /// every node of the final AccessKit update that carries an author id.
    pub(crate) fn author_ids(mut add: impl FnMut(&mut egui::Ui)) -> Vec<Named> {
        let ctx = egui::Context::default();
        let theme = onecad_widgets::theme::load(false).expect("bundled theme");
        onecad_widgets::theme::install(&ctx, theme, false);
        ctx.enable_accesskit();
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 820.0),
            )),
            ..Default::default()
        };
        ctx.run_ui(input(), &mut add).textures_delta.clear();
        let mut output = ctx.run_ui(input(), &mut add);
        output.textures_delta.clear();
        let update = output
            .platform_output
            .accesskit_update
            .expect("AccessKit is enabled");
        update
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                let author = node.author_id()?;
                Some((author.to_owned(), node.role(), node.toggled()))
            })
            .collect()
    }

    /// The single node named `id`; a missing or duplicated id fails.
    pub(crate) fn find<'a>(named: &'a [Named], id: &str) -> &'a Named {
        let mut matching = named.iter().filter(|(author, _, _)| author == id);
        let found = matching
            .next()
            .unwrap_or_else(|| panic!("{id} missing: {named:?}"));
        assert!(matching.next().is_none(), "{id} is not unique");
        found
    }
}

#[cfg(test)]
#[path = "automation_tests.rs"]
mod tests;
