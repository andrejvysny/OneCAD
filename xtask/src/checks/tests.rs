use super::{contracts, coverage, json, layers};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

fn coverage_manifest() -> Value {
    json(&root().join("docs/qa/modeling-operation-coverage.json")).unwrap()
}

fn contracts_manifest() -> Value {
    json(&root().join("docs/qa/modeling-operation-contracts.json")).unwrap()
}

fn coverage_failure(mut manifest: Value, mutate: impl FnOnce(&mut Value), expected: &str) {
    coverage::validate(&root(), &manifest)
        .expect("baseline coverage must pass before negative control");
    mutate(&mut manifest);
    let error = coverage::validate(&root(), &manifest)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(expected),
        "expected {expected:?}, got {error}"
    );
}

fn contract_failure(mutate: impl FnOnce(&mut Value), expected: &str) {
    let mut manifest = contracts_manifest();
    let coverage = coverage_manifest();
    contracts::validate(&root(), &manifest, &coverage)
        .expect("baseline contracts must pass before negative control");
    mutate(&mut manifest);
    let error = contracts::validate(&root(), &manifest, &coverage)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(expected),
        "expected {expected:?}, got {error}"
    );
}

#[test]
fn modeling_coverage_negative_controls() {
    coverage_failure(
        coverage_manifest(),
        |m| {
            let row = m["rows"][0].clone();
            m["rows"].as_array_mut().unwrap().push(row);
        },
        "duplicates",
    );
    for (field, path) in [
        ("rustRealWorkerTest", "crates/onecad-app/tests/ghost.rs"),
        ("cppTest", "worker/tests/test_ghost.cpp"),
    ] {
        coverage_failure(
            coverage_manifest(),
            |m| {
                m["rows"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| !r[field].as_str().unwrap().is_empty())
                    .unwrap()[field] = json!(path);
            },
            "names a missing file",
        );
    }
    coverage_failure(
        coverage_manifest(),
        |m| {
            m["rows"][0]["ciJob"] = json!("macos-full");
        },
        "which is not a job in any workflow",
    );
    coverage_failure(
        coverage_manifest(),
        |m| {
            let row = m["rows"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|r| !r["rustRealWorkerTest"].as_str().unwrap().is_empty())
                .unwrap();
            row["ciJob"] = json!(row["ciJob"]
                .as_str()
                .unwrap()
                .split(';')
                .map(str::trim)
                .filter(|v| *v != "rust-8-0-1")
                .collect::<Vec<_>>()
                .join("; "));
        },
        "cites rustRealWorkerTest but names no job",
    );
    coverage_failure(
        coverage_manifest(),
        |m| {
            m["rows"]
                .as_array_mut()
                .unwrap()
                .retain(|r| r["operation"] != "Hole");
        },
        "KnownOperation knows Hole",
    );
    coverage_failure(
        coverage_manifest(),
        |m| {
            m["rows"][0]["operation"] = json!("Bevel");
            m["rows"][0]["mode"] = json!("invented");
            m["rows"][0]["corpusCase"] = json!("");
        },
        "row operation Bevel is in no registry",
    );
    coverage_failure(
        coverage_manifest(),
        |m| {
            m["nonOperationRows"] = json!([]);
        },
        "is not declared in nonOperationRows",
    );
    coverage_failure(
        coverage_manifest(),
        |m| {
            m["rows"][0].as_object_mut().unwrap().remove("uiExposure");
        },
        "missing string uiExposure",
    );
    coverage_failure(
        coverage_manifest(),
        |m| {
            m["rows"][0]["uiExposure"] = json!("exposed");
            m["rows"][0]["playwrightTest"] = json!("");
        },
        "lacks a native UI or browser lane",
    );
    coverage_failure(
        coverage_manifest(),
        |m| {
            m["rows"][0]["playwrightTest"] = json!("e2e/boolean-preview.spec.ts");
        },
        "is UI-hidden but cites a browser exposure lane",
    );
}

#[test]
fn native_modeling_coverage_requires_ui_evidence_and_actual_ci_owner() {
    for (field, value, expected) in [
        ("frontendTest", "", "lacks a native UI or browser lane"),
        (
            "frontendTest",
            "crates/onecad-app/tests/loft_ops.rs",
            "frontendTest must name native host Rust tests",
        ),
        (
            "frontendTest",
            "crates/onecad-ui/src/../../onecad-app/tests/loft_ops.rs",
            "frontendTest must name native host Rust tests",
        ),
        (
            "ciJob",
            "worker-8-0-1",
            "cites frontendTest but names no job",
        ),
        (
            "ciJob",
            "worker-8-0-1; native-host-lifecycle",
            "cites rustRealWorkerTest but names no job",
        ),
    ] {
        coverage_failure(
            coverage_manifest(),
            |manifest| {
                let row = manifest["rows"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|row| row["operation"] == "Loft")
                    .unwrap();
                row[field] = json!(value);
            },
            expected,
        );
    }
}

