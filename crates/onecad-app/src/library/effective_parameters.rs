//! Resolve generator inputs from a verified package, keeping overrides separate.
use super::*;
#[cfg(test)]
mod tests;

pub(crate) fn resolve(
    package: &ComponentPackage,
    component_id: &str,
    overrides: &BTreeMap<String, ComponentParamValue>,
) -> Result<BTreeMap<String, ComponentParamValue>, ApiError> {
    validate_component_params("component parameters", package, component_id, overrides)?;
    let mut effective = free_defaults(&package.parameters);
    effective.extend(overrides.clone());
    for (key, spec) in &package.parameters {
        if spec.role == ParameterRole::Free && !effective.contains_key(key) {
            return Err(ApiError::InvalidCommand(format!(
                "component {component_id} parameter `{key}` requires a scalar default or override"
            )));
        }
    }
    validate_component_params("component defaults", package, component_id, &effective)?;
    if let package::SourceSpec::Generator { generator, .. } = &package.source {
        validate_generator_inputs(generator, &effective)?;
    }
    Ok(effective)
}

fn validate_generator_inputs(
    generator: &str,
    effective: &BTreeMap<String, ComponentParamValue>,
) -> Result<(), ApiError> {
    if !matches!(generator, "iso4762" | "iso4032" | "iso7089") {
        return Ok(());
    }
    for (key, numeric, required) in [
        ("length", true, generator == "iso4762"),
        ("thread", false, true),
        ("thread_detail", false, false),
    ] {
        let valid = match effective.get(key) {
            None => !required,
            Some(ComponentParamValue::Number(_)) => numeric,
            Some(ComponentParamValue::Text(_)) => !numeric,
            _ => false,
        };
        if !valid {
            return Err(ApiError::InvalidCommand(format!(
                "generator {generator} parameter `{key}` requires {}{}",
                if required { "an explicit " } else { "a " },
                if numeric { "number" } else { "text value" },
            )));
        }
    }
    Ok(())
}

pub(crate) fn source(
    package: &ComponentPackage,
    component_id: &str,
    source: ComponentSourceRef,
    overrides: &BTreeMap<String, ComponentParamValue>,
) -> Result<ComponentSourceRef, ApiError> {
    match source {
        ComponentSourceRef::Generator {
            generator_id,
            generator_version,
            extra,
            ..
        } => {
            if !matches!(&package.source, package::SourceSpec::Generator {
                generator, generator_version: version,
            } if *generator == generator_id && *version == generator_version)
            {
                return Err(ApiError::InvalidCommand(
                    "component generator identity disagrees with verified snapshot".into(),
                ));
            }
            Ok(ComponentSourceRef::Generator {
                generator_id,
                generator_version,
                params: resolve(package, component_id, overrides)?,
                extra,
            })
        }
        source => source_with_free_params(source, overrides, component_id),
    }
}
