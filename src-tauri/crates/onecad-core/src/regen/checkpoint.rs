//! Regen checkpoints and their versioned envelopes (SCHEMA §7.7; V1/V2 §4.2).
//!
//! A checkpoint is an **atomic artifact set** for a step (BREP blobs via
//! BinTools, the ElementMap partition JSON, the three signatures and the
//! `historyPrefixHash`), each wrapped in a Rust-readable [`CheckpointEnvelope`].
//! The core stores the **opaque bytes plus the parsed envelope metadata** (SCHEMA
//! §7.7: "core stores bytes + parsed envelope metadata").
//!
//! **Checkpoints are disposable caches (Invariant 7).** An envelope whose
//! versions/fingerprint are incompatible, or whose stored history-prefix hash no
//! longer matches the timeline, is discarded — the planner falls back to an
//! earlier checkpoint or replays from empty. A checkpoint never blocks opening
//! the authoritative JSON, and **correctness never depends on the cache**.

use std::collections::BTreeMap;

use crate::document::body::BodyRegistry;
use crate::document::element_index::ElementIndex;
use crate::ids::{BodyId, SnapshotId};

use super::engine::StepSignatures;
use super::planner::{HistoryPrefixHash, PlanContext};

/// The current checkpoint artifact-schema version (SCHEMA §7.7
/// `artifactSchemaVersion`).
pub const ARTIFACT_SCHEMA_VERSION: u32 = 1;

/// A checkpoint id (SCHEMA §7.7 `checkpointId`, e.g. `"ckpt_9"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CheckpointId(pub String);

impl CheckpointId {
    /// Wraps a checkpoint id string.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The raw id string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A reference to a checkpoint used as a plan's base (SCHEMA §7.2
/// `baseCheckpoint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRef {
    pub step_index: usize,
    pub checkpoint_id: CheckpointId,
}

/// A checkpoint artifact envelope (SCHEMA §7.7). Per-artifact (per body), but the
/// **version axes and fingerprint are shared** across a checkpoint's artifacts,
/// so a single representative envelope drives compatibility in [`CheckpointMeta`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointEnvelope {
    pub artifact_schema_version: u32,
    /// The body this artifact serializes.
    pub body: BodyId,
    pub step: usize,
    /// History-prefix hash of the checkpoint's base state.
    pub history_prefix_hash: HistoryPrefixHash,
    pub brep_content_hash: String,
    /// OCCT fingerprint (governs BREP/checkpoint compatibility).
    pub occt_fingerprint: String,
    pub descriptor_version: u32,
    pub resolver_version: u32,
    pub quantization_version: u32,
    pub signature_version: u32,
    /// The codec that produced the bytes (e.g. `"brep-bintools"`).
    pub codec: String,
    pub size: u64,
    /// Content hash of the bytes (integrity).
    pub content_hash: String,
}

impl CheckpointEnvelope {
    /// Whether this envelope is compatible with the current policy/fingerprint
    /// (SCHEMA §7.7 / §13). An incompatible envelope is discarded — the cache
    /// degrades to replay, never to a wrong result (Invariant 7).
    #[must_use]
    pub fn is_compatible(&self, ctx: &PlanContext) -> bool {
        self.artifact_schema_version == ARTIFACT_SCHEMA_VERSION
            && self.occt_fingerprint == ctx.occt_fingerprint
            && self.descriptor_version == ctx.policy_versions.descriptor
            && self.resolver_version == ctx.policy_versions.resolver
            && self.quantization_version == ctx.policy_versions.quantization
            && self.signature_version == ctx.policy_versions.signature
    }
}

/// One stored artifact: the parsed envelope + the opaque bytes (SCHEMA §7.7 —
/// "core stores bytes + parsed envelope metadata").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointArtifact {
    pub envelope: CheckpointEnvelope,
    /// The opaque codec bytes (BREP blob, etc.). The core never interprets them.
    pub bytes: Vec<u8>,
}

/// The atomic artifact set emitted by `SaveCheckpoint` (SCHEMA §7.7).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointArtifacts {
    pub step: usize,
    /// Per-body artifacts (BREP + envelope).
    pub artifacts: Vec<CheckpointArtifact>,
    /// The ElementMap partition JSON bytes (SCHEMA §7.7 `elementMapPartition`).
    pub element_map_partition: Vec<u8>,
    /// The three step signatures at this checkpoint.
    pub signatures: StepSignatures,
    /// History-prefix hash of the checkpoint's base state.
    pub history_prefix_hash: HistoryPrefixHash,
}

