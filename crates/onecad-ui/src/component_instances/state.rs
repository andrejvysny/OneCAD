//! Per-document instance store: the listing, the open review and edit, request sequencing.
//!
//! Listing and planning are latest-only per kind; a mutation is never superseded (its
//! reply is the only report that history changed), so one runs at a time. Replies carry
//! the document scope they were asked in and are dropped for any other.
use super::{
    InstanceView, Mutated, Op, Reply, Request, Scope,
    model::{draft_for, upgradable},
    review::{Review, STALE},
};
use crate::library_catalog::draft::ParamDraft;

/// What the listing was last requested for; any change asks again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    scope: Scope,
    revision: u64,
    generation: u64,
}

#[derive(Clone, Debug)]
struct Inflight {
    sequence: u64,
    scope: Scope,
}

#[derive(Clone, Debug)]
struct Pending {
    sequence: u64,
    scope: Scope,
    what: &'static str,
}

/// The inspector's edit of one instance.
#[derive(Clone, Debug)]
pub(crate) struct Edit {
    pub record_id: String,
    pub draft: ParamDraft,
    /// `State::listed` when the draft was seeded.
    pub seeded_at: u64,
    /// The last map committed from this draft; a refused commit is not resent unchanged.
    /// The effective values the draft was seeded from; typing is measured against them.
    pub baseline:
        std::collections::BTreeMap<String, onecad_core::document::record::ComponentParamValue>,
    pub attempted: Option<super::model::Commit>,
}

/// What a reply meant for the host, beyond the store's own bookkeeping.
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    Dropped,
    Listed,
    Planned,
    Mutated(Mutated),
    Failed { what: &'static str, error: String },
    ListFailed(String),
    PlanFailed(String),
}

#[derive(Debug, Default)]
pub(crate) struct State {
    next: u64,
    outbox: Vec<Request>,
    list: Option<Inflight>,
    plan: Option<Inflight>,
    mutation: Option<Pending>,
    stamp: Option<Stamp>,
    items: Vec<InstanceView>,
    listed: u64,
    list_error: Option<String>,
    pub edit: Option<Edit>,
    pub confirm_detach: Option<String>,
    pub review: Option<Review>,
}

impl State {
    pub(crate) fn items(&self) -> &[InstanceView] {
        &self.items
    }
    pub(crate) fn find(&self, record_id: &str) -> Option<&InstanceView> {
        self.items.iter().find(|v| v.dto.record_id == record_id)
    }
    pub(crate) fn list_error(&self) -> Option<&str> {
        self.list_error.as_deref()
    }
    pub(crate) fn take_outbox(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
    }
    /// A mutation is in flight: history is about to change and edits must wait.
    pub(crate) fn blocks(&self) -> bool {
        self.mutation.is_some()
    }
    /// The listing describes `revision` and no refresh is in flight, so edits compare
    /// against current values.
    pub(crate) fn settled(&self, revision: u64) -> bool {
        self.list.is_none() && self.stamp.as_ref().is_some_and(|s| s.revision == revision)
    }
    pub(crate) fn upgradable(&self) -> Vec<String> {
        upgradable(&self.items)
    }

    fn issue(&mut self, scope: &Scope, op: Op) -> u64 {
        self.next += 1;
        self.outbox.push(Request {
            sequence: self.next,
            scope: scope.clone(),
            op,
        });
        self.next
    }

    /// Asks for a fresh listing when the document, its revision or the library changed.
    pub(crate) fn sync(&mut self, scope: &Scope, revision: u64, generation: u64) {
        let stamp = Stamp {
            scope: scope.clone(),
            revision,
            generation,
        };
        if self.stamp.as_ref() == Some(&stamp) {
            return;
        }
        if self.stamp.as_ref().is_some_and(|s| s.scope != *scope) {
            self.forget();
        }
        self.stamp = Some(stamp);
        self.outbox.retain(|r| !matches!(r.op, Op::List));
        let sequence = self.issue(scope, Op::List);
        self.list = Some(Inflight {
            sequence,
            scope: scope.clone(),
        });
    }

    /// Another document (or none): nothing here describes it.
    pub(crate) fn forget(&mut self) {
        *self = Self {
            next: self.next,
            ..Self::default()
        };
    }

    pub(crate) fn request_plan(&mut self, scope: &Scope, record_ids: Vec<String>) {
        self.outbox.retain(|r| !matches!(r.op, Op::Plan { .. }));
        let sequence = self.issue(scope, Op::Plan { record_ids });
        self.plan = Some(Inflight {
            sequence,
            scope: scope.clone(),
        });
    }

    /// Opens the review for every instance with an update.
    pub(crate) fn open_review(&mut self, scope: &Scope) {
        let ids = self.upgradable();
        if ids.is_empty() {
            return;
        }
        self.review = Some(Review::opening(ids.clone()));
        self.request_plan(scope, ids);
    }

