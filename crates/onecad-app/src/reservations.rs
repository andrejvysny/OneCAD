//! Which open document host owns which file path and document identity.
//!
//! Several hosts in one process must never own the same `.onecad` path (two
//! writers could replace each other's completed saves) or the same
//! `DocumentId` (the autosave container, crash marker and recovery decisions are
//! all keyed by it). A [`Claim`] leases all keys for one path or document: it is taken before the
//! fallible open or save work starts, kept by the host once that work commits, and
//! released by `Drop` — so a failed or cancelled open can never leak ownership.
//!
//! A host counts toward [`MAX_OPEN_DOCUMENTS`] while it holds any document claim,
//! staged or committed. A host may claim a key it already owns again (a reopen or
//! a save onto its own path); claims are counted so each lease releases only itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

/// A lease on a path entry and any referent, or one document identity.
pub struct Claim {
    table: Arc<Reservations>,
    host: HostId,
    keys: Vec<Key>,
}

impl std::fmt::Debug for Claim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Claim")
            .field("host", &self.host)
            .field("keys", &self.keys)
            .finish()
    }
}

impl Claim {
    /// Whether this lease is on a file path (rather than a document identity).
    #[must_use]
    pub fn is_path(&self) -> bool {
        matches!(self.keys.first(), Some(Key::Path(_)))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut owners = self.table.owners();
        for key in &self.keys {
            if let Some((host, count)) = owners.get_mut(key) {
                if *host == self.host {
                    *count -= 1;
                    if *count == 0 {
                        owners.remove(key);
                    }
                }
            }
        }
    }
}

impl Reservations {
    /// Leases `path` for `host`. Another host owning it is [`ApiError::AlreadyOpen`],
    /// naming that host so the caller can focus it instead.
    pub fn claim_path(self: &Arc<Self>, host: HostId, path: &Path) -> Result<Claim, ApiError> {
        let keys = path_keys(path)?;
        let mut owners = self.owners();
        for key in &keys {
            if let Some((owner, _)) = owners.get(key).filter(|(owner, _)| *owner != host) {
                return Err(ApiError::AlreadyOpen {
                    host: *owner,
                    message: format!("{} is already open", path.display()),
                });
            }
        }
        Ok(self.lease(&mut owners, host, keys))
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
        Ok(self.lease(&mut owners, host, vec![key]))
    }

    /// The host owning `path`, if any.
    #[must_use]
    pub fn path_owner(&self, path: &Path) -> Option<HostId> {
        let keys = path_keys(path).ok()?;
        let owners = self.owners();
        keys.iter()
            .find_map(|key| owners.get(key).map(|(host, _)| *host))
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
        keys: Vec<Key>,
    ) -> Claim {
        for key in &keys {
            owners.entry(key.clone()).or_insert((host, 0)).1 += 1;
        }
        Claim {
            table: self.clone(),
            host,
            keys,
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

/// Own both the directory entry and any existing resolved referent. The entry
/// remains stable when atomic replacement replaces a final-component symlink.
fn path_keys(path: &Path) -> Result<Vec<Key>, ApiError> {
    let name = path
        .file_name()
        .ok_or_else(|| ApiError::Io("save path has no file name".into()))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent.canonicalize().map_err(|error| {
        ApiError::Io(format!(
            "cannot reserve {}: parent directory is unavailable: {error}",
            path.display()
        ))
    })?;
    let entry = Key::Path(parent.join(name));
    let mut keys = vec![entry];
    if let Ok(referent) = path.canonicalize() {
        let referent = Key::Path(referent);
        if !keys.contains(&referent) {
            keys.push(referent);
        }
    }
    Ok(keys)
}

#[cfg(test)]
#[path = "reservations_tests.rs"]
mod tests;