impl CheckpointArtifacts {
    /// A representative envelope for compatibility checks (the first artifact's).
    /// All of a checkpoint's artifacts share version axes + fingerprint, so any
    /// one is representative. `None` for a checkpoint with no BREP artifacts.
    #[must_use]
    pub fn representative_envelope(&self) -> Option<&CheckpointEnvelope> {
        self.artifacts.first().map(|a| &a.envelope)
    }
}

/// Cache-level metadata for a stored checkpoint (SCHEMA §7.7 — what the planner
/// browses to pick a base). Carries the representative envelope for compatibility
/// and the stored history-prefix hash for staleness detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointMeta {
    pub id: CheckpointId,
    pub step: usize,
    pub history_prefix_hash: HistoryPrefixHash,
    pub envelope: CheckpointEnvelope,
}

/// Which signature drifted on restore (SCHEMA §7.7 `driftDetail.signature`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftSignature {
    Geometry,
    BodyLifecycle,
    ReferencedBinding,
}

/// Restore drift detail (SCHEMA §7.7 `driftDetail`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftDetail {
    pub signature: DriftSignature,
    pub expected: String,
    pub actual: String,
}

/// `RestoreCheckpoint` result (SCHEMA §7.7).
///
/// Carries the **reconstructed base state** so the executor seeds its scratch
/// from the checkpoint artifacts, not from live session state (review F3): the
/// worker restores the BREP and the ElementMap partition, then returns the body
/// registry and element index they represent. The `base_registry` lifecycle log
/// ends at `checkpoint_step`, so folding the plan's remaining steps onto it
/// appends (never duplicates — the body.rs clear-before-replay contract). A
/// `restored == false` or `drift_detected == true` means the checkpoint is
/// unusable; the executor then falls back to replay-from-0 (review F12).
// No `Eq`: `base_elements` carries the opaque `serde_json::Value` descriptor an
// entry needs to be re-bound after a reopen (DI-4), and that is `PartialEq` only.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreResult {
    pub restored: bool,
    pub snapshot_id: SnapshotId,
    pub drift_detected: bool,
    pub drift_detail: Option<DriftDetail>,
    /// The step the checkpoint represents (its base covers steps `0..=step`).
    pub checkpoint_step: usize,
    /// The restored body registry (log truncated to `checkpoint_step`).
    pub base_registry: BodyRegistry,
    /// The restored element partition index at the checkpoint.
    pub base_elements: ElementIndex,
}

impl RestoreResult {
    /// A successful, drift-free restore of `step` carrying the reconstructed base
    /// registry + element index.
    #[must_use]
    pub fn ok(
        snapshot_id: SnapshotId,
        step: usize,
        base_registry: BodyRegistry,
        base_elements: ElementIndex,
    ) -> Self {
        Self {
            restored: true,
            snapshot_id,
            drift_detected: false,
            drift_detail: None,
            checkpoint_step: step,
            base_registry,
            base_elements,
        }
    }
}

/// A checkpoint loaded from the store (its full artifact set + meta).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCheckpoint {
    pub meta: CheckpointMeta,
    pub artifacts: CheckpointArtifacts,
}

// ─────────────────────────────────────────────────────────────────────────────
// Store
// ─────────────────────────────────────────────────────────────────────────────

/// A checkpoint cache. Implementations are disposable stores keyed by step; the
/// planner reads [`list`](CheckpointStore::list) to pick an accelerated base.
pub trait CheckpointStore {
    /// All stored checkpoint metadata (order unspecified; the planner picks by
    /// step + compatibility).
    fn list(&self) -> Vec<CheckpointMeta>;

    /// Stores a checkpoint's artifacts at `step`, returning its minted id. A
    /// later save at the same step supersedes the earlier one. Implementations MAY
    /// prune older entries to stay bounded (a checkpoint is disposable).
    fn save(&mut self, step: usize, artifacts: CheckpointArtifacts) -> CheckpointId;

    /// Loads the checkpoint stored at `step`, if any.
    fn load(&self, step: usize) -> Option<StoredCheckpoint>;

    /// Drops every checkpoint at or above `step` — the **truncation** eviction. A
    /// timeline mutation at `step` makes those checkpoints' stored prefix hashes
    /// stale by definition; without this they linger as orphans the planner rejects
    /// on every plan and the cache only ever grows. The edit layer calls it (only the
    /// edit layer knows which step moved).
    fn invalidate_from(&mut self, step: usize);
}

/// How many checkpoints [`InMemoryCheckpointStore`] retains (the ladder width).
const LADDER_WIDTH: usize = 5;

