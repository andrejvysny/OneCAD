//! Launch-time recovery and the persisted circuit: turning another launch's
//! leftover op records into strikes, and the strikes into circuit seeds.

use super::{now_ms, JobRecord, KernelJournal, PoisonRecord, State, IN_PROCESS_POISON_THRESHOLD};
use crate::worker::host::PoisonSeed;

impl KernelJournal {
    /// Persisted circuit entries for a worker that handshook under
    /// `fingerprint`. The first call in the process also applies the leftover
    /// records of the previous launch (one strike per op record whose
    /// fingerprint matches; everything else is discarded) and drops strikes
    /// keyed under any other fingerprint, which can never match again.
    pub(crate) fn seeds(&self, fingerprint: &str) -> Vec<PoisonSeed> {
        let mut state = self.state.lock().unwrap();
        let mut changed = false;
        if let Some(leftover) = state.leftover.take() {
            changed |= strike_leftover(&mut state, &leftover, fingerprint);
            // The previous launch's file is replaced by this launch's state.
            self.persist_inflight(&state);
        }
        let suffix = format!("|{fingerprint}");
        let before = state.poison.len();
        state.poison.retain(|key, _| key.ends_with(&suffix));
        changed |= state.poison.len() != before;
        if changed {
            self.persist_poison(&state);
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
}

/// One strike per leftover OP record under `fingerprint`; `true` if any.
fn strike_leftover(state: &mut State, leftover: &[JobRecord], fingerprint: &str) -> bool {
    let mut struck = false;
    for record in leftover {
        let (Some(key), Some(op_id)) = (&record.key, &record.op_id) else {
            tracing::warn!(verb = %record.verb,
                "kernel journal: the previous launch ended inside a non-plan job — nothing poisoned");
            continue;
        };
        if record.fingerprint != fingerprint {
            tracing::info!(key = %key,
                "kernel journal: leftover record from another OCCT build discarded");
            continue;
        }
        let entry = state.poison.entry(key.clone()).or_insert(PoisonRecord {
            strikes: 0,
            op_id: op_id.clone(),
            last_ms: 0,
        });
        entry.strikes += 1;
        entry.last_ms = now_ms();
        tracing::warn!(key = %key, strikes = entry.strikes,
            "kernel journal: the previous launch ended inside this op — suspected interruption");
        struck = true;
    }
    struck
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
