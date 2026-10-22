use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

use super::ExportArtifact;

/// Trusted host configuration; guests can refer only to `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalApplicationDescriptor {
    pub id: String,
    pub display_name: String,
    pub macos_bundle_names: Vec<String>,
    pub macos_executable_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalApplicationSettings {
    pub executable: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchOutcome {
    /// The OS accepted the handoff; application import is not observable here.
    Launched,
    Failed(String),
}

pub trait ApplicationLauncher: Send + Sync {
    fn launch(&self, executable: &Path, artifact: &Path) -> Result<(), String>;
}

pub struct ProcessApplicationLauncher;

impl ApplicationLauncher for ProcessApplicationLauncher {
    fn launch(&self, executable: &Path, artifact: &Path) -> Result<(), String> {
        let mut child = launch_command(executable, artifact)
            .spawn()
            .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        if macos_launch_bundle(executable).is_some() {
            return wait_for_launch_services(&mut child);
        }
        // Reap the child without blocking the bounded document service lane.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(any(target_os = "macos", test))]
fn wait_for_launch_services(child: &mut std::process::Child) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                return Err(format!("macOS could not open the exported file: {status}"))
            }
            None if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("macOS application handoff timed out".into());
            }
        }
    }
}

fn launch_command(executable: &Path, artifact: &Path) -> Command {
    let mut command = Command::new(executable);
    #[cfg(target_os = "macos")]
    if let Some(bundle) = macos_launch_bundle(executable) {
        command = Command::new("/usr/bin/open");
        command.arg("-a").arg(bundle);
    }
    command.arg(artifact);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(any(target_os = "macos", test))]
fn macos_launch_bundle(executable: &Path) -> Option<&Path> {
    if executable
        .extension()
        .is_some_and(|extension| extension == "app")
    {
        return Some(executable);
    }
    application_bundle(executable)
}

#[cfg(any(target_os = "macos", test))]
fn application_bundle(executable: &Path) -> Option<&Path> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then_some(bundle)
}

/// A launch failure never changes or deletes the completed export.
pub fn launch_artifact(
    descriptor: &ExternalApplicationDescriptor,
    settings: &ExternalApplicationSettings,
    artifact: &ExportArtifact,
    launcher: &dyn ApplicationLauncher,
) -> LaunchOutcome {
    let result = (|| {
        if descriptor.id.is_empty()
            || !artifact.path().is_absolute()
            || !artifact.path().is_file()
            || artifact.path().extension().is_none_or(|ext| ext != "3mf")
        {
            return Err("invalid external application or exported artifact".into());
        }
        let executable = resolve_executable(descriptor, settings)?;
        launcher.launch(&executable, artifact.path())
    })();
    match result {
        Ok(()) => LaunchOutcome::Launched,
        Err(error) => LaunchOutcome::Failed(error),
    }
}

pub fn resolve_executable(
    descriptor: &ExternalApplicationDescriptor,
    settings: &ExternalApplicationSettings,
) -> Result<PathBuf, String> {
    let executable = settings
        .executable
        .clone()
        .or_else(|| discover_application(descriptor))
        .ok_or_else(|| {
            format!(
                "Choose the executable for {} in extension settings",
                descriptor.display_name
            )
        })?;
    let is_application = executable.is_file();
    #[cfg(target_os = "macos")]
    let is_application = is_application || is_macos_bundle(&executable);
    if !executable.is_absolute() || !is_application {
        return Err("Choose an existing absolute executable path in extension settings".into());
    }
    Ok(executable)
}

/// Discovery is a convenience default. A saved user choice always takes priority.
pub fn discover_application(descriptor: &ExternalApplicationDescriptor) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        discover_in_roots(descriptor, &roots)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = descriptor;
        None
    }
}

#[cfg(any(target_os = "macos", test))]
fn discover_in_roots(
    descriptor: &ExternalApplicationDescriptor,
    roots: &[PathBuf],
) -> Option<PathBuf> {
    for root in roots {
        for bundle in &descriptor.macos_bundle_names {
            if !bundle.ends_with(".app") || Path::new(bundle).components().count() != 1 {
                continue;
            }
            let bundle = root.join(bundle);
            if let Some(executable) = &descriptor.macos_executable_name {
                if Path::new(executable).components().count() == 1 {
                    let candidate = bundle.join("Contents/MacOS").join(executable);
                    if candidate.is_absolute() && candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
            if bundle.is_absolute() && is_macos_bundle(&bundle) {
                return Some(bundle);
            }
        }
    }
    None
}

#[cfg(any(target_os = "macos", test))]
fn is_macos_bundle(path: &Path) -> bool {
    path.is_dir()
        && path.extension().is_some_and(|extension| extension == "app")
        && path.join("Contents/Info.plist").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_paths_are_single_literal_arguments() {
        let executable = Path::new("/Applications/Slicer Tools/slicer");
        let artifact = Path::new("/tmp/my model; $(touch unwanted).3mf");
        let command = launch_command(executable, artifact);
        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![artifact.as_os_str()]
        );
    }

    #[test]
    fn macos_executable_paths_route_to_the_bundle() {
        assert_eq!(
            application_bundle(Path::new(
                "/Applications/Test Slicer.app/Contents/MacOS/slicer"
            )),
            Some(Path::new("/Applications/Test Slicer.app"))
        );
        assert_eq!(application_bundle(Path::new("/usr/local/bin/slicer")), None);
        assert_eq!(
            macos_launch_bundle(Path::new("/Applications/Slicer.app")),
            Some(Path::new("/Applications/Slicer.app"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_helper_acceptance_is_reported() {
        let mut child = Command::new("/bin/false").spawn().unwrap();
        assert!(wait_for_launch_services(&mut child)
            .unwrap_err()
            .contains("could not open"));
    }

    #[test]
    fn macos_discovery_uses_bundle_binary_and_saved_choice_wins() {
        let root = tempfile::tempdir().unwrap();
        let descriptor = ExternalApplicationDescriptor {
            id: "test-slicer".into(),
            display_name: "Test slicer".into(),
            macos_bundle_names: vec!["Test Slicer.app".into()],
            macos_executable_name: Some("slicer".into()),
        };
        let binary = root.path().join("Test Slicer.app/Contents/MacOS/slicer");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "").unwrap();
        assert_eq!(
            discover_in_roots(&descriptor, &[root.path().to_path_buf()]),
            Some(binary)
        );
        std::fs::write(
            root.path().join("Test Slicer.app/Contents/Info.plist"),
            "bundle metadata",
        )
        .unwrap();
        let mut bundle_only = descriptor.clone();
        bundle_only.macos_executable_name = None;
        assert_eq!(
            discover_in_roots(&bundle_only, &[root.path().to_path_buf()]),
            Some(root.path().join("Test Slicer.app"))
        );
        let chosen = root.path().join("my slicer");
        std::fs::write(&chosen, "").unwrap();
        assert_eq!(
            resolve_executable(
                &descriptor,
                &ExternalApplicationSettings {
                    executable: Some(chosen.clone())
                }
            ),
            Ok(chosen)
        );
    }
}