#[test]
fn malformed_non_operation_entries_fail_closed() {
    for value in [Value::Null, json!(42), json!(false), json!({}), json!([])] {
        coverage_failure(
            coverage_manifest(),
            |manifest| {
                manifest["nonOperationRows"]
                    .as_array_mut()
                    .unwrap()
                    .push(value);
            },
            "must be a string",
        );
    }
}

#[test]
fn modeling_contracts_negative_controls() {
    contract_failure(
        |m| {
            m["rows"][0]
                .as_object_mut()
                .unwrap()
                .remove("validationTier");
        },
        "missing validationTier",
    );
    contract_failure(
        |m| {
            m["rows"][0]["uiExposure"] = json!("maybe");
        },
        "invalid uiExposure",
    );
    contract_failure(
        |m| {
            let row = m["rows"][0].clone();
            m["rows"].as_array_mut().unwrap().push(row);
        },
        "duplicates",
    );
    contract_failure(
        |m| {
            m["rows"][0]["operation"] = json!("Bevel");
        },
        "contract operation Bevel is missing from coverage manifest",
    );
    for (op, tier, expected) in [
        (
            "Fillet",
            "Tier A plus semantic edge diagnostics",
            "only ever requests Tier B",
        ),
        ("TransformBody", "Tier B per result", "never requests it"),
    ] {
        contract_failure(
            |m| {
                m["rows"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r["operation"] == op)
                    .unwrap()["validationTier"] = json!(tier);
            },
            expected,
        );
    }
    contract_failure(
        |m| {
            let row = m["rows"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|r| r["operation"] == "Sweep")
                .unwrap();
            row["supportStatus"] = json!("supported");
            row["uiExposure"] = json!("exposed");
        },
        "worker has no Sweep dispatch arm",
    );
    contract_failure(
        |m| {
            m["rows"][0]["authoritativeHistory"] = json!("Deferred");
        },
        "is deferred but missing owner/dependency",
    );
    contract_failure(
        |m| {
            m["rows"][0]["supportStatus"] = json!("unsupported");
            m["rows"][0]["uiExposure"] = json!("unsupported");
        },
        "marked unsupported but the worker dispatches Sketch",
    );
}

fn metadata(from: &str, dependencies: Vec<Value>, extra: &[&str]) -> Value {
    let packages: Vec<_> = std::iter::once(from)
        .chain(extra.iter().copied())
        .map(|name| {
            json!({
                "id": format!("path+file:///fixture/{name}#0.1.0"), "name": name,
                "manifest_path": format!("/fixture/{name}/Cargo.toml"),
                "dependencies": if name == from { dependencies.clone() } else { vec![] },
            })
        })
        .collect();
    let members: Vec<_> = packages.iter().map(|p| p["id"].clone()).collect();
    json!({"packages": packages, "workspace_members": members})
}

fn external(name: &str, kind: Value) -> Value {
    json!({"name": name, "kind": kind})
}

fn local(name: &str, kind: Value) -> Value {
    json!({"name": name, "kind": kind, "path": format!("/fixture/{name}")})
}

#[test]
fn all_registered_edges_pass_each_declaration_kind() {
    for (from, allowed) in layers::LAYERS {
        for to in *allowed {
            for kind in [Value::Null, json!("dev"), json!("build")] {
                if *from == "onecad-extension-host"
                    && ["onecad-app", "onecad-core"].contains(to)
                    && kind != json!("dev")
                {
                    let errors =
                        layers::check(metadata(from, vec![local(to, kind)], &[to])).unwrap();
                    assert!(errors.iter().any(|error| error.contains("test-only")));
                    continue;
                }
                assert!(layers::check(metadata(from, vec![local(to, kind)], &[to]))
                    .unwrap()
                    .is_empty());
            }
        }
    }
}

#[test]
fn retired_dependencies_fail_globally_by_package_identity() {
    for (from, _) in layers::LAYERS {
        for name in [
            "tauri",
            "tauri-build",
            "tauri-runtime",
            "wry",
            "webkit",
            "webkit2gtk",
            "webkit2gtk-sys",
            "webkit6",
            "objc2-web-kit",
        ] {
            for kind in [Value::Null, json!("dev"), json!("build")] {
                let mut dep = external(name, kind);
                dep["rename"] = json!("safe_math");
                dep["optional"] = json!(true);
                dep["target"] = json!("cfg(target_os = \"windows\")");
                let errors = layers::check(metadata(from, vec![dep], &[])).unwrap();
                assert_eq!(errors.len(), 1, "{from} -> {name}");
                assert!(
                    errors[0].contains("retired desktop dependency forbidden"),
                    "{}",
                    errors[0]
                );
                assert!(errors[0].contains("renamed safe_math"));
                assert!(errors[0].contains("optional"));
                assert!(errors[0].contains("target_os = \"windows\""));
            }
        }
    }
}

