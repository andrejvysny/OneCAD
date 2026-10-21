//! A document host's leases on its open path and document identity.
//!
//! Every lane that installs a document (new, open, import, recover) leases the
//! keys it is about to own before publishing, and swaps them in with
//! [`AppState::adopt_claims`] once the runtime is published. A failed open drops
//! its staged leases, which releases them. See [`crate::reservations`].

use std::path::Path;
use std::sync::atomic::Ordering;

use onecad_core::ids::DocumentId;

use crate::error::ApiError;
use crate::reservations::Claim;
use crate::state::AppState;

impl AppState {
    /// Leases `path` for this host (open source or save target).
    pub fn claim_path(&self, path: &Path) -> Result<Claim, ApiError> {
        self.services()
            .reservations()
            .claim_path(self.host_id(), path)
    }

    /// Leases `document` for this host, enforcing the duplicate and cap refusals.
    pub fn claim_document(&self, document: DocumentId) -> Result<Claim, ApiError> {
        self.services()
            .reservations()
            .claim_document(self.host_id(), document)
    }

    /// Fails fast when this host could not take another document, before an open
    /// spends a worker on it. The authoritative check is [`claim_document`].
    ///
    /// [`claim_document`]: Self::claim_document
    pub fn ensure_document_capacity(&self) -> Result<(), ApiError> {
        if self.tasks.closing.load(Ordering::Acquire) {
            return Err(ApiError::InvalidCommand(
                "this document is closing; open the file in another tab".into(),
            ));
        }
        let probe = DocumentId::new();
        self.claim_document(probe).map(drop)
    }

    /// Replaces every lease this host holds with `claims` — the commit point of an
    /// install. The previous document's leases are released here.
    pub fn adopt_claims(&self, claims: Vec<Claim>) {
        let previous = std::mem::replace(&mut *self.claims_guard(), claims);
        drop(previous);
    }

    /// Adds a lease without releasing any held one (an alias of the open file).
    pub fn add_claim(&self, claim: Claim) {
        self.claims_guard().push(claim);
    }

    /// Replaces this host's path lease after a Save As committed to a new path.
    pub fn adopt_path_claim(&self, claim: Claim) {
        let mut claims = self.claims_guard();
        claims.retain(|held| !held.is_path());
        claims.push(claim);
    }

    /// Closes admission: every later install on this host is refused.
    pub(crate) fn begin_closing(&self) {
        self.tasks.closing.store(true, Ordering::Release);
    }

    /// Reopens admission after a refused close.
    pub(crate) fn cancel_closing(&self) {
        self.tasks.closing.store(false, Ordering::Release);
    }

    /// Releases every lease — the end of a host's teardown.
    pub fn release_claims(&self) {
        let previous = std::mem::take(&mut *self.claims_guard());
        drop(previous);
    }

    fn claims_guard(&self) -> std::sync::MutexGuard<'_, Vec<Claim>> {
        self.claims
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
