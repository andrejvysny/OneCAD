use super::*;

fn package(default: &str) -> ComponentPackage {
    package::parse(
        &format!(
            r#"
[identity]
id = "example.part"
version = "1.0.0"
revision = "sha256:{}"
[source]
kind = "generator"
generator = "iso4762"
generator_version = 1
[parameters]
thread = {{ role = "free", value = "M6" }}
length = {{ role = "free"{default} }}
"#,
            "0".repeat(64)
        ),
        Path::new("component.toml"),
    )
    .unwrap()
}

#[test]
fn every_free_parameter_requires_a_representable_effective_value() {
    for default in ["", ", value = [20, 30]"] {
        let package = package(default);
        assert!(
            resolve(&package, "example.part", &BTreeMap::new()).is_err(),
            "{default}"
        );
    }
    let package = package("");
    let overrides = BTreeMap::from([("length".into(), ComponentParamValue::Number(37.0))]);
    assert_eq!(
        resolve(&package, "example.part", &overrides).unwrap()["length"],
        overrides["length"]
    );
}

#[test]
fn source_identity_must_match_verified_generator_package() {
    let package = package(", value = 37");
    for (id, version) in [("iso4032", 1), ("iso4762", 2)] {
        let candidate = ComponentSourceRef::Generator {
            generator_id: id.into(),
            generator_version: version,
            params: BTreeMap::new(),
            extra: Default::default(),
        };
        assert!(
            source(&package, "example.part", candidate, &BTreeMap::new()).is_err(),
            "{id}@{version}"
        );
    }
}

#[test]
fn known_generators_refuse_inputs_that_worker_would_silently_default() {
    for (generator, overrides) in [
        ("iso4762", BTreeMap::new()),
        ("iso4032", BTreeMap::new()),
        ("iso7089", BTreeMap::new()),
    ] {
        let mut package = package(", value = 37");
        package.parameters.clear();
        package.source = package::SourceSpec::Generator {
            generator: generator.into(),
            generator_version: 1,
        };
        assert!(
            resolve(&package, "example.part", &overrides).is_err(),
            "{generator} missing required inputs"
        );
    }
    for (key, value) in [
        ("length", ComponentParamValue::Bool(true)),
        ("thread", ComponentParamValue::Number(6.0)),
        ("thread_detail", ComponentParamValue::Bool(false)),
    ] {
        let mut package = package(", value = 37");
        package.parameters.clear();
        for (name, value) in [
            ("length", toml::Value::Integer(37)),
            ("thread", toml::Value::String("M6".into())),
            (
                key,
                match value {
                    ComponentParamValue::Number(number) => toml::Value::Float(number),
                    ComponentParamValue::Text(text) => toml::Value::String(text),
                    ComponentParamValue::Bool(boolean) => toml::Value::Boolean(boolean),
                },
            ),
        ] {
            package.parameters.insert(
                name.into(),
                ParameterSpec {
                    role: ParameterRole::Free,
                    key: None,
                    value: Some(value),
                    domain: None,
                    snap: None,
                    min: None,
                    from: None,
                },
            );
        }
        assert!(
            resolve(&package, "example.part", &BTreeMap::new()).is_err(),
            "{key} invalid scalar kind"
        );
    }
}