/// A bounded in-memory [`CheckpointStore`] — the vertical-slice default. With **no
/// checkpoints saved, plans replay from 0** (the correct, cache-free baseline).
///
/// Retention is a **geometric ladder** anchored at the highest stored step `head`:
/// `head`, `head−1`, and the nearest stored steps at or below `head/2`, `head/4` and
/// `0`. That keeps re-editing the newest feature cheap while still holding a coarse
/// fallback deep in the timeline, in a fixed [`LADDER_WIDTH`] slots — so a long
/// editing session's memory is flat, not linear in the number of saves.
#[derive(Debug, Default)]
pub struct InMemoryCheckpointStore {
    by_step: BTreeMap<usize, StoredCheckpoint>,
    next_id: u64,
}

impl InMemoryCheckpointStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Prunes to the retention ladder. Anchored at the highest stored step, **never**
    /// at the step just saved: a rollback-save at cursor 4 must not evict a still-valid
    /// step-10 checkpoint (the timeline above 4 is untouched — only
    /// [`invalidate_from`](CheckpointStore::invalidate_from) may drop it).
    fn prune(&mut self) {
        let Some(&head) = self.by_step.keys().next_back() else {
            return;
        };
        let mut keep: Vec<usize> = vec![head, head.saturating_sub(1)];
        for target in [head / 2, head / 4, 0] {
            if let Some(&s) = self.by_step.range(..=target).next_back().map(|(s, _)| s) {
                keep.push(s);
            }
        }
        keep.sort_unstable();
        keep.dedup();
        debug_assert!(keep.len() <= LADDER_WIDTH);
        self.by_step.retain(|s, _| keep.contains(s));
    }
}

impl CheckpointStore for InMemoryCheckpointStore {
    fn list(&self) -> Vec<CheckpointMeta> {
        self.by_step.values().map(|c| c.meta.clone()).collect()
    }

    fn save(&mut self, step: usize, artifacts: CheckpointArtifacts) -> CheckpointId {
        let id = CheckpointId::new(format!("ckpt_{}", self.next_id));
        self.next_id += 1;
        // Representative envelope for the meta (all artifacts share version axes);
        // synthesize a minimal one when a checkpoint carries no BREP artifacts.
        let envelope = artifacts
            .representative_envelope()
            .cloned()
            .unwrap_or_else(|| CheckpointEnvelope {
                artifact_schema_version: ARTIFACT_SCHEMA_VERSION,
                body: BodyId(uuid::Uuid::nil()),
                step,
                history_prefix_hash: artifacts.history_prefix_hash.clone(),
                brep_content_hash: String::new(),
                occt_fingerprint: String::new(),
                descriptor_version: 1,
                resolver_version: 1,
                quantization_version: 1,
                signature_version: 1,
                codec: String::new(),
                size: 0,
                content_hash: String::new(),
            });
        let meta = CheckpointMeta {
            id: id.clone(),
            step,
            history_prefix_hash: artifacts.history_prefix_hash.clone(),
            envelope,
        };
        self.by_step
            .insert(step, StoredCheckpoint { meta, artifacts });
        self.prune();
        id
    }

    fn load(&self, step: usize) -> Option<StoredCheckpoint> {
        self.by_step.get(&step).cloned()
    }

    fn invalidate_from(&mut self, step: usize) {
        self.by_step.split_off(&step);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regen::engine::{Signature, StepSignatures};
    use crate::regen::planner::PlanContext;

    fn sigs() -> StepSignatures {
        StepSignatures {
            geometry: Signature::new("g"),
            body_lifecycle: Signature::new("b"),
            referenced_binding: Signature::new("r"),
        }
    }

    fn envelope(fp: &str, desc: u32) -> CheckpointEnvelope {
        CheckpointEnvelope {
            artifact_schema_version: ARTIFACT_SCHEMA_VERSION,
            body: BodyId(uuid::Uuid::from_u128(1)),
            step: 2,
            history_prefix_hash: HistoryPrefixHash::new("abc"),
            brep_content_hash: "aa".into(),
            occt_fingerprint: fp.into(),
            descriptor_version: desc,
            resolver_version: 1,
            quantization_version: 1,
            signature_version: 1,
            codec: "brep-bintools".into(),
            size: 10,
            content_hash: "bb".into(),
        }
    }

    fn ctx() -> PlanContext {
        PlanContext {
            policy_versions: Default::default(),
            occt_fingerprint: "fp".into(),
        }
    }

    #[test]
    fn compatible_matches_versions_and_fingerprint() {
        assert!(envelope("fp", 1).is_compatible(&ctx()));
        assert!(
            !envelope("other", 1).is_compatible(&ctx()),
            "fingerprint mismatch"
        );
        assert!(
            !envelope("fp", 2).is_compatible(&ctx()),
            "descriptor version mismatch"
        );
    }

    #[test]
    fn store_save_list_load_round_trip() {
        let mut store = InMemoryCheckpointStore::new();
        let artifacts = CheckpointArtifacts {
            step: 2,
            artifacts: vec![CheckpointArtifact {
                envelope: envelope("fp", 1),
                bytes: vec![1, 2, 3],
            }],
            element_map_partition: vec![],
            signatures: sigs(),
            history_prefix_hash: HistoryPrefixHash::new("abc"),
        };
        let id = store.save(2, artifacts);
        let metas = store.list();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, id);
        assert_eq!(metas[0].step, 2);
        let loaded = store.load(2).unwrap();
        assert_eq!(loaded.artifacts.artifacts[0].bytes, vec![1, 2, 3]);
        assert!(store.load(5).is_none());
    }