    /// Queues a mutation unless one is running; returns whether it was queued.
    pub(crate) fn mutate(&mut self, scope: &Scope, what: &'static str, op: Op) -> bool {
        if self.mutation.is_some() {
            return false;
        }
        let sequence = self.issue(scope, op);
        self.mutation = Some(Pending {
            sequence,
            scope: scope.clone(),
            what,
        });
        true
    }

    /// A request the bridge refused never gets a reply: settle it as a failure.
    pub(crate) fn refuse(&mut self, current: &Scope, sequence: u64, error: String) -> Outcome {
        self.accept(current, sequence, Err(error))
    }

    pub(crate) fn accept(
        &mut self,
        current: &Scope,
        sequence: u64,
        result: Result<Reply, String>,
    ) -> Outcome {
        if let Some(pending) = self.mutation.as_ref().filter(|m| m.sequence == sequence) {
            let (scope, what) = (pending.scope.clone(), pending.what);
            self.mutation = None;
            return if scope != *current {
                Outcome::Dropped
            } else {
                self.mutated(current, what, result)
            };
        }
        if self.list.as_ref().is_some_and(|l| l.sequence == sequence) {
            let scope = self.list.take().map(|l| l.scope);
            return match scope {
                Some(scope) if scope == *current => self.listed_reply(result),
                _ => Outcome::Dropped,
            };
        }
        if self.plan.as_ref().is_some_and(|p| p.sequence == sequence) {
            let scope = self.plan.take().map(|p| p.scope);
            return match scope {
                Some(scope) if scope == *current => self.planned_reply(result),
                _ => Outcome::Dropped,
            };
        }
        Outcome::Dropped
    }

    fn listed_reply(&mut self, result: Result<Reply, String>) -> Outcome {
        match result {
            Ok(Reply::Listed(items)) => {
                self.items = items;
                self.listed += 1;
                self.list_error = None;
                self.reseed_edit();
                Outcome::Listed
            }
            Ok(_) => self.list_failed("unexpected component reply".into()),
            Err(error) => self.list_failed(error),
        }
    }

    fn list_failed(&mut self, error: String) -> Outcome {
        self.list_error = Some(error.clone());
        Outcome::ListFailed(error)
    }

    fn planned_reply(&mut self, result: Result<Reply, String>) -> Outcome {
        let Some(review) = self.review.as_mut() else {
            return Outcome::Dropped;
        };
        match result {
            Ok(Reply::Planned(items)) => {
                review.planned(items);
                Outcome::Planned
            }
            Ok(_) => plan_failed(review, "unexpected component reply".into()),
            Err(error) => plan_failed(review, error),
        }
    }

    fn mutated(
        &mut self,
        scope: &Scope,
        what: &'static str,
        result: Result<Reply, String>,
    ) -> Outcome {
        match result {
            Ok(Reply::Mutated(done)) => {
                if self.review.is_some() && done.failure.is_none() {
                    self.review = None;
                }
                Outcome::Mutated(*done)
            }
            Ok(_) => Outcome::Failed {
                what,
                error: "unexpected component reply".into(),
            },
            Err(error) if error.contains(STALE) && self.review.is_some() => {
                // The approved plan no longer matches the document: plan again.
                let ids = self.review.as_mut().map(|review| {
                    review.stale();
                    review.record_ids.clone()
                });
                if let Some(ids) = ids {
                    self.request_plan(scope, ids);
                }
                Outcome::Failed { what, error }
            }
            Err(error) => {
                if let Some(review) = self.review.as_mut() {
                    review.error = Some(error.clone());
                }
                Outcome::Failed { what, error }
            }
        }
    }

    /// Rebuilds the open edit's draft from the new listing unless the user has typed.
    fn reseed_edit(&mut self) {
        let Some(edit) = self.edit.as_mut() else {
            return;
        };
        let Some(view) = self
            .items
            .iter()
            .find(|v| v.dto.record_id == edit.record_id)
        else {
            self.edit = None;
            return;
        };
        if !super::model::dirty(&edit.draft, &edit.baseline) {
            if let Some(draft) = draft_for(view) {
                edit.draft = draft;
                edit.seeded_at = self.listed;
                edit.baseline = view.dto.effective.clone();
                edit.attempted = None;
            } else {
                self.edit = None;
            }
        }
    }

    /// The draft for `record_id`, seeding it when the selection moved.
    pub(crate) fn edit_for(&mut self, record_id: &str) -> Option<&mut Edit> {
        if self.edit.as_ref().is_none_or(|e| e.record_id != record_id) {
            let view = self.items.iter().find(|v| v.dto.record_id == record_id)?;
            let draft = draft_for(view)?;
            self.edit = Some(Edit {
                record_id: record_id.to_owned(),
                draft,
                seeded_at: self.listed,
                baseline: view.dto.effective.clone(),
                attempted: None,
            });
        }
        self.edit.as_mut()
    }
}

fn plan_failed(review: &mut Review, error: String) -> Outcome {
    review.loading = false;
    review.error = Some(error.clone());
    Outcome::PlanFailed(error)
}
