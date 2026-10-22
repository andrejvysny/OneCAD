use super::*;
use crate::package::PackageContents;

impl ExtensionRegistry {
    /// Replaces one explicitly selected local package. Approval never transfers
    /// to the replacement, including when the source has identical contents.
    pub fn update_package(
        &mut self,
        id: &str,
        expected_fingerprint: &str,
        source: &Path,
    ) -> Result<()> {
        let previous = self.checked_update_target(id, expected_fingerprint)?;
        let replacement = read_package(source)?;
        self.validate_replacement(id, &replacement)?;
        let staging = self.update_staging_path(id)?;
        replacement.copy_into(&staging)?;
        if let Err(error) = self.validate_staged_replacement(&staging, &replacement.fingerprint) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        // Source inspection and copying can take time. Fence the installed bytes
        // again immediately before moving them out of the active directory.
        if let Err(error) = self.checked_update_target(id, expected_fingerprint) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        let mut backup = staging.as_os_str().to_owned();
        backup.push(".backup");
        let backup = PathBuf::from(backup);
        self.commit_update(previous, replacement, &staging, &backup)
    }

    fn checked_update_target(&self, id: &str, fingerprint: &str) -> Result<PackageSummary> {
        if fs::symlink_metadata(&self.root)?.file_type().is_symlink() {
            return Err(HostError::InvalidPackage(
                "registry root must not be a symbolic link".into(),
            ));
        }
        let summary = self
            .packages
            .get(id)
            .ok_or_else(|| HostError::NotFound(id.into()))?;
        if summary.path != self.root.join(id) || summary.fingerprint != fingerprint {
            return Err(HostError::ApprovalChanged);
        }
        let installed = read_package(&summary.path)?;
        if installed.manifest.id != id || installed.fingerprint != fingerprint {
            return Err(HostError::ApprovalChanged);
        }
        Ok(summary.clone())
    }

    fn validate_replacement(&self, id: &str, package: &PackageContents) -> Result<()> {
        if package.manifest.id != id {
            return Err(HostError::InvalidPackage(
                "replacement package ID must match the installed package".into(),
            ));
        }
        if package.manifest.runtime == api::ExtensionRuntime::PortableComponent {
            self.sandbox.validate(&package.component)?;
        }
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
        Ok(())
    }

    fn update_staging_path(&self, id: &str) -> Result<PathBuf> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| HostError::InvalidPackage(error.to_string()))?
            .as_nanos();
        Ok(self
            .root
            .join(format!(".update-{id}-{}-{nonce}", std::process::id())))
    }

    fn validate_staged_replacement(&self, staging: &Path, fingerprint: &str) -> Result<()> {
        let staged = read_package(staging)?;
        if staged.fingerprint != fingerprint {
            return Err(HostError::InvalidPackage(
                "staged package contents changed during copying".into(),
            ));
        }
        Ok(())
    }

    fn commit_update(
        &mut self,
        previous: PackageSummary,
        replacement: PackageContents,
        staging: &Path,
        backup: &Path,
    ) -> Result<()> {
        if let Err(error) = fs::rename(&previous.path, backup) {
            let _ = fs::remove_dir_all(staging);
            return Err(error.into());
        }
        if let Err(error) = fs::rename(staging, &previous.path) {
            let restored = fs::rename(backup, &previous.path);
            let _ = fs::remove_dir_all(staging);
            return Err(rollback_error(error.into(), restored));
        }
        let id = previous.manifest.id.clone();
        let saved = self.saved.insert(id.clone(), SavedState::default());
        self.packages.insert(
            id.clone(),
            PackageSummary {
                manifest: replacement.manifest,
                fingerprint: replacement.fingerprint,
                path: previous.path.clone(),
                state: PackageState::Disabled,
            },
        );
        if let Err(error) = self.persist() {
            self.packages.insert(id.clone(), previous.clone());
            if let Some(saved) = saved {
                self.saved.insert(id, saved);
            } else {
                self.saved.remove(&id);
            }
            let restored = fs::remove_dir_all(&previous.path)
                .and_then(|()| fs::rename(backup, &previous.path));
            return Err(rollback_error(error, restored));
        }
        // A leftover hidden backup cannot register commands or inherit grants.
        let _ = fs::remove_dir_all(backup);
        Ok(())
    }
}

fn rollback_error(error: HostError, restored: std::io::Result<()>) -> HostError {
    match restored {
        Ok(()) => error,
        Err(rollback) => HostError::InvalidPackage(format!("{error}; package rollback failed: {rollback}; the previous package is retained in its hidden backup")),
    }
}
