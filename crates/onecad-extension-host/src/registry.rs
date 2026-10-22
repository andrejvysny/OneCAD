use crate::{
    api,
    package::read_package,
    policy::{validate_approval, validate_execution},
    sandbox::{Sandbox, SandboxLimits},
    HostError, Result,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

mod update;

const MAX_APPROVAL_BYTES: usize = 1024 * 1024;
const MAX_FAILURE_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub fingerprint: String,
    pub capabilities: Vec<api::Capability>,
    /// Approved application IDs, scoped to descriptors in this exact manifest.
    pub applications: Vec<String>,
    pub trust_native: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PackageState {
    Disabled,
    Enabled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct PackageSummary {
    pub manifest: api::Manifest,
    pub fingerprint: String,
    pub state: PackageState,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RegisteredCommand {
    pub extension_id: String,
    pub command: api::CommandContribution,
    pub enabled: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedState {
    enabled: bool,
    approval: Option<Approval>,
    #[serde(default)]
    failure: Option<String>,
}

pub struct ExtensionRegistry {
    root: PathBuf,
    packages: BTreeMap<String, PackageSummary>,
    saved: BTreeMap<String, SavedState>,
    errors: Vec<(PathBuf, String)>,
    sandbox: Sandbox,
}

impl ExtensionRegistry {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        Self::with_limits(root, SandboxLimits::default())
    }

    pub fn with_limits(root: impl Into<PathBuf>, limits: SandboxLimits) -> Result<Self> {
        let root: PathBuf = root.into();
        let root = std::path::absolute(root)?;
        fs::create_dir_all(&root)?;
        if fs::symlink_metadata(&root)?.file_type().is_symlink() {
            return Err(HostError::InvalidPackage(
                "registry root must not be a symbolic link".into(),
            ));
        }
        let state_path = root.join(".approvals.json");
        let saved = if state_path.exists() {
            let metadata = fs::symlink_metadata(&state_path)?;
            if !metadata.is_file() || metadata.len() > MAX_APPROVAL_BYTES as u64 {
                return Err(HostError::InvalidPackage("invalid approval store".into()));
            }
            serde_json::from_slice(&fs::read(state_path)?)?
        } else {
            BTreeMap::new()
        };
        let mut registry = Self {
            root,
            packages: BTreeMap::new(),
            saved,
            errors: Vec::new(),
            sandbox: Sandbox::new(limits)?,
        };
        registry.refresh()?;
        Ok(registry)
    }

    pub fn packages(&self) -> Vec<PackageSummary> {
        self.packages.values().cloned().collect()
    }

    /// Invalid discovered directories remain visible as diagnostics, never commands.
    pub fn discovery_errors(&self) -> &[(PathBuf, String)] {
        &self.errors
    }

    pub fn commands(&self) -> Vec<RegisteredCommand> {
        self.packages
            .values()
            .flat_map(|package| {
                package
                    .manifest
                    .contributions
                    .commands
                    .iter()
                    .map(|command| RegisteredCommand {
                        extension_id: package.manifest.id.clone(),
                        command: command.clone(),
                        enabled: package.state == PackageState::Enabled,
                    })
            })
            .collect()
    }

    pub fn refresh(&mut self) -> Result<()> {
        let previous = std::mem::take(&mut self.packages);
        self.errors.clear();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if let Err(error) = self.discover_one(&entry.path()) {
                let id = entry.file_name().to_string_lossy().into_owned();
                let message = error.to_string();
                // A malformed or unreadable update unloads commands and revokes
                // prior approval, even when its manifest cannot be parsed.
                self.saved.insert(
                    id.clone(),
                    SavedState {
                        failure: Some(bounded_failure(&message)),
                        ..SavedState::default()
                    },
                );
                if let Some(mut summary) = previous.get(&id).cloned() {
                    summary.state = PackageState::Failed(message.clone());
                    self.packages.insert(id, summary);
                }
                self.errors.push((entry.path(), message));
            }
        }
        self.reject_collisions();
        // Removed packages cannot regain saved grants when later rediscovered.
        self.saved.retain(|id, _| self.packages.contains_key(id));
        self.persist()
    }

    fn discover_one(&mut self, path: &Path) -> Result<()> {
        let package = read_package(path)?;
        if path.file_name().and_then(|s| s.to_str()) != Some(&package.manifest.id) {
            return Err(HostError::InvalidPackage(
                "directory must match package id".into(),
            ));
        }
        let saved = self.saved.entry(package.manifest.id.clone()).or_default();
        if saved
            .approval
            .as_ref()
            .is_some_and(|a| a.fingerprint != package.fingerprint)
        {
            *saved = SavedState {
                failure: Some("Package changed; review permissions again".into()),
                ..SavedState::default()
            };
        }
        let state = if let Some(failure) = &saved.failure {
            PackageState::Failed(failure.clone())
        } else if saved.enabled && saved.approval.is_some() {
            PackageState::Enabled
        } else {
            PackageState::Disabled
        };
        self.packages.insert(
            package.manifest.id.clone(),
            PackageSummary {
                manifest: package.manifest,
                fingerprint: package.fingerprint,
                path: path.to_owned(),
                state,
            },
        );
        Ok(())
    }

    fn reject_collisions(&mut self) {
        let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, package) in &self.packages {
            for command in &package.manifest.contributions.commands {
                owners
                    .entry(command.id.clone())
                    .or_default()
                    .push(id.clone());
            }
        }
        for ids in owners.values().filter(|ids| ids.len() > 1) {
            for id in ids {
                let message = "Command ID conflicts with another installed package".to_owned();
                if let Some(package) = self.packages.get_mut(id) {
                    package.state = PackageState::Failed(message.clone());
                }
                let saved = self.saved.entry(id.clone()).or_default();
                saved.enabled = false;
                saved.failure = Some(message);
            }
        }
    }

    pub fn install_package(&mut self, source: &Path) -> Result<String> {
        let package = read_package(source)?;
        if package.manifest.runtime == api::ExtensionRuntime::PortableComponent {
            self.sandbox.validate(&package.component)?;
        }
        if package.manifest.contributions.commands.iter().any(|c| {
            self.packages.values().any(|p| {
                p.manifest
                    .contributions
                    .commands
                    .iter()
                    .any(|existing| existing.id == c.id)
            })
        }) {
            return Err(HostError::InvalidPackage(
                "command ID conflicts with installed package".into(),
            ));
        }
        let id = package.manifest.id.clone();
        let destination = self.root.join(&id);
        if destination.exists() {
            return Err(HostError::InvalidPackage(
                "package already installed; remove it before replacing".into(),
            ));
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| HostError::InvalidPackage(e.to_string()))?
            .as_nanos();
        let staging = self
            .root
            .join(format!(".install-{}-{}-{nonce}", id, std::process::id()));
        package.copy_into(&staging)?;
        if let Err(error) = fs::rename(&staging, &destination) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
        self.refresh()?;
        Ok(id)
    }

    pub fn uninstall_package(&mut self, id: &str) -> Result<()> {
        if !api::valid_id(id) {
            return Err(HostError::InvalidPackage("invalid package id".into()));
        }
        let summary = self
            .packages
            .get(id)
            .ok_or_else(|| HostError::NotFound(id.into()))?;
        let expected = self.root.join(id);
        if summary.path != expected
            || fs::symlink_metadata(&self.root)?.file_type().is_symlink()
            || fs::symlink_metadata(&expected)?.file_type().is_symlink()
        {
            return Err(HostError::InvalidPackage(
                "uninstall requires a direct installed package directory".into(),
            ));
        }
        fs::remove_dir_all(expected)?;
        self.packages.remove(id);
        self.saved.remove(id);
        self.persist()
    }

    pub fn set_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        approval: Option<Approval>,
    ) -> Result<()> {
        if !self.packages.contains_key(id) {
            return Err(HostError::NotFound(id.into()));
        }
        if !enabled {
            self.saved.entry(id.to_owned()).or_default().enabled = false;
            self.saved.entry(id.to_owned()).or_default().failure = None;
            if let Some(package) = self.packages.get_mut(id) {
                package.state = PackageState::Disabled;
            }
            return self.persist();
        }
        let summary = self
            .packages
            .get(id)
            .ok_or_else(|| HostError::NotFound(id.into()))?;
        let package = read_package(&summary.path)?;
        if package
            .manifest
            .contributions
            .commands
            .iter()
            .any(|command| {
                self.packages.iter().any(|(other_id, other)| {
                    other_id.as_str() != id
                        && other
                            .manifest
                            .contributions
                            .commands
                            .iter()
                            .any(|existing| existing.id == command.id)
                })
            })
        {
            return Err(HostError::InvalidPackage(
                "command ID conflicts with installed package".into(),
            ));
        }
        let approval = approval
            .or_else(|| self.saved.get(id).and_then(|s| s.approval.clone()))
            .ok_or_else(|| {
                HostError::PermissionDenied("explicit package approval required".into())
            })?;
        if approval.fingerprint != package.fingerprint {
            return Err(HostError::ApprovalChanged);
        }
        validate_approval(&package.manifest, &approval)?;
        if package.manifest.runtime == api::ExtensionRuntime::NativeTrusted {
            return Err(HostError::NativeUnsupported);
        }
        self.sandbox.validate(&package.component)?;
        self.saved.insert(
            id.to_owned(),
            SavedState {
                enabled: true,
                approval: Some(approval),
                failure: None,
            },
        );
        self.refresh()
    }

    pub fn execute(
        &mut self,
        command_id: &str,
        context: &api::CommandContext,
    ) -> Result<api::CommandExecution> {
        let summary = self
            .packages
            .values()
            .find(|p| {
                p.manifest
                    .contributions
                    .commands
                    .iter()
                    .any(|c| c.id == command_id)
            })
            .cloned()
            .ok_or_else(|| HostError::NotFound(command_id.into()))?;
        if summary.state != PackageState::Enabled {
            return Err(HostError::Unavailable(
                "extension is disabled or failed".into(),
            ));
        }
        let result = self.execute_package(&summary, command_id, context);
        if let Err(error) = &result {
            if matches!(error, HostError::Unavailable(_) | HostError::NotFound(_)) {
                return result;
            }
            let saved = self.saved.entry(summary.manifest.id.clone()).or_default();
            saved.enabled = false;
            saved.failure = Some(bounded_failure(&error.to_string()));
            if matches!(error, HostError::ApprovalChanged) {
                saved.approval = None;
            }
            if let Some(package) = self.packages.get_mut(&summary.manifest.id) {
                package.state = PackageState::Failed(error.to_string());
            }
            self.persist()?;
        }
        result
    }

    fn execute_package(
        &self,
        summary: &PackageSummary,
        command_id: &str,
        context: &api::CommandContext,
    ) -> Result<api::CommandExecution> {
        let package = read_package(&summary.path)?;
        let approval = self
            .saved
            .get(&summary.manifest.id)
            .and_then(|s| s.approval.as_ref())
            .ok_or_else(|| HostError::PermissionDenied("missing approval".into()))?;
        if package.fingerprint != approval.fingerprint {
            return Err(HostError::ApprovalChanged);
        }
        validate_approval(&package.manifest, approval)?;
        let command = package
            .manifest
            .contributions
            .commands
            .iter()
            .find(|c| c.id == command_id)
            .ok_or_else(|| HostError::NotFound(command_id.into()))?;
        if (command.requires_document && !context.has_document)
            || (command.requires_selection && !context.has_selection)
        {
            return Err(HostError::Unavailable(
                "command requires an active document or selection".into(),
            ));
        }
        if package.manifest.runtime != api::ExtensionRuntime::PortableComponent {
            return Err(HostError::NativeUnsupported);
        }
        let execution = self
            .sandbox
            .execute(&package.component, command_id, context)?;
        validate_execution(&package.manifest, approval, context, &execution)?;
        Ok(execution)
    }

    fn persist(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.saved)?;
        if bytes.len() > MAX_APPROVAL_BYTES {
            return Err(HostError::InvalidPackage(
                "approval store exceeds 1 MiB".into(),
            ));
        }
        let temp = self.root.join(".approvals.tmp");
        // Replacing rather than following an existing symlink keeps persistence local.
        if fs::symlink_metadata(&temp).is_ok() {
            fs::remove_file(&temp)?;
        }
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(temp, self.root.join(".approvals.json"))?;
        Ok(())
    }
}

fn bounded_failure(message: &str) -> String {
    if message.len() <= MAX_FAILURE_BYTES {
        return message.to_owned();
    }
    let mut end = MAX_FAILURE_BYTES - '…'.len_utf8();
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_approval_write_preserves_readable_store() {
        let root = tempfile::tempdir().unwrap();
        let mut registry = ExtensionRegistry::open(root.path()).unwrap();
        let original = fs::read(root.path().join(".approvals.json")).unwrap();
        registry.saved.insert(
            "test.addon".into(),
            SavedState {
                failure: Some("x".repeat(MAX_APPROVAL_BYTES)),
                ..SavedState::default()
            },
        );
        assert!(registry.persist().is_err());
        assert_eq!(
            fs::read(root.path().join(".approvals.json")).unwrap(),
            original
        );
        assert!(ExtensionRegistry::open(root.path()).is_ok());
    }
}
