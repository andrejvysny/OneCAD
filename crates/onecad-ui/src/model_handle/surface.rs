//! Analytic surface anchors keep radial dimensions independent of tessellation.
use super::{Geometry, anchors::vector};
use onecad_app::dto::ClassifyElementDto;
use onecad_core::{document::record::OffsetDistanceType, math::Vec3};
use onecad_scene::DVec3;

#[derive(Clone, Copy, Debug)]
pub(super) struct Surface {
    pub point: DVec3,
    pub outward: DVec3,
    cylinder: Option<(DVec3, DVec3)>,
}
impl Surface {
    pub fn from(point: Vec3, classification: &ClassifyElementDto) -> Option<Self> {
        let point = vector(point);
        let frame = classification.frame.as_ref()?;
        if !point.is_finite() || classification.kind != "face" {
            return None;
        }
        match classification.surface_type.as_str() {
            "plane" => {
                let outward = DVec3::from_array(frame.normal?).try_normalize()?;
                Some(Self {
                    point,
                    outward,
                    cylinder: None,
                })
            }
            "cylinder" => {
                let origin = DVec3::from_array(frame.origin);
                let axis = DVec3::from_array(frame.axis?).try_normalize()?;
                let radius = frame.radius.filter(|r| r.is_finite() && *r > 0.0)?;
                let center = origin + axis * (point - origin).dot(axis);
                let offset = point - center;
                if !center.is_finite() || (offset.length() - radius).abs() > 1e-6 * radius.max(1.0)
                {
                    return None;
                }
                let radial = offset.try_normalize()?;
                let outward = match frame.sidedness.as_deref()? {
                    "pin" => radial,
                    "hole" => -radial,
                    _ => return None,
                };
                Some(Self {
                    point,
                    outward,
                    cylinder: Some((center, radial)),
                })
            }
            _ => None,
        }
    }
    pub fn geometry(self, mode: OffsetDistanceType) -> Option<Geometry> {
        match mode {
            OffsetDistanceType::Offset => Some(Geometry::axis(self.point, self.outward, 1.0)),
            OffsetDistanceType::Radius | OffsetDistanceType::Diameter => {
                let (center, radial) = self.cylinder?;
                let scale = if mode == OffsetDistanceType::Diameter {
                    0.5
                } else {
                    1.0
                };
                Some(Geometry::axis(center, radial, scale))
            }
            OffsetDistanceType::Total => None,
        }
    }
}

pub(crate) fn capture(
    app: &mut crate::app::App,
    record: &onecad_core::document::record::OperationRecord,
    context: &onecad_interaction::DocumentContext,
    classification: Option<&ClassifyElementDto>,
) {
    let Some(operands) = app
        .doc
        .model_handle
        .operands
        .as_mut()
        .filter(|o| o.matches(record.record_id, context))
    else {
        return;
    };
    let onecad_core::document::record::Operation::Known(
        onecad_core::document::record::KnownOperation::OffsetFace(params),
    ) = &record.op
    else {
        return;
    };
    operands.surface = classification.and_then(|surface| {
        Surface::from(params.faces.first()?.anchor.as_ref()?.world_point, surface)
    });
    // A failed analytic read must not revive the cancelled full-cylinder mesh normal.
    operands.face = operands
        .surface
        .map(|surface| (surface.point, surface.outward));
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_app::dto::ClassifyElementFrameDto;

    fn cylinder(axis: DVec3, origin: DVec3, radius: f64, side: &str) -> ClassifyElementDto {
        ClassifyElementDto {
            kind: "face".into(),
            surface_type: "cylinder".into(),
            curve_type: String::new(),
            frame: Some(ClassifyElementFrameDto {
                origin: origin.to_array(),
                normal: None,
                axis: Some(axis.to_array()),
                radius: Some(radius),
                sidedness: Some(side.into()),
            }),
        }
    }
    #[test]
    fn external_and_internal_rotated_cylinders_use_absolute_radius_and_diameter_scale() {
        for axis in [DVec3::Z, DVec3::X, DVec3::new(1.0, 2.0, 3.0).normalize()] {
            let radial = axis.any_orthonormal_vector();
            let origin = DVec3::new(7.0, -4.0, 2.0);
            let center = origin + axis * 9.0;
            let point = center + radial * 10.0;
            for (side, sign) in [("pin", 1.0), ("hole", -1.0)] {
                let surface = Surface::from(
                    super::super::anchors::core(point),
                    &cylinder(axis, origin, 10.0, side),
                )
                .unwrap();
                for (mode, expected_base, expected_direction, expected_scale) in [
                    (OffsetDistanceType::Offset, point, radial * sign, 1.0),
                    (OffsetDistanceType::Radius, center, radial, 1.0),
                    (OffsetDistanceType::Diameter, center, radial, 0.5),
                ] {
                    let Geometry::Axis {
                        base,
                        direction,
                        scale,
                        ..
                    } = surface.geometry(mode).unwrap()
                    else {
                        panic!()
                    };
                    assert!(base.distance(expected_base) < 1e-9);
                    assert!(direction.dot(expected_direction) > 0.999999999);
                    assert_eq!(scale, expected_scale);
                    assert!(
                        (1.0 / scale
                            - if mode == OffsetDistanceType::Diameter {
                                2.0
                            } else {
                                1.0
                            })
                        .abs()
                            < 1e-9
                    );
                }
            }
        }
    }
    #[test]
    fn absent_sidedness_axis_samples_and_nonfinite_frames_fail_closed() {
        let point = Vec3::new_unchecked(10.0, 0.0, 0.0);
        let mut description = cylinder(DVec3::Z, DVec3::ZERO, 10.0, "pin");
        description.frame.as_mut().unwrap().sidedness = None;
        assert!(Surface::from(point, &description).is_none());
        description = cylinder(DVec3::Z, DVec3::ZERO, 10.0, "pin");
        assert!(Surface::from(Vec3::new_unchecked(0.0, 0.0, 5.0), &description).is_none());
        description.frame.as_mut().unwrap().axis = Some([f64::NAN, 0.0, 1.0]);
        assert!(Surface::from(point, &description).is_none());
    }
}
