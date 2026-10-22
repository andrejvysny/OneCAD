//! Typed numeric edit sites. Field names mirror the core scalar registry exactly;
//! no record is lowered through JSON and no absent optional field is manufactured.
use onecad_core::{
    document::{
        record::{ExtrudeMode, HoleType, KnownOperation, OffsetDistanceType, Operation},
        variables::Scalar,
    },
    expr::Dimension,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterAxis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericParameter {
    ExtrudeDistance,
    ExtrudeSecondDistance,
    ExtrudeDraftAngle,
    RevolveAngle,
    FilletRadius,
    ChamferDistance,
    ChamferSecondDistance,
    ChamferAngle,
    ShellThickness,
    HoleDiameter,
    HoleDepth,
    HoleCounterboreDiameter,
    HoleCounterboreDepth,
    HoleCountersinkDiameter,
    HoleCountersinkAngle,
    HoleThreadPitch,
    HoleThreadDepth,
    OffsetFaceDistance,
    LinearPatternSpacing,
    CircularPatternAngle,
    FeaturePatternSpacing,
    FeaturePatternAngle,
    TransformTranslation(ParameterAxis),
    TransformRotationAngle,
    GearModule,
    GearHeight,
    GearPressureAngle,
    GearAxleDiameter,
    GearOffsetDiameter,
    GearOffsetDistance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NumericParameterDescriptor {
    pub parameter: NumericParameter,
    pub label: &'static str,
    pub dimension: Dimension,
    pub scalar: Scalar,
}

const PARAMETERS: &[NumericParameter] = &[
    NumericParameter::ExtrudeDistance,
    NumericParameter::ExtrudeSecondDistance,
    NumericParameter::ExtrudeDraftAngle,
    NumericParameter::RevolveAngle,
    NumericParameter::FilletRadius,
    NumericParameter::ChamferDistance,
    NumericParameter::ChamferSecondDistance,
    NumericParameter::ChamferAngle,
    NumericParameter::ShellThickness,
    NumericParameter::HoleDiameter,
    NumericParameter::HoleDepth,
    NumericParameter::HoleCounterboreDiameter,
    NumericParameter::HoleCounterboreDepth,
    NumericParameter::HoleCountersinkDiameter,
    NumericParameter::HoleCountersinkAngle,
    NumericParameter::HoleThreadPitch,
    NumericParameter::HoleThreadDepth,
    NumericParameter::OffsetFaceDistance,
    NumericParameter::LinearPatternSpacing,
    NumericParameter::CircularPatternAngle,
    NumericParameter::FeaturePatternSpacing,
    NumericParameter::FeaturePatternAngle,
    NumericParameter::TransformTranslation(ParameterAxis::X),
    NumericParameter::TransformTranslation(ParameterAxis::Y),
    NumericParameter::TransformTranslation(ParameterAxis::Z),
    NumericParameter::TransformRotationAngle,
    NumericParameter::GearModule,
    NumericParameter::GearHeight,
    NumericParameter::GearPressureAngle,
    NumericParameter::GearAxleDiameter,
    NumericParameter::GearOffsetDiameter,
    NumericParameter::GearOffsetDistance,
];

/// Available controls carry the original scalar, including its verbatim expression.
/// Pattern counts are u32, not registered Scalar fields, so are deliberately absent.
pub fn numeric_parameters(operation: &Operation) -> Vec<NumericParameterDescriptor> {
    let Operation::Known(known) = operation else {
        return Vec::new();
    };
    let mut copy = known.clone();
    let slots = copy.scalars_mut();
    PARAMETERS
        .iter()
        .filter_map(|parameter| {
            if !parameter.active(known) {
                return None;
            }
            let mut matching = slots
                .iter()
                .filter(|(name, _, _)| *name == parameter.field_name());
            let (_, dimension, scalar) = matching.next()?;
            if *dimension != parameter.dimension() || matching.next().is_some() {
                return None;
            }
            Some(NumericParameterDescriptor {
                parameter: *parameter,
                label: parameter.label(),
                dimension: parameter.dimension(),
                scalar: (**scalar).clone(),
            })
        })
        .collect()
}

impl NumericParameter {
    pub fn site(self) -> &'static str {
        match self.dimension() {
            Dimension::Angle => "angle",
            _ => "length",
        }
    }
    pub fn dimension(self) -> Dimension {
        match self {
            Self::ExtrudeDraftAngle
            | Self::RevolveAngle
            | Self::ChamferAngle
            | Self::HoleCountersinkAngle
            | Self::CircularPatternAngle
            | Self::FeaturePatternAngle
            | Self::TransformRotationAngle
            | Self::GearPressureAngle => Dimension::Angle,
            _ => Dimension::Length,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::ExtrudeDistance => "Extrude distance",
            Self::ExtrudeSecondDistance => "Second extrude distance",
            Self::ExtrudeDraftAngle => "Draft angle",
            Self::RevolveAngle => "Revolve angle",
            Self::FilletRadius => "Fillet radius",
            Self::ChamferDistance => "Chamfer distance",
            Self::ChamferSecondDistance => "Second chamfer distance",
            Self::ChamferAngle => "Chamfer angle",
            Self::ShellThickness => "Shell thickness",
            Self::HoleDiameter => "Hole diameter",
            Self::HoleDepth => "Hole depth",
            Self::HoleCounterboreDiameter => "Counterbore diameter",
            Self::HoleCounterboreDepth => "Counterbore depth",
            Self::HoleCountersinkDiameter => "Countersink diameter",
            Self::HoleCountersinkAngle => "Countersink angle",
            Self::HoleThreadPitch => "Thread pitch",
            Self::HoleThreadDepth => "Thread depth",
            Self::OffsetFaceDistance => "Offset face distance",
            Self::LinearPatternSpacing | Self::FeaturePatternSpacing => "Pattern spacing",
            Self::CircularPatternAngle | Self::FeaturePatternAngle => "Pattern angle",
            Self::TransformTranslation(ParameterAxis::X) => "Translation X",
            Self::TransformTranslation(ParameterAxis::Y) => "Translation Y",
            Self::TransformTranslation(ParameterAxis::Z) => "Translation Z",
            Self::TransformRotationAngle => "Rotation angle",
            Self::GearModule => "Gear module",
            Self::GearHeight => "Gear height",
            Self::GearPressureAngle => "Pressure angle",
            Self::GearAxleDiameter => "Axle hole diameter",
            Self::GearOffsetDiameter => "Offset hole diameter",
            Self::GearOffsetDistance => "Offset hole position",
        }
    }
    fn field_name(self) -> &'static str {
        match self {
            Self::ExtrudeDistance => "Extrude.distance",
            Self::ExtrudeSecondDistance => "Extrude.distance2",
            Self::ExtrudeDraftAngle => "Extrude.draftAngleDeg",
            Self::RevolveAngle => "Revolve.angleDeg",
            Self::FilletRadius => "Fillet.radius",
            Self::ChamferDistance => "Chamfer.radius",
            Self::ChamferSecondDistance => "Chamfer.distance2",
            Self::ChamferAngle => "Chamfer.angleDeg",
            Self::ShellThickness => "Shell.thickness",
            Self::HoleDiameter => "Hole.diameter",
            Self::HoleDepth => "Hole.depth",
            Self::HoleCounterboreDiameter => "Hole.cbDiameter",
            Self::HoleCounterboreDepth => "Hole.cbDepth",
            Self::HoleCountersinkDiameter => "Hole.csDiameter",
            Self::HoleCountersinkAngle => "Hole.csAngleDeg",
            Self::HoleThreadPitch => "Hole.thread.pitchMm",
            Self::HoleThreadDepth => "Hole.thread.depthMm",
            Self::OffsetFaceDistance => "OffsetFace.distance",
            Self::LinearPatternSpacing => "LinearPattern.spacing",
            Self::CircularPatternAngle => "CircularPattern.angleDeg",
            Self::FeaturePatternSpacing => "FeaturePattern.layout.spacing",
            Self::FeaturePatternAngle => "FeaturePattern.layout.angleDeg",
            Self::TransformTranslation(ParameterAxis::X) => "TransformBody.translate[0]",
            Self::TransformTranslation(ParameterAxis::Y) => "TransformBody.translate[1]",
            Self::TransformTranslation(ParameterAxis::Z) => "TransformBody.translate[2]",
            Self::TransformRotationAngle => "TransformBody.rotate.angleDeg",
            Self::GearModule => "Gear.module",
            Self::GearHeight => "Gear.height",
            Self::GearPressureAngle => "Gear.pressureAngleDeg",
            Self::GearAxleDiameter => "Gear.axleHoleDiameter",
            Self::GearOffsetDiameter => "Gear.offsetHoleDiameter",
            Self::GearOffsetDistance => "Gear.offsetHoleOffset",
        }
    }
    /// Widget identity salt; the Extrude distance keeps its established id.
    pub fn salt(self) -> &'static str {
        match self {
            Self::ExtrudeDistance => "extrude-distance",
            other => other.field_name(),
        }
    }
    pub(crate) fn current_scalar(self, operation: &Operation) -> Result<Scalar, String> {
        let mut copy = operation.clone();
        Ok(self.slot(&mut copy)?.clone())
    }
    /// Present slot whatever the mode; absent optional slots are not registered.
    pub(crate) fn registered_scalar(self, operation: &Operation) -> Result<Scalar, String> {
        let mut copy = operation.clone();
        Ok(self.registered_slot(&mut copy)?.clone())
    }
    pub(crate) fn is_active(self, operation: &Operation) -> bool {
        matches!(operation, Operation::Known(known) if self.active(known))
    }
    /// Writes an already validated value into the active slot.
    pub(crate) fn write(self, operation: &mut Operation, scalar: Scalar) -> Result<(), String> {
        *self.slot(operation)? = scalar;
        Ok(())
    }
    fn slot(self, operation: &mut Operation) -> Result<&mut Scalar, String> {
        if !self.is_active(operation) {
            return Err(self.unsupported());
        }
        self.registered_slot(operation)
    }
    fn registered_slot(self, operation: &mut Operation) -> Result<&mut Scalar, String> {
        let Operation::Known(known) = operation else {
            return Err(self.unsupported());
        };
        let mut matching = known
            .scalars_mut()
            .into_iter()
            .filter(|(name, _, _)| *name == self.field_name());
        let (_, dimension, scalar) = matching.next().ok_or_else(|| self.unsupported())?;
        if dimension != self.dimension() || matching.next().is_some() {
            return Err(
                "Numeric parameter registry is ambiguous or has a different dimension".into(),
            );
        }
        Ok(scalar)
    }
    pub(crate) fn unsupported(self) -> String {
        format!(
            "{} is not available for this operation or mode",
            self.label()
        )
    }
    pub(crate) fn apply(self, operation: &mut Operation, scalar: Scalar) -> Result<(), String> {
        self.validate_value(operation, scalar.value)?;
        *self.slot(operation)? = scalar;
        validate_operation(operation)
    }
    fn active(self, operation: &KnownOperation) -> bool {
        match (self, operation) {
            (Self::ExtrudeDistance, KnownOperation::Extrude(p)) => {
                p.mode == ExtrudeMode::Blind
                    || (!p.two_directions && p.mode == ExtrudeMode::Symmetric)
            }
            (Self::ExtrudeSecondDistance, KnownOperation::Extrude(p)) => {
                p.two_directions && p.mode2 == ExtrudeMode::Blind
            }
            (Self::ExtrudeDraftAngle, KnownOperation::Extrude(p)) => {
                !p.two_directions && p.mode == ExtrudeMode::Blind
            }
            (
                Self::HoleCounterboreDiameter | Self::HoleCounterboreDepth,
                KnownOperation::Hole(p),
            ) => p.hole_type == HoleType::Counterbore,
            (
                Self::HoleCountersinkDiameter | Self::HoleCountersinkAngle,
                KnownOperation::Hole(p),
            ) => p.hole_type == HoleType::Countersink,
            // Every remaining variant is discriminated by its exact registered name.
            _ => true,
        }
    }
    pub(crate) fn validate_value(self, operation: &Operation, value: f64) -> Result<(), String> {
        if !value.is_finite() {
            return Err("Enter a finite value".into());
        }
        let valid = match self {
            Self::ExtrudeDistance | Self::ExtrudeSecondDistance => {
                value != 0.0
                    && !matches!(operation, Operation::Known(KnownOperation::Extrude(p)) if p.two_directions && value < 0.0)
            }
            Self::ExtrudeDraftAngle => (-89.0..=89.0).contains(&value),
            Self::RevolveAngle => (0.5..=360.0).contains(&value),
            Self::FilletRadius | Self::ChamferDistance | Self::ChamferSecondDistance => {
                value >= 0.1
            }
            Self::HoleDiameter
            | Self::HoleDepth
            | Self::HoleCounterboreDiameter
            | Self::HoleCounterboreDepth
            | Self::HoleCountersinkDiameter => value >= 0.01,
            Self::CircularPatternAngle => (1.0..=360.0).contains(&value),
            Self::OffsetFaceDistance => !matches!(operation,
                Operation::Known(KnownOperation::OffsetFace(p)) if p.distance_type != OffsetDistanceType::Offset && value <= 0.0),
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(format!("{} is outside its authorable range", self.label()))
        }
    }
}

// Typed entry parity: ModelToolController refuseBelowEdgeOpMinimum (0.1 mm),
// modelToolMachine draft ±89°, revolve confirm 0.5° plus onRevolveChip [0,360],
// circular clampCircularAngle [1,360], and holeMachine dim 0.01 mm. Reject rather
// than clamp, preserving the expression. Shell's 0.1 floor is drag-only: its
// typed path has no such floor and keeps authoritative worker failure behavior.
pub(crate) fn validate_operation(operation: &Operation) -> Result<(), String> {
    match operation {
        Operation::Known(KnownOperation::Extrude(p)) => p.validate(),
        Operation::Known(KnownOperation::Revolve(p)) => p.validate(),
        Operation::Known(KnownOperation::Gear(p)) => p.validate(),
        Operation::Known(KnownOperation::Chamfer(p)) => p.validate(),
        Operation::Known(KnownOperation::Shell(p)) => p.validate(),
        Operation::Known(KnownOperation::Hole(p)) => p.validate(),
        Operation::Known(KnownOperation::OffsetFace(p)) => p.validate(),
        Operation::Known(KnownOperation::TransformBody(p)) => p.validate(),
        Operation::Known(op) => crate::modeling_options::validate(op),
        _ => Ok(()),
    }
}
