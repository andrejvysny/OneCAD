//! Which open document host owns which file path and document identity.
//!
//! Several hosts in one process must never own the same `.onecad` path (two
//! writers would race the container's temp sweep and rename) or the same
//! `DocumentId` (the autosave container, crash marker and recovery decisions are
//! all keyed by it). A [`Claim`] is a lease on one such key: it is taken before the
//! fallible open or save work starts, kept by the host once that work commits, and
//! released by `Drop` — so a failed or cancelled open can never leak ownership.
//!
//! A host counts toward [`MAX_OPEN_DOCUMENTS`] while it holds any document claim,
//! staged or committed. A host may claim a key it already owns again (a reopen or
//! a save onto its own path); claims are counted so each lease releases only itself.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};

use onecad_core::ids::DocumentId;

use crate::error::ApiError;
use crate::process::HostId;

/// The soft cap on simultaneously open documents. Each open document owns one
/// geometry worker process, so this bounds the process count and memory.
pub const MAX_OPEN_DOCUMENTS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Path(PathBuf),
    Document(DocumentId),
}

#[derive(Default)]
pub struct Reservations {
    owners: StdMutex<HashMap<Key, (HostId, usize)>>,
}

/// A lease on one path or document identity, released on drop.
pub struct Claim {
    table: Arc<Reservations>,
    host: HostId,
    key: Key,
    source_alias: bool,
}

impl std::fmt::Debug for Claim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Claim")
            .field("host", &self.host)
            .field("key", &self.key)
            .finish()
    }
}

impl Claim {
    /// Whether this lease is on a file path (rather than a document identity).
    #[must_use]
    pub fn is_path(&self) -> bool {
        matches!(self.key, Key::Path(_))
    }

    /// Keep an imported/original source reserved when the editable target changes.
    pub fn protect_source(mut self) -> Self {
        self.source_alias = self.is_path();
        self
    }

    pub(crate) fn source_path(&self) -> Option<&Path> {
        match &self.key {
            Key::Path(path) if self.source_alias => Some(path),
            _ => None,
        }
    }
}

impl Clone for Claim {
    fn clone(&self) -> Self {
        // This live lease prevents another host from acquiring the key while its
        // operation takes an independent reference, even during host teardown.
        let mut claim = self
            .table
            .lease(&mut self.table.owners(), self.host, self.key.clone());
        claim.source_alias = self.source_alias;
        claim
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut owners = self.table.owners();
        if let Some((host, count)) = owners.get_mut(&self.key) {
            if *host == self.host {
                *count -= 1;
                if *count == 0 {
                    owners.remove(&self.key);
                }
            }
        }
    }
}

impl Reservations {
    /// Leases `path` for `host`. Another host owning it is [`ApiError::AlreadyOpen`],
    /// naming that host so the caller can focus it instead.
    pub fn claim_path(self: &Arc<Self>, host: HostId, path: &Path) -> Result<Claim, ApiError> {
        let key = Key::Path(canonical(path));
        let mut owners = self.owners();
        if let Some((owner, _)) = owners.get(&key).filter(|(owner, _)| *owner != host) {
            return Err(ApiError::AlreadyOpen {
                host: *owner,
                message: format!("{} is already open", path.display()),
            });
        }
        Ok(self.lease(&mut owners, host, key))
    }

    /// Leases `document` for `host`, enforcing the duplicate-identity refusal and
    /// the open-document cap.
    pub fn claim_document(
        self: &Arc<Self>,
        host: HostId,
        document: DocumentId,
    ) -> Result<Claim, ApiError> {
        let key = Key::Document(document);
        let mut owners = self.owners();
        if let Some((owner, _)) = owners.get(&key).filter(|(owner, _)| *owner != host) {
            return Err(ApiError::DuplicateDocument {
                host: *owner,
                message: "a copy of this document is already open; close it first".into(),
            });
        }
        let occupied = occupied_hosts(&owners);
        if !occupied.contains(&host) && occupied.len() >= MAX_OPEN_DOCUMENTS {
            return Err(ApiError::TooManyDocuments(format!(
                "at most {MAX_OPEN_DOCUMENTS} documents can be open; close one first"
            )));
        }
        Ok(self.lease(&mut owners, host, key))
    }

    /// The host owning `path`, if any.
    #[must_use]
    pub fn path_owner(&self, path: &Path) -> Option<HostId> {
        self.owners()
            .get(&Key::Path(canonical(path)))
            .map(|(host, _)| *host)
    }

    /// Number of hosts currently holding a document.
    #[must_use]
    pub fn open_documents(&self) -> usize {
        occupied_hosts(&self.owners()).len()
    }

    fn lease(
        self: &Arc<Self>,
        owners: &mut HashMap<Key, (HostId, usize)>,
        host: HostId,
        key: Key,
    ) -> Claim {
        owners.entry(key.clone()).or_insert((host, 0)).1 += 1;
        Claim {
            table: self.clone(),
            host,
            key,
            source_alias: false,
        }
    }

    /// Poison-tolerant: the table has no invariant a panicking holder could break
    /// halfway, and a poisoned lock must not block every later open and close.
    fn owners(&self) -> MutexGuard<'_, HashMap<Key, (HostId, usize)>> {
        self.owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn occupied_hosts(owners: &HashMap<Key, (HostId, usize)>) -> Vec<HostId> {
    let mut hosts: Vec<HostId> = owners
        .iter()
        .filter(|(key, _)| matches!(key, Key::Document(_)))
        .map(|(_, (host, _))| *host)
        .collect();
    hosts.sort_unstable();
    hosts.dedup();
    hosts
}

/// Resolves existing prefixes before normalizing absent Save As directories.
/// In particular, `missing/../held.onecad` must collide with `held.onecad`
/// before a writer creates `missing`; symlink parents retain filesystem semantics.
pub(crate) fn canonical(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            component => {
                resolved.push(component.as_os_str());
                if let Ok(existing) = resolved.canonicalize() {
                    resolved = existing;
                }
            }
        }
    }
    resolved
}

#[cfg(test)]
#[path = "reservations_tests.rs"]
mod tests;
