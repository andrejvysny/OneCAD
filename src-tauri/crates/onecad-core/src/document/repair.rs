//! Topological-naming repair state (V1/V2 §3.7 + SCHEMA §9).
//!
//! `NeedsRepair` is a first-class **state**, never an `Err`: a low-confidence or
//! ambiguous (tie) rebind surfaces here rather than silently binding to the
//! wrong element (SCHEMA §9: a false positive is strictly worse than a false
//! negative). Repair = Rust rewrites the `OperationRecord` reference and
//! re-regens — there is no worker `BindRepair` verb.
//!
//! [`RepairState`] stores, **per step** (V1/V2 §3.7): the unresolved refs, their
//! candidate lists + scores, which ladder level failed, and UI-friendly labels.
//! The payload mirrors the SCHEMA §9 `needsRepair` wire shape so a worker
//! `planStep.needsRepair[]` entry maps 1:1 into a [`RepairItem`].

use serde::{Deserialize, Serialize};

use crate::document::refs::{AnchorIntent, Extra};
use crate::ids::ElementId;
use crate::math::Vec3;

/// Which ladder level failed to decide (SCHEMA §9 `ladderFailed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LadderLevel {
    /// OCCT history gave no / an ambiguous mapping.
    History,
    /// Descriptor + anchor matching was ambiguous / low-confidence.
    Descriptor,
}

/// Why the ladder could not confidently bind (SCHEMA §9 `reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepairReason {
    /// Two or more candidates tie within the policy margin.
    Ambiguous,
    /// No candidate matched the frozen descriptor.
    NoCandidates,
    /// The best candidate scored below the auto-bind threshold.
    LowConfidence,
}

/// One repair candidate returned by the ladder (SCHEMA §9 `candidates[]`).
///
/// `score` is the normalized `[0,1]` versioned confidence and `margin` is
/// `score1 − score2` (SCHEMA §10). `feature_contributions` (SCHEMA
/// `featureContributions`) and any other worker fields round-trip via `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairCandidate {
    /// Snapshot-scoped topology key of the candidate element.
    pub topo_key: crate::ids::TopoKey,
    /// Normalized `[0,1]` confidence.
    pub score: f64,
    /// `score1 − score2` (best minus second-best).
    pub margin: f64,
    /// Candidate world position (for highlighting).
    pub world_pos: Vec3,
    /// Human-readable summary (e.g. `"planar face, area≈120mm²"`).
    pub summary: String,
    /// Unknown keys (e.g. `featureContributions`), preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// A single unresolved reference awaiting repair (SCHEMA §9 payload +
/// V1/V2 §3.7 per-step storage).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairItem {
    /// Timeline step whose regen produced this NeedsRepair.
    pub step_index: usize,
    /// The op-input ref identity (e.g. `"op_5.input0"`).
    pub ref_id: String,
    /// The last-known `ElementId` of the ref, if any (SCHEMA `elementId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<ElementId>,
    /// Which ladder level failed.
    pub ladder_failed: LadderLevel,
    /// Why binding failed.
    pub reason: RepairReason,
    /// Ranked candidates (sorted by `score` descending; SCHEMA §9).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<RepairCandidate>,
    /// The `resolverVersion` the candidate scores were computed under (SCHEMA §9 `scoringVersion`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoring_version: Option<u32>,
    /// Selection intent captured when the ref was authored (SCHEMA §9 `anchor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<AnchorIntent>,
    /// UI-friendly label (SCHEMA `uiLabel`; V1/V2 §3.7 "UI-friendly labels").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ui_label: String,
    /// **Policy seed, not a ladder outcome.** `true` marks an item the EDIT layer
    /// planted to force the repair flow — the SCHEMA §7.3 `TransformBody`
    /// edit-safety gate. A seeded item is NOT produced by a regen and therefore
    /// survives [`RepairState::set_step`] / [`RepairState::clear_from`]; only an
    /// explicit un-seed (the repair flow rewriting the ref) or undo clears it.
    ///
    /// Absent from the wire when `false`, so every pre-existing document and
    /// worker-published item is byte-identical to before this field existed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub seeded: bool,
}

/// `skip_serializing_if` predicate for a `false` flag (keeps the wire byte-stable).
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// The document's repair state: unresolved refs organized by step (V1/V2 §3.7).
///
/// Stored as a flat, order-stable `Vec<RepairItem>` (each item self-describes
/// its `step_index`); accessors project per-step views and [`clear_from`] drops
/// everything at or after a step (a re-regen from step `k` clears stale repair
/// state for `[k, ∞)`). Serializes transparently as the item array.
///
/// [`clear_from`]: RepairState::clear_from
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepairState {
    items: Vec<RepairItem>,
}

impl RepairState {
    /// An empty repair state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True iff there are no unresolved refs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Total unresolved-ref count across all steps.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// All repair items (order-stable).
    #[must_use]
    pub fn items(&self) -> &[RepairItem] {
        &self.items
    }

