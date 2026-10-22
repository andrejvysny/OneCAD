// Adapted from rust-desktop-app xtask/src/layers.rs; see ATTRIBUTION.md.
use super::{finish, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub(super) const LAYERS: &[(&str, &[&str])] = &[
    ("onecad-core", &[]),
    ("onecad-extension-api", &[]),
    ("onecad-extension-sdk", &["onecad-extension-api"]),
    ("onecad-addon-bambu-studio", &["onecad-extension-sdk"]),
    (
        "onecad-extension-host",
        &["onecad-extension-api", "onecad-app", "onecad-core"],
    ),
    ("onecad-widgets", &[]),
    ("onecad-protocol", &[]),
    ("onecad-library", &[]),
    (
        "onecad-app",
        &["onecad-core", "onecad-protocol", "onecad-library"],
    ),
    ("onecad-scene", &["onecad-core", "onecad-protocol"]),
    (
        "onecad-interaction",
        &["onecad-app", "onecad-scene", "onecad-core"],
    ),
    (
        "onecad-render",
        &["onecad-scene", "onecad-protocol", "onecad-core"],
    ),
    (
        "onecad-ui",
        &[
            "onecad-app",
            "onecad-scene",
            "onecad-interaction",
            "onecad-render",
            "onecad-core",
            "onecad-protocol",
            "onecad-library",
            "onecad-widgets",
            "onecad-extension-api",
            "onecad-extension-host",
        ],
    ),
    (
        "onecad-mesh-fixture",
        &["onecad-app", "onecad-core", "onecad-protocol"],
    ),
    ("onecad-regen", &["onecad-app", "onecad-core"]),
    ("onecad-library-ingest", &["onecad-app"]),
    ("onecad-kernelbench", &[]),
    ("onecad-worker-stub", &["onecad-protocol"]),
    ("xtask", &[]),
];
const TOOLKITS: &[&str] = &[
    "tauri",
    "egui",
    "eframe",
    "epaint",
    "emath",
    "ecolor",
    "winit",
    "rfd",
    "bevy",
    "arboard",
    "muda",
    "tray-icon",
    "objc",
    "objc2",
    "cocoa",
    "core-graphics",
    "core-text",
    "windows",
    "gtk",
    "gtk4",
    "gdk",
    "gdk4",
    "gdk-pixbuf",
    "gio",
    "glib",
    "gobject",
    "gsk4",
    "pango",
    "iced",
    "slint",
    "i-slint",
    "fltk",
    "wry",
    "web-sys",
    "js-sys",
    "wasm-bindgen",
    "accesskit",
    "sdl2",
    "dioxus",
];
const GPU: &[&str] = &[
    "wgpu", "naga", "ash", "vulkano", "vulkan", "metal", "glow", "glutin", "glium",
];
const NETWORK: &[&str] = &["reqwest", "hyper", "ureq", "isahc", "surf", "curl", "tonic"];
const RETIRED: &[&str] = &[
    "tauri",
    "wry",
    "webkit",
    "webkit2gtk",
    "webkitgtk",
    "webkit2gtk-sys",
    "webkit6",
    "objc2-web-kit",
];

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
}
#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: PathBuf,
    dependencies: Vec<Dependency>,
}
#[derive(Deserialize)]
struct Dependency {
    name: String,
    kind: Option<String>,
    path: Option<PathBuf>,
    target: Option<String>,
    #[serde(default)]
    optional: bool,
    rename: Option<String>,
}

fn family(name: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| {
        name == *p || name.starts_with(&format!("{p}-")) || name.starts_with(&format!("{p}_"))
    })
}

fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => (),
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(part),
        }
    }
    result
}