#[test]
fn upward_and_concrete_product_edges_fail() {
    for (from, to) in [
        ("onecad-core", "onecad-app"),
        ("onecad-app", "onecad-interaction"),
        ("onecad-scene", "onecad-render"),
        ("onecad-render", "onecad-ui"),
        ("onecad-regen", "onecad-ui"),
        ("onecad-library-ingest", "onecad-ui"),
        ("onecad-app", "onecad-ios"),
        ("onecad-ui", "onecad-ios"),
        ("onecad-render", "onecad-ios"),
        ("onecad-embedded-worker", "onecad-app"),
        ("xtask", "onecad-core"),
    ] {
        for kind in [Value::Null, json!("dev"), json!("build")] {
            let error = layers::check(metadata(from, vec![local(to, kind)], &[to])).unwrap();
            assert!(error[0].contains("workspace dependency forbidden"));
        }
    }
}

#[test]
fn pure_layers_reject_toolkits_and_gpu() {
    for (from, _) in layers::LAYERS {
        if layers::HOSTS.contains(from) {
            continue;
        }
        for name in [
            "egui-wgpu",
            "eframe",
            "winit",
            "objc2-app-kit",
            "gtk4",
            "slint",
            "wgpu",
        ] {
            if *from == "onecad-render" && name == "wgpu" {
                continue;
            }
            for kind in [Value::Null, json!("dev"), json!("build")] {
                assert_eq!(
                    layers::check(metadata(from, vec![external(name, kind)], &[]))
                        .unwrap()
                        .len(),
                    1,
                    "{from} -> {name}"
                );
            }
        }
    }
}

#[test]
fn render_gpu_and_host_toolkits_preserve_existing_exceptions() {
    assert!(layers::check(metadata(
        "onecad-render",
        vec![
            external("wgpu", Value::Null),
            external("wgpu-types", Value::Null),
            external("naga", json!("dev"))
        ],
        &[]
    ))
    .unwrap()
    .is_empty());
    for name in ["egui", "egui-wgpu", "naga", "metal", "ash"] {
        assert_eq!(
            layers::check(metadata(
                "onecad-render",
                vec![external(name, Value::Null)],
                &[]
            ))
            .unwrap()
            .len(),
            1
        );
    }
    for from in layers::HOSTS {
        assert!(layers::check(metadata(
            from,
            vec![external("egui", Value::Null), external("wgpu", Value::Null)],
            &[]
        ))
        .unwrap()
        .is_empty());
    }
    for from in [
        "onecad-app",
        "onecad-scene",
        "onecad-interaction",
        "onecad-core",
    ] {
        assert_eq!(
            layers::check(metadata(from, vec![external("naga", json!("dev"))], &[]))
                .unwrap()
                .len(),
            1
        );
    }
    assert_eq!(
        layers::check(metadata(
            "onecad-render",
            vec![external("naga", json!("build"))],
            &[]
        ))
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn paths_network_and_unregistered_crates_fail_closed() {
    for name in ["hidden-ui", "solidcraft-gpu", "solidcraft-render"] {
        assert!(layers::check(metadata(
            "onecad-render",
            vec![local(name, Value::Null)],
            &[]
        ))
        .unwrap()[0]
            .contains("not a workspace member"));
    }
    let mut dep = local("onecad-core", Value::Null);
    dep["path"] = json!("/outside/onecad-core");
    assert!(
        layers::check(metadata("onecad-app", vec![dep], &["onecad-core"])).unwrap()[0]
            .contains("not a workspace member")
    );
    assert!(layers::check(metadata(
        "onecad-app",
        vec![external("onecad-core", Value::Null)],
        &["onecad-core"]
    ))
    .unwrap()[0]
        .contains("must point to a registered workspace member"));
    assert!(
        layers::check(metadata("onecad-surprise", vec![], &[])).unwrap()[0]
            .contains("unknown workspace package")
    );
    assert!(layers::check(metadata(
        "onecad-library",
        vec![external("reqwest", json!("dev"))],
        &[]
    ))
    .unwrap()[0]
        .contains("network dependency forbidden"));
}

#[test]
fn malformed_metadata_fails_instead_of_passing() {
    for value in [
        json!({}),
        json!({"packages": [], "workspace_members": []}),
        json!({"packages": [], "workspace_members": ["missing"]}),
        metadata(
            "onecad-app",
            vec![external("serde", json!("surprise"))],
            &[],
        ),
    ] {
        assert!(layers::check(value)
            .unwrap_err()
            .to_string()
            .contains("metadata:"));
    }
}
