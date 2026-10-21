use std::{error::Error, path::Path, process::Command};

const RETIRED: &[&str] = &[
    "src-tauri/",
    "tools/tauri-agent/",
    "node_modules/",
    "e2e-tauri/",
    "spikes/toolkit-egui/",
    "spikes/toolkit-slint/",
    "spikes/render-look/",
    "OneCAD-modeling-correctness-roadmap/",
    "plans/",
    "UX/",
    "TheComponentLibrary/",
    ".claude/skills/onecad-agent-test/",
];
const LEDGERS: &[&str] = &[
    "TODO.md",
    "PLAN.md",
    "HANDOFF.md",
    "CURRENT_STATE.md",
    "MODEL_CHOICE.md",
    "KERNELBENCH_CONTINUATION.md",
    "CLAUDE-CODE-HANDOFF-2026-09-12.md",
    "OneCAD-UX-Review-2026-09-11.md",
];
const WEB_SETUP: &[&str] = &["package.json", "bun.lock", "bun.lockb", "tsconfig.json"];

fn forbidden(path: &str) -> bool {
    RETIRED.iter().any(|prefix| path.starts_with(prefix))
        || LEDGERS.contains(&path)
        || WEB_SETUP.contains(&path)
        || (path.starts_with("scripts/") || path.starts_with("xtask/"))
            && matches!(
                Path::new(path).extension().and_then(|e| e.to_str()),
                Some("js" | "mjs" | "ts" | "tsx" | "cjs")
            )
}

fn ignored_setup(root: &Path) -> Vec<String> {
    RETIRED
        .iter()
        .chain(LEDGERS)
        .chain(WEB_SETUP)
        .map(|path| path.trim_end_matches('/'))
        .filter(|path| root.join(path).exists())
        .map(str::to_owned)
        .collect()
}

pub fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    if !args.is_empty() {
        return Err("usage: cargo xtask check-hygiene".into());
    }
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()?;
    if !output.status.success() {
        return Err("git file inventory failed".into());
    }
    let files = String::from_utf8(output.stdout)?;
    let mut violations: Vec<String> = files
        .split('\0')
        .filter(|path| !path.is_empty() && Path::new(path).exists() && forbidden(path))
        .map(str::to_owned)
        .collect();
    violations.extend(ignored_setup(Path::new(".")));
    if Path::new(".mcp.json").exists()
        && std::fs::read_to_string(".mcp.json")?
            .to_lowercase()
            .contains("tauri")
    {
        violations.push(".mcp.json: retired Tauri registration".into());
    }
    violations.sort();
    violations.dedup();
    if !violations.is_empty() {
        return Err(format!(
            "retired repository setup or management files:\n{}",
            violations.join("\n")
        )
        .into());
    }
    println!("Repository hygiene: no retired setup or duplicate management files");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_retired_setup_and_automation() {
        for path in [
            "src-tauri/icons/32.png",
            "tools/tauri-agent/src/main.ts",
            "TODO.md",
            "plans/new.md",
            "scripts/new.mjs",
            "package.json",
        ] {
            assert!(forbidden(path), "{path}");
        }
    }

    #[test]
    fn preserves_native_contracts_and_vendor_material() {
        for path in [
            "protocol/SCHEMA.md",
            "assets/native/tokens.css",
            "assets/native/cad-icons.json",
            "worker/third_party/vendor/example.js",
            "spikes/inproc-worker/Cargo.toml",
            "xtask/src/main.rs",
            ".mcp.json",
        ] {
            assert!(!forbidden(path), "{path}");
        }
    }

    #[test]
    fn ignored_retired_setup_cannot_bypass_inventory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("package.json"), "{}").unwrap();
        assert_eq!(ignored_setup(root.path()), ["package.json"]);
    }
}