    /// The repair items for a single step.
    #[must_use]
    pub fn items_for_step(&self, step: usize) -> Vec<&RepairItem> {
        self.items.iter().filter(|i| i.step_index == step).collect()
    }

    /// True iff any step has unresolved refs (the document `NeedsRepair` badge).
    #[must_use]
    pub fn needs_repair(&self) -> bool {
        !self.items.is_empty()
    }

    /// Replaces the repair items for `step` with `items` (a step re-regen
    /// publishes a fresh NeedsRepair set for that step). Existing items for the
    /// step are dropped first; the result stays sorted by `(step_index, ref_id)`.
    ///
    /// **Seeded items are preserved**: they encode an edit-layer policy gate
    /// ([`RepairItem::seeded`]), not a ladder outcome, so a regen may not clear
    /// them — the whole point of the gate is that a regen must NOT re-decide those
    /// refs on its own.
    pub fn set_step(&mut self, step: usize, items: Vec<RepairItem>) {
        self.items.retain(|i| i.step_index != step || i.seeded);
        self.items.extend(items);
        self.sort();
    }

    /// Drops all non-seeded repair items at or after `step` (a re-regen from
    /// `step` invalidates their bindings). Seeded items survive — see
    /// [`set_step`](Self::set_step).
    pub fn clear_from(&mut self, step: usize) {
        self.items.retain(|i| i.step_index < step || i.seeded);
    }

    /// Drops every repair item, seeded ones included (a full reset — e.g. the
    /// clear-publish path where the document holds no geometry at all).
    pub fn clear(&mut self) {
        self.items.clear();
    }

    // ── Seeded (policy-gate) items ───────────────────────────────────────────

    /// Adds `items` as seeded policy gates, de-duplicated by `(step, ref_id)`
    /// against what is already stored. Each item's `seeded` flag is forced on.
    pub fn seed(&mut self, items: impl IntoIterator<Item = RepairItem>) {
        for mut item in items {
            item.seeded = true;
            if self
                .items
                .iter()
                .any(|i| i.step_index == item.step_index && i.ref_id == item.ref_id)
            {
                continue;
            }
            self.items.push(item);
        }
        self.sort();
    }

    /// Drops every seeded item for `step` (the repair flow closed the gate).
    /// Non-seeded (ladder-produced) items for that step are untouched.
    pub fn clear_seeded_for_step(&mut self, step: usize) {
        self.items.retain(|i| !(i.seeded && i.step_index == step));
    }

    /// The seeded (policy-gate) items, order-stable.
    #[must_use]
    pub fn seeded_items(&self) -> Vec<RepairItem> {
        self.items.iter().filter(|i| i.seeded).cloned().collect()
    }

    /// The distinct timeline steps carrying a seeded gate, ascending.
    #[must_use]
    pub fn seeded_steps(&self) -> Vec<usize> {
        let mut steps: Vec<usize> = self
            .items
            .iter()
            .filter(|i| i.seeded)
            .map(|i| i.step_index)
            .collect();
        steps.sort_unstable();
        steps.dedup();
        steps
    }

    /// The LOWEST timeline step carrying a seeded gate — the regen **ceiling**:
    /// a plan may execute steps strictly below it and no further, so a gated ref
    /// is never bound outside the repair flow (SCHEMA §7.3 edit-safety gate).
    #[must_use]
    pub fn first_seeded_step(&self) -> Option<usize> {
        self.items
            .iter()
            .filter(|i| i.seeded)
            .map(|i| i.step_index)
            .min()
    }

    /// Replaces the seeded subset with `seeded`, leaving ladder-produced items
    /// alone. Used to mirror the authoritative document's gates into the regen
    /// session copy without clobbering worker-published repair state.
    pub fn sync_seeded(&mut self, seeded: Vec<RepairItem>) {
        self.items.retain(|i| !i.seeded);
        self.items.extend(seeded.into_iter().map(|mut i| {
            i.seeded = true;
            i
        }));
        self.sort();
    }

