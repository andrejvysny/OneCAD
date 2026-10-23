//! Launch-time recovery and the persisted circuit: turning another launch's
//! leftover op records into strikes, the strikes into circuit seeds, and the
//! strikes into dispatch order.

use std::collections::BTreeMap;

use super::{
    now_ms, JobRecord, KernelJournal, Leftover, PoisonRecord, State, IN_PROCESS_POISON_THRESHOLD,
};
use crate::worker::host::{PlanPermit, PoisonSeed};

impl KernelJournal {
    /// Persisted circuit entries for a worker that handshook under
    /// `fingerprint`. The first call in the process also applies the leftover
    /// records of the previous launch (one strike per op record whose
    /// fingerprint matches; everything else is discarded) and drops strikes
    /// keyed under any other fingerprint, which can never match again.
    ///
    /// Order: `poison.json` (strikes + the applied leftover's session) is
    /// written BEFORE the leftover `inflight.json` is replaced, so a kill in
    /// between neither loses the strike nor applies it twice.
    pub(crate) fn seeds(&self, fingerprint: &str) -> Vec<PoisonSeed> {
        let mut state = self.state.lock().unwrap();
        let leftover = state.leftover.take();
        let mut changed = false;
        if let Some(leftover) = &leftover {
            changed |= apply_leftover(&mut state, leftover, fingerprint);
        }
        let suffix = format!("|{fingerprint}");
        let before = state.poison.len();
        state.poison.retain(|key, _| key.ends_with(&suffix));
        changed |= state.poison.len() != before;
        while state.poison.len() > super::MAX_POISON {
            let victim = state
                .poison
                .iter()
                .min_by_key(|(_, entry)| entry.last_ms)
                .map(|(key, _)| key.clone())
                .unwrap();
            state.poison.remove(&victim);
            changed = true;
        }
        if changed {
            self.persist_poison(&state);
        }
        if leftover.is_some() {
            // The previous launch's file is replaced by this launch's state.
            self.persist_inflight(&state);
        }
        state
            .poison
            .iter()
            .map(|(key, record)| seed(key, record))
            .collect()
    }

    /// A prepared plan healed `keys`: their strikes are forgotten (only
    /// written when one was actually persisted).
    pub(crate) fn heal(&self, keys: &[String]) {
        let mut state = self.state.lock().unwrap();
        let before = state.poison.len();
        for key in keys {
            state.poison.remove(key);
        }
        if state.poison.len() != before {
            self.persist_poison(&state);
        }
    }

    /// The user cleared every circuit.
    pub(crate) fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.poison.clear();
        self.persist_poison(&state);
    }

    /// Dispatch order for one plan: exclusive when any of `keys` holds a strike
    /// (it runs alone among this process's plans, so an interruption during it
    /// is attributable), shared otherwise. Held until the plan's terminal.
    pub(crate) async fn plan_permit(&self, keys: &[String]) -> PlanPermit {
        let alone = {
            let state = self.state.lock().unwrap();
            keys.iter()
                .any(|key| state.poison.get(key).is_some_and(|r| r.strikes > 0))
        };
        if alone {
            Box::new(self.order.clone().write_owned().await)
        } else {
            Box::new(self.order.clone().read_owned().await)
        }
    }
}

/// Applies `leftover` once (its session is remembered); `true` if anything
/// in the persisted state changed.
fn apply_leftover(state: &mut State, leftover: &Leftover, fingerprint: &str) -> bool {
    if state.applied_session.as_deref() == Some(leftover.session.as_str()) {
        tracing::info!("kernel journal: leftover already applied (interrupted recovery)");
        return false;
    }
    state.applied_session = Some(leftover.session.clone());
    strike_leftover(state, &leftover.jobs, fingerprint, leftover.complete);
    true
}

/// One strike per distinct leftover OP key under `fingerprint`. With more than
/// one key in flight the crash is ambiguous: the strike is capped below the
/// threshold, so only an unambiguous interruption can open a circuit.
fn strike_leftover(state: &mut State, leftover: &[JobRecord], fingerprint: &str, complete: bool) {
    let keys = op_keys(leftover, fingerprint);
    // Any unfinished job may have caused process loss, including another lane
    // or kernel build. Filtering to plan keys first would invent attribution.
    let ambiguous = !complete || leftover.len() != 1;
    let cap =
        if ambiguous {
            tracing::warn!(ops = keys.len(),
            "kernel journal: several ops were in flight — each is struck but runs alone until it \
             completes; none opens on this evidence");
            IN_PROCESS_POISON_THRESHOLD - 1
        } else {
            u32::MAX
        };
    for (key, op_id) in keys {
        let entry = state.poison.entry(key.clone()).or_insert(PoisonRecord {
            strikes: 0,
            op_id,
            last_ms: 0,
        });
        if entry.strikes < cap {
            entry.strikes += 1;
        }
        entry.last_ms = now_ms();
        tracing::warn!(key = %key, strikes = entry.strikes, ambiguous,
            "kernel journal: the previous launch ended inside this op — suspected interruption");
    }
}

/// The distinct op keys (and their op ids) of `leftover` under `fingerprint`.
fn op_keys(leftover: &[JobRecord], fingerprint: &str) -> BTreeMap<String, String> {
    let mut keys = BTreeMap::new();
    for record in leftover {
        if record.kind != super::JobKind::Kernel
            || !matches!(record.verb.as_str(), "ExecutePlan" | "PreviewPlan")
            || record.instance == 0
            || record.epoch == 0
            || record.request_id == 0
        {
            continue;
        }
        let (Some(key), Some(op_id)) = (&record.key, &record.op_id) else {
            tracing::warn!(verb = %record.verb,
                "kernel journal: the previous launch ended inside a non-plan job — nothing poisoned");
            continue;
        };
        if record.fingerprint != fingerprint
            || op_id.is_empty()
            || record.step_index.is_none()
            || record.op_position.is_none()
            || !key.ends_with(&format!("|{op_id}|{fingerprint}"))
        {
            tracing::info!(key = %key,
                "kernel journal: leftover record from another OCCT build discarded");
            continue;
        }
        keys.insert(key.clone(), op_id.clone());
    }
    keys
}

fn seed(key: &str, record: &PoisonRecord) -> PoisonSeed {
    PoisonSeed {
        key: key.to_string(),
        strikes: record.strikes,
        open: record.strikes >= IN_PROCESS_POISON_THRESHOLD,
        message: format!(
            "suspected in-process kernel interruption in op {} ({} earlier launch(es) ended \
             inside it; kernel journal)",
            record.op_id, record.strikes
        ),
    }
}