pub(super) fn check(value: serde_json::Value) -> Result<Vec<String>> {
    let metadata: Metadata = serde_json::from_value(value).map_err(|e| format!("metadata: {e}"))?;
    if metadata.workspace_members.is_empty() {
        return Err("metadata: empty workspace_members".into());
    }
    let mut packages = Vec::new();
    for id in &metadata.workspace_members {
        let matches: Vec<_> = metadata.packages.iter().filter(|p| &p.id == id).collect();
        if matches.len() != 1 {
            return Err(format!("metadata: workspace member {id} must have one package").into());
        }
        let pkg = matches[0];
        if pkg.name.is_empty() || pkg.manifest_path.as_os_str().is_empty() {
            return Err("metadata: missing package name or manifest_path".into());
        }
        packages.push(pkg);
    }
    if packages
        .iter()
        .map(|p| &p.name)
        .collect::<BTreeSet<_>>()
        .len()
        != packages.len()
    {
        return Err("metadata: duplicate workspace package names".into());
    }
    let mut errors = BTreeSet::new();
    for pkg in &packages {
        let Some((_, allowed)) = LAYERS.iter().find(|(name, _)| *name == pkg.name) else {
            errors.insert(format!(
                "{}: unknown workspace package; register it in xtask/src/checks/layers.rs",
                pkg.name
            ));
            continue;
        };
        for dep in &pkg.dependencies {
            if let Some(error) = check_dependency(pkg, dep, &packages, allowed)? {
                errors.insert(error);
            }
        }
    }
    Ok(errors.into_iter().collect())
}

fn check_dependency(
    pkg: &Package,
    dep: &Dependency,
    packages: &[&Package],
    allowed: &[&str],
) -> Result<Option<String>> {
    let name = dep.name.as_str();
    let kind = dep.kind.as_deref().unwrap_or("normal");
    if name.is_empty() {
        return Err("metadata: missing dependency name".into());
    }
    if !["normal", "dev", "build"].contains(&kind) {
        return Err(format!("metadata: unknown dependency kind {kind}").into());
    }
    let mut details = vec![kind.to_owned()];
    if let Some(target) = &dep.target {
        details.push(target.clone());
    }
    if dep.optional {
        details.push("optional".to_owned());
    }
    if let Some(rename) = &dep.rename {
        details.push(format!("renamed {rename}"));
    }
    let context = format!("{} -> {name} ({})", pkg.name, details.join(", "));
    let target = packages.iter().find(|p| p.name == name);
    let identity = name.to_ascii_lowercase();
    let reason = if family(&identity, RETIRED) || identity.starts_with("webkit") {
        Some("retired desktop dependency forbidden")
    } else if let Some(path) = &dep.path {
        if target.is_none_or(|p| {
            normalized(path) != normalized(p.manifest_path.parent().unwrap_or(Path::new("")))
        }) {
            Some("local dependency is not a workspace member at its declared path")
        } else if !allowed.contains(&name) {
            Some("workspace dependency forbidden")
        } else {
            None
        }
    } else if target.is_some()
        || LAYERS.iter().any(|(n, _)| *n == name)
        || name.starts_with("onecad-")
    {
        Some("OneCAD dependency must point to a registered workspace member")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Ok(Some(format!("{context}: {reason}")));
    }
    let host = ["onecad-ui", "onecad-widgets"].contains(&pkg.name.as_str());
    let renderer_gpu = pkg.name == "onecad-render"
        && (family(name, &["wgpu"]) || (name == "naga" && kind == "dev"));
    let reason = if pkg.name == "onecad-extension-host"
        && ["onecad-app", "onecad-core"].contains(&name)
        && kind != "dev"
    {
        Some("extension sandbox application dependencies are test-only")
    } else if !host && family(name, TOOLKITS) {
        Some("UI/platform toolkit forbidden")
    } else if !host && family(name, GPU) && !renderer_gpu {
        Some("GPU dependency forbidden")
    } else if pkg.name == "onecad-library" && family(name, NETWORK) {
        Some("network dependency forbidden")
    } else {
        None
    };
    Ok(reason.map(|reason| format!("{context}: {reason}")))
}

pub(super) fn run(root: &Path) -> Result<()> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let value = serde_json::from_slice(&output.stdout)?;
    let errors = check(value)?;
    finish("Rust dependency layering failed", errors)?;
    // Read the member count from the same metadata response used for validation.
    let metadata: Metadata = serde_json::from_slice(&output.stdout)?;
    println!(
        "Rust layers: {} workspace packages, no violations",
        metadata.workspace_members.len()
    );
    Ok(())
}
