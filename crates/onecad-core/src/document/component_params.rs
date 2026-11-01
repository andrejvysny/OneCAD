//! Free-parameter application for placed components (Component Library
//! placement flow). Pure: no library or worker access, so the app crate's
//! placement path and the draft/preview path derive the SAME record from the
//! same inputs.

use std::collections::BTreeMap;

use super::record::{ComponentParamValue, ComponentSourceRef, PlaceComponentParams};

/// Why free params could not be applied to a component source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComponentParamError {
    /// The source carries geometry baked at authoring time; recording params
    /// against it would show a designation over unrelated geometry (spec §0
    /// invariant 4).
    #[error("component source carries baked geometry, which has no free parameters to set")]
    BakedSource,
}

impl ComponentSourceRef {
    /// Carries `free` into the source kinds that re-derive geometry from
    /// params. A `Generator`'s params are replaced wholesale (they are the
    /// RESOLVED set); a `Profile`'s are MERGED over its seeded defaults
    /// (`length` is required, so dropping the seed would author an invalid
    /// record). `Embedded`/`Document` accept only an empty `free`.
    ///
    /// # Errors
    /// [`ComponentParamError::BakedSource`] for a baked kind with params.
    pub fn with_free_params(
        self,
        free: &BTreeMap<String, ComponentParamValue>,
    ) -> Result<Self, ComponentParamError> {
        match self {
            Self::Generator {
                generator_id,
                generator_version,
                extra,
                ..
            } => Ok(Self::Generator {
                generator_id,
                generator_version,
                params: free.clone(),
                extra,
            }),
            Self::Profile {
                sha256,
                codec,
                brep_format,
                mut params,
                extra,
            } => {
                params.extend(free.clone());
                Ok(Self::Profile {
                    sha256,
                    codec,
                    brep_format,
                    params,
                    extra,
                })
            }
            baked @ (Self::Embedded { .. } | Self::Document { .. }) if free.is_empty() => Ok(baked),
            Self::Embedded { .. } | Self::Document { .. } => Err(ComponentParamError::BakedSource),
        }
    }
}

impl PlaceComponentParams {
    /// A copy with `params` replaced and `source` re-derived from them, so a
    /// draft's free-param edit and its eventual commit cannot disagree.
    ///
    /// # Errors
    /// [`ComponentParamError::BakedSource`] when `params` is non-empty and the
    /// source is `Embedded`/`Document`.
    pub fn with_free_params(
        &self,
        params: BTreeMap<String, ComponentParamValue>,
    ) -> Result<Self, ComponentParamError> {
        let source = self.source.clone().with_free_params(&params)?;
        Ok(Self {
            params,
            source,
            ..self.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::record::{FrozenPlacement, ImportSourceCodec, TransformRotation};
    use crate::document::refs::Extra;
    use crate::document::variables::Scalar;

    fn num(v: f64) -> ComponentParamValue {
        ComponentParamValue::Number(v)
    }

    fn placed(source: ComponentSourceRef) -> PlaceComponentParams {
        PlaceComponentParams {
            component_id: "onecad.std.iso4762".into(),
            component_version: "1.0.0".into(),
            component_revision: format!("sha256:{}", "0".repeat(64)),
            params: BTreeMap::new(),
            source,
            mate: None,
            placement: FrozenPlacement {
                translate: [Scalar::new(1.0), Scalar::new(2.0), Scalar::new(3.0)],
                rotate: TransformRotation::default(),
            },
            extra: Extra::new(),
        }
    }

    fn generator(params: BTreeMap<String, ComponentParamValue>) -> ComponentSourceRef {
        ComponentSourceRef::Generator {
            generator_id: "iso4762".into(),
            generator_version: 1,
            params,
            extra: Extra::new(),
        }
    }

    fn embedded() -> ComponentSourceRef {
        ComponentSourceRef::Embedded {
            sha256: "a".repeat(64),
            codec: ImportSourceCodec::Brep,
            brep_format: Some(1),
            extra: Extra::new(),
        }
    }

    #[test]
    fn generator_params_are_replaced_wholesale() {
        let base = placed(generator([("length".to_string(), num(10.0))].into()));
        let want: BTreeMap<_, _> = [("length".to_string(), num(25.0))].into();
        let out = base.with_free_params(want.clone()).unwrap();
        assert_eq!(out.params, want);
        assert_eq!(out.source, generator(want));
        assert_eq!(out.placement, base.placement);
    }

    #[test]
    fn profile_params_merge_over_seeded_defaults() {
        let seeded: BTreeMap<_, _> = [
            ("length".to_string(), num(500.0)),
            ("k".to_string(), num(1.0)),
        ]
        .into();
        let profile = ComponentSourceRef::Profile {
            sha256: "b".repeat(64),
            codec: ImportSourceCodec::Brep,
            brep_format: Some(1),
            params: seeded,
            extra: Extra::new(),
        };
        let out = placed(profile)
            .with_free_params([("length".to_string(), num(30.0))].into())
            .unwrap();
        let ComponentSourceRef::Profile { params, .. } = out.source else {
            panic!("profile stays profile")
        };
        assert_eq!(params["length"], num(30.0));
        assert_eq!(params["k"], num(1.0));
    }

    #[test]
    fn baked_kinds_refuse_params_but_accept_none() {
        let base = placed(embedded());
        let err = base
            .with_free_params([("length".to_string(), num(1.0))].into())
            .unwrap_err();
        assert_eq!(err, ComponentParamError::BakedSource);
        assert_eq!(base.with_free_params(BTreeMap::new()).unwrap(), base);
    }
}