    fn artifacts_at(step: usize) -> CheckpointArtifacts {
        let mut env = envelope("fp", 1);
        env.step = step;
        CheckpointArtifacts {
            step,
            artifacts: vec![CheckpointArtifact {
                envelope: env,
                bytes: vec![step as u8],
            }],
            element_map_partition: vec![],
            signatures: sigs(),
            history_prefix_hash: HistoryPrefixHash::new(format!("h{step}")),
        }
    }

    fn steps(store: &InMemoryCheckpointStore) -> Vec<usize> {
        let mut s: Vec<usize> = store.list().iter().map(|m| m.step).collect();
        s.sort_unstable();
        s
    }

    /// Saving every step keeps the ladder at ≤5 slots, always anchored on the head +
    /// its predecessor, with coarse fallbacks near head/2, head/4 and the floor.
    ///
    /// The mid-rungs **thin out** as the head advances: pruning is greedy, so a step
    /// dropped at head `h` cannot come back to serve `h'/2` later, and a
    /// save-every-step session settles on `{0, 2, head−1, head}`. Bounded memory is
    /// the contract; the deep rungs are best-effort acceleration (Invariant 7 — a
    /// missing checkpoint only ever costs a replay). Pinned here so a future ladder
    /// change is a deliberate one.
    #[test]
    fn ladder_stays_bounded_and_keeps_head_and_floor() {
        let mut store = InMemoryCheckpointStore::new();
        for step in 0..=32 {
            store.save(step, artifacts_at(step));
            let got = steps(&store);
            assert!(
                got.len() <= LADDER_WIDTH,
                "head {step}: ladder must stay ≤{LADDER_WIDTH}, got {got:?}"
            );
            assert_eq!(*got.last().unwrap(), step, "head {step} is always retained");
            assert_eq!(got[0], 0, "the floor checkpoint is always retained");
            if step >= 1 {
                assert!(
                    got.contains(&(step - 1)),
                    "head {step}: the predecessor is retained, got {got:?}"
                );
            }
        }
        assert_eq!(
            steps(&store),
            vec![0, 2, 31, 32],
            "the settled save-every-step ladder at head 32"
        );
    }

    /// The degenerate heads: nothing panics and nothing is over-evicted.
    #[test]
    fn ladder_saturates_at_head_zero_and_one() {
        let mut store = InMemoryCheckpointStore::new();
        store.save(0, artifacts_at(0));
        assert_eq!(steps(&store), vec![0]);
        store.save(1, artifacts_at(1));
        assert_eq!(steps(&store), vec![0, 1]);
    }

    /// Review M7: pruning is anchored at the **highest stored step**, never at the step
    /// just written. A rollback-save low in the timeline must not destroy a still-valid
    /// checkpoint above it — only `invalidate_from` may do that.
    #[test]
    fn rollback_save_keeps_the_later_checkpoint() {
        let mut store = InMemoryCheckpointStore::new();
        store.save(0, artifacts_at(0));
        store.save(9, artifacts_at(9));
        store.save(10, artifacts_at(10));
        assert_eq!(steps(&store), vec![0, 9, 10]);
        store.save(4, artifacts_at(4));
        assert_eq!(
            steps(&store),
            vec![0, 4, 9, 10],
            "the step-10 head survives a save at step 4"
        );
    }

    #[test]
    fn invalidate_from_evicts_at_and_above_the_step() {
        let mut store = InMemoryCheckpointStore::new();
        for step in 0..=6 {
            store.save(step, artifacts_at(step));
        }
        assert_eq!(steps(&store), vec![0, 1, 2, 5, 6]);
        store.invalidate_from(2);
        assert_eq!(steps(&store), vec![0, 1], "2, 5, 6 evicted (>= 2)");
        assert!(store.load(2).is_none());
        store.invalidate_from(0);
        assert!(
            steps(&store).is_empty(),
            "invalidate_from(0) clears the cache"
        );
    }
}