    fn sort(&mut self) {
        self.items.sort_by(|a, b| {
            a.step_index
                .cmp(&b.step_index)
                .then_with(|| a.ref_id.cmp(&b.ref_id))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TopoKey;

    fn item(step: usize, ref_id: &str) -> RepairItem {
        RepairItem {
            step_index: step,
            ref_id: ref_id.into(),
            element_id: None,
            ladder_failed: LadderLevel::Descriptor,
            reason: RepairReason::Ambiguous,
            candidates: vec![RepairCandidate {
                topo_key: TopoKey::new("f:31"),
                score: 0.91,
                margin: 0.0,
                world_pos: Vec3::new_unchecked(12.0, 3.5, 0.0),
                summary: "planar face".into(),
                extra: Default::default(),
            }],
            anchor: None,
            ui_label: "Fillet edge".into(),
            scoring_version: None,
            seeded: false,
        }
    }

    fn seeded_item(step: usize, ref_id: &str) -> RepairItem {
        RepairItem {
            seeded: true,
            ..item(step, ref_id)
        }
    }

    #[test]
    fn seeded_items_survive_regen_publication_and_clear_from() {
        let mut r = RepairState::new();
        r.seed([seeded_item(3, "op_3.gate")]);
        r.set_step(3, vec![item(3, "op_3.input0")]);
        assert_eq!(r.items_for_step(3).len(), 2, "the seed rides alongside");
        // A fresh regen publishes an EMPTY set for step 3 — the seed still stands.
        r.set_step(3, vec![]);
        assert_eq!(r.items_for_step(3).len(), 1);
        assert!(r.items_for_step(3)[0].seeded);
        // clear_from wipes ladder items but never a policy gate.
        r.set_step(4, vec![item(4, "op_4.input0")]);
        r.clear_from(0);
        assert_eq!(r.len(), 1, "only the seed survives a full clear_from");
        assert_eq!(r.first_seeded_step(), Some(3));
        assert_eq!(r.seeded_steps(), vec![3]);
        // The explicit un-seed (the repair flow) closes it.
        r.clear_seeded_for_step(3);
        assert!(r.is_empty());
    }

    #[test]
    fn seed_is_idempotent_and_sync_seeded_replaces_the_subset() {
        let mut r = RepairState::new();
        r.seed([seeded_item(2, "a"), seeded_item(2, "a")]);
        assert_eq!(r.len(), 1, "duplicate (step, refId) seeds collapse");
        r.set_step(2, vec![item(2, "ladder")]);
        r.sync_seeded(vec![item(5, "b")]); // note: `seeded` forced on by sync
        assert_eq!(r.seeded_steps(), vec![5]);
        assert_eq!(
            r.items_for_step(2).len(),
            1,
            "the ladder item at step 2 is untouched by a seed sync"
        );
    }

    #[test]
    fn seeded_flag_is_absent_from_the_wire_when_false() {
        let v = serde_json::to_value(item(1, "op_1.input0")).unwrap();
        assert!(v.get("seeded").is_none(), "byte-stable for old documents");
        let v = serde_json::to_value(seeded_item(1, "op_1.gate")).unwrap();
        assert_eq!(v["seeded"], serde_json::json!(true));
        let back: RepairItem = serde_json::from_value(v).unwrap();
        assert!(back.seeded);
    }

    #[test]
    fn set_step_replaces_and_clear_from_trims() {
        let mut r = RepairState::new();
        r.set_step(2, vec![item(2, "op_2.input0")]);
        r.set_step(5, vec![item(5, "op_5.input0"), item(5, "op_5.input1")]);
        assert_eq!(r.len(), 3);
        assert_eq!(r.items_for_step(5).len(), 2);
        // Re-regen step 5 with a single unresolved ref.
        r.set_step(5, vec![item(5, "op_5.input0")]);
        assert_eq!(r.items_for_step(5).len(), 1);
        // clear_from(5) drops step 5 but keeps step 2.
        r.clear_from(5);
        assert!(r.items_for_step(5).is_empty());
        assert_eq!(r.items_for_step(2).len(), 1);
        assert!(r.needs_repair());
    }

    #[test]
    fn enums_serialize_to_schema_tokens() {
        assert_eq!(
            serde_json::to_value(LadderLevel::History).unwrap(),
            serde_json::json!("history")
        );
        assert_eq!(
            serde_json::to_value(RepairReason::NoCandidates).unwrap(),
            serde_json::json!("no-candidates")
        );
        assert_eq!(
            serde_json::to_value(RepairReason::LowConfidence).unwrap(),
            serde_json::json!("low-confidence")
        );
    }

    #[test]
    fn state_serializes_transparently_as_array() {
        let mut r = RepairState::new();
        r.set_step(1, vec![item(1, "op_1.input0")]);
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.is_array());
        let back: RepairState = serde_json::from_value(v).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn scoring_version_round_trips_and_is_absent_when_none() {
        // None: field is dropped from the wire (byte-stable for old documents).
        let none_item = item(1, "op_1.input0");
        let v = serde_json::to_value(&none_item).unwrap();
        assert!(v.get("scoringVersion").is_none());
        let back: RepairItem = serde_json::from_value(v).unwrap();
        assert_eq!(back, none_item);

        // Some(1): preserved verbatim through a round trip (SCHEMA §9 `scoringVersion`).
        let mut some_item = item(1, "op_1.input0");
        some_item.scoring_version = Some(1);
        let v = serde_json::to_value(&some_item).unwrap();
        assert_eq!(v["scoringVersion"], serde_json::json!(1));
        let back: RepairItem = serde_json::from_value(v).unwrap();
        assert_eq!(back, some_item);
        assert_eq!(back.scoring_version, Some(1));
    }
}
