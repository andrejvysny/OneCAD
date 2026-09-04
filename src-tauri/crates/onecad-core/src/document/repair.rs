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

use serde::{Deserialize, Deserializer, Serialize};

use crate::document::refs::{AnchorIntent, Extra};
use crate::ids::{ElementId, RecordId};
use crate::math::Vec3;

/// Which ladder level failed to decide (SCHEMA §9 `ladderFailed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LadderLevel {
    /// OCCT history gave no / an ambiguous mapping.
    History,
    /// Descriptor + anchor matching was ambiguous / low-confidence.
    ///
    /// Also the level an OP-BUILT item carries when **no ladder ran at all**
    /// (SCHEMA §9): `ladderFailed` is a closed enum with nowhere to say "not
    /// applicable", so [`RepairReason::LegacyReferenceFace`] — a policy halt on an
    /// EMPTY slot, not a failed rebind — reports the last level and lets the
    /// `reason` token be the discriminator.
    Descriptor,
}

/// Why the ladder could not confidently bind (SCHEMA §9 `reason`).
///
/// **Forward compatibility.** This is a bare unit enum persisted verbatim in
/// `document.json`, so a derived `Deserialize` would make an OLDER release **fail
/// to open** a document written by a newer one that added a token. The hand-written
/// [`Deserialize`] below therefore degrades any unrecognized token to
/// [`Unknown`](Self::Unknown) instead of erroring. Serialization stays derived
/// (kebab-case), so a round-trip through an older release flattens the unknown
/// token to `"unknown"` — lossy, but recoverable state (the next regen republishes
/// the real reason) versus an unopenable file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepairReason {
    /// A `reason` token this build does not know (a newer release wrote it).
    /// Never produced by this build's own ladder or policy — only by
    /// deserialization.
    Unknown,
    /// Two or more candidates tie within the policy margin.
    Ambiguous,
    /// No candidate matched the frozen descriptor.
    NoCandidates,
    /// The best candidate scored below the auto-bind threshold.
    LowConfidence,
    /// **Rust-seeded policy gate, never a worker outcome** (VF-B6). The reference
    /// stands on an ordinal child `body_<opId>:<k>` of an N-body op, and the §7.2
    /// `rankKey` evidence shows the op's children **permuted** across a parametric
    /// edit: `:<k>` now names a different solid than the ref was authored against.
    /// Left alone the ref would re-resolve *cleanly* to the WRONG body — the exact
    /// silent mis-bind (H5-B) this stack exists to eliminate.
    OrdinalPermutation,
    /// **Worker-emitted but OP-BUILT, never a ladder outcome** (SCHEMA §9,
    /// kernel-hardening WP-F 2026-09-03). An asymmetric Chamfer that carries no
    /// `referenceFaces` pair for a contour used to measure `radius` on the adjacent
    /// face with the smaller SNAPSHOT-SCOPED ordinal — so an upstream edit that
    /// reordered the face map silently mirrored the chamfer's legs. That fallback is
    /// GONE: an uncovered contour now halts with one such item in EVERY lane,
    /// regardless of §7.2 `editedFrom` (gating it on the edit claim made the guard
    /// fire on open and vanish on redo). `ref_id` names the EMPTY slot the pair will
    /// occupy,
    /// `element_id` is absent, [`seed_edge_id`](RepairItem::seed_edge_id) names the
    /// contour edge the pair is keyed by, and the two candidates are that edge's
    /// adjacent faces at a deliberate tie (`score` 0.5, `margin` 0.0 — the user MUST
    /// choose). Repair is a CREATE, not a rebind.
    ///
    /// The wire token is camelCase (`"legacyReferenceFace"`), unlike the kebab-case
    /// ladder outcomes — SCHEMA §9 names it that way, so the variant is renamed
    /// rather than left to the derived kebab-case rule.
    #[serde(rename = "legacyReferenceFace")]
    LegacyReferenceFace,
}

impl<'de> Deserialize<'de> for RepairReason {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(d)?.as_str() {
            "ambiguous" => Self::Ambiguous,
            "no-candidates" => Self::NoCandidates,
            "low-confidence" => Self::LowConfidence,
            "ordinal-permutation" => Self::OrdinalPermutation,
            "legacyReferenceFace" => Self::LegacyReferenceFace,
            _ => Self::Unknown,
        })
    }
}

/// The quantized geometric rank key an N-body op ordered one child by (SCHEMA §7.2
/// `bodyEvents[].rankKey`): `[volume, cx, cy, cz, faceCount]`, the first four
/// `llround(value * 1e6)`.
///
/// Integers on purpose: [`BodyMeta`](crate::document::body::BodyMeta),
/// [`BodyRegistry`](crate::document::body::BodyRegistry) and
/// [`Document`](crate::document::Document) all need `Eq`, which a float triple
/// (`Vec3`) cannot give — and the worker's ordinal assignment is *defined* on the
/// quantized tuple, so storing anything else would compare a different thing than
/// the one that decided the ordinal.
pub type RankKey = [i64; 5];

/// One `(ordinal, rank key)` pair of an [`OrdinalAnchor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrdinalStamp {
    /// The split ordinal `k` in `body_<opId>:<k>`.
    pub k: usize,
    /// The §7.2 rank key that ordinal held **before** the permutation.
    pub key: RankKey,
}

/// The pre-permutation ordinal→rank-key snapshot a [`RepairReason::OrdinalPermutation`]
/// gate was seeded against (VF-B6).
///
/// **This is what makes the tripwire self-healing without a command inverse.** The
/// gate is planted from the REGEN side (`commit_snapshot`), which is not an
/// `EditCommand` and therefore has no undo entry to ride. Instead the gate carries
/// the state it was raised against: every later regen re-tests the op's CURRENT
/// stamps against this anchor rather than against the immediately-previous regen, so
///
/// * a gate stays raised across intervening regens that merely re-publish the
///   permuted geometry (comparing against the previous regen would see "no change"
///   and wrongly clear it),
/// * **undoing** the offending edit restores the anchored ordering, the comparison
///   comes back identity, and the tripwire drops its own seeds, and
/// * closing the gate through the repair flow ([`RepairState::clear_seeded_for_step`])
///   destroys the anchor with it, so the *new* ordering silently becomes the
///   baseline and the gate is not immediately re-seeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrdinalAnchor {
    /// The N-body op whose ordinal children permuted.
    pub op: RecordId,
    /// The ordinals and the rank keys they held before the permutation, ascending
    /// by `k`.
    pub stamps: Vec<OrdinalStamp>,
}

impl OrdinalAnchor {
    /// The anchor's `k → key` map.
    #[must_use]
    pub fn keys(&self) -> std::collections::BTreeMap<usize, RankKey> {
        self.stamps.iter().map(|s| (s.k, s.key)).collect()
    }
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
    /// The pre-permutation ordinal snapshot a [`RepairReason::OrdinalPermutation`]
    /// gate was raised against (VF-B6) — see [`OrdinalAnchor`] for why the gate
    /// carries its own baseline. `None` on every other item, and skipped on the
    /// wire, so pre-VF-B6 documents and worker-published items stay byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal_anchor: Option<OrdinalAnchor>,
    /// The contour edge a [`RepairReason::LegacyReferenceFace`] repair must pair the
    /// chosen face with (SCHEMA §9 `seedEdgeId`, kernel-hardening WP-F).
    ///
    /// The item names an EMPTY slot, so there is no stored ref to read the edge off:
    /// without this the repair could not key the `{edgeId, faceId}` pair it creates.
    /// `None` on every other reason, and skipped on the wire, so pre-WP-F documents
    /// and worker-published items stay byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_edge_id: Option<ElementId>,
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

    // ── VF-B6 ordinal-permutation gates ──────────────────────────────────────

    /// The anchor of the ordinal-permutation gate raised for `op`, if any (the
    /// baseline the tripwire re-tests against — see [`OrdinalAnchor`]). All items of
    /// one gate share the anchor, so the first match is authoritative.
    #[must_use]
    pub fn ordinal_anchor_for(&self, op: RecordId) -> Option<&OrdinalAnchor> {
        self.items
            .iter()
            .filter(|i| i.seeded && i.reason == RepairReason::OrdinalPermutation)
            .find_map(|i| i.ordinal_anchor.as_ref().filter(|a| a.op == op))
    }

    /// The ops currently carrying an ordinal-permutation gate, ascending by id.
    #[must_use]
    pub fn ordinal_gated_ops(&self) -> Vec<RecordId> {
        let mut ops: Vec<RecordId> = self
            .items
            .iter()
            .filter(|i| i.seeded && i.reason == RepairReason::OrdinalPermutation)
            .filter_map(|i| i.ordinal_anchor.as_ref().map(|a| a.op))
            .collect();
        ops.sort_unstable_by_key(|r| r.0);
        ops.dedup();
        ops
    }

    /// Drops every ordinal-permutation gate anchored on `op` (the tripwire's own
    /// self-heal: the ordering came back to the anchor). Returns `true` if anything
    /// was dropped. Ladder-produced items and gates of other kinds are untouched.
    pub fn clear_ordinal_gates(&mut self, op: RecordId) -> bool {
        let before = self.items.len();
        self.items.retain(|i| {
            !(i.seeded
                && i.reason == RepairReason::OrdinalPermutation
                && i.ordinal_anchor.as_ref().is_some_and(|a| a.op == op))
        });
        self.items.len() != before
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

    /// Remaps seeded `step_index`es after a record is INSERTED at `at`
    /// (VF-B5c): every gate at or after the insertion point shifts down the
    /// timeline by one.
    ///
    /// `step_index` is POSITIONAL — nothing else ties a gate to its record — so
    /// without this a timeline insert silently re-points the regen ceiling
    /// ([`first_seeded_step`](Self::first_seeded_step)) at the wrong step, either
    /// truncating an innocent op or letting the gated one execute. Ladder-produced
    /// items are deliberately untouched: they are re-published (or dropped) by the
    /// next regen of their step, which is exactly what an insert forces.
    pub fn shift_seeded_for_insert(&mut self, at: usize) {
        for item in &mut self.items {
            if item.seeded && item.step_index >= at {
                item.step_index += 1;
            }
        }
        self.sort();
    }

    /// Remaps seeded `step_index`es after the record at `at` is REMOVED
    /// (VF-B5c): gates above it shift up by one, and the removed step's own gates
    /// are DROPPED — the record they guard no longer exists, so nothing could ever
    /// close them and the ceiling would pin regen forever.
    ///
    /// See [`shift_seeded_for_insert`](Self::shift_seeded_for_insert) on why
    /// ladder items are left alone.
    pub fn shift_seeded_for_remove(&mut self, at: usize) {
        self.items.retain(|i| !(i.seeded && i.step_index == at));
        for item in &mut self.items {
            if item.seeded && item.step_index > at {
                item.step_index -= 1;
            }
        }
        self.sort();
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
            ordinal_anchor: None,
            seed_edge_id: None,
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
    fn unknown_reason_token_degrades_instead_of_failing_the_open() {
        // VF-B6 review B3: `RepairReason` is persisted in document.json, so a token
        // this build has never heard of (a NEWER release wrote it) must not make the
        // document unopenable.
        let from = |tok: &str| -> RepairReason {
            serde_json::from_value(serde_json::json!(tok)).expect("never an error")
        };
        assert_eq!(from("ambiguous"), RepairReason::Ambiguous);
        assert_eq!(from("no-candidates"), RepairReason::NoCandidates);
        assert_eq!(from("low-confidence"), RepairReason::LowConfidence);
        assert_eq!(
            from("ordinal-permutation"),
            RepairReason::OrdinalPermutation
        );
        assert_eq!(from("a-reason-from-2027"), RepairReason::Unknown);
        assert_eq!(from(""), RepairReason::Unknown);
        // A whole item carrying the unknown token still deserializes.
        let mut v = serde_json::to_value(item(1, "op_1.input0")).unwrap();
        v["reason"] = serde_json::json!("something-new");
        let back: RepairItem = serde_json::from_value(v).expect("item still parses");
        assert_eq!(back.reason, RepairReason::Unknown);
    }

    #[test]
    fn ordinal_anchor_round_trips_and_is_absent_when_none() {
        use crate::ids::RecordId;
        // Absent on every pre-VF-B6 item ⇒ byte-stable for old documents.
        let v = serde_json::to_value(item(1, "op_1.input0")).unwrap();
        assert!(v.get("ordinalAnchor").is_none());

        let op = RecordId(uuid::Uuid::from_u128(0x5150));
        let anchored = RepairItem {
            reason: RepairReason::OrdinalPermutation,
            ordinal_anchor: Some(OrdinalAnchor {
                op,
                stamps: vec![
                    OrdinalStamp {
                        k: 0,
                        key: [8_500_000_000, 8_500_000, 0, 0, 6],
                    },
                    OrdinalStamp {
                        k: 1,
                        key: [9_500_000_000, 30_500_000, 0, 0, 6],
                    },
                ],
            }),
            seed_edge_id: None,
            ..seeded_item(2, "op_2.input0")
        };
        let v = serde_json::to_value(&anchored).unwrap();
        assert_eq!(v["reason"], serde_json::json!("ordinal-permutation"));
        assert_eq!(v["ordinalAnchor"]["stamps"][1]["k"], serde_json::json!(1));
        let back: RepairItem = serde_json::from_value(v).unwrap();
        assert_eq!(back, anchored);
        assert_eq!(
            back.ordinal_anchor.as_ref().unwrap().keys()[&1],
            [9_500_000_000, 30_500_000, 0, 0, 6]
        );
    }

    #[test]
    fn ordinal_gates_are_addressed_by_their_anchor_op() {
        use crate::ids::RecordId;
        let op = RecordId(uuid::Uuid::from_u128(7));
        let other = RecordId(uuid::Uuid::from_u128(8));
        let gate = |step, refid, anchor_op| RepairItem {
            reason: RepairReason::OrdinalPermutation,
            ordinal_anchor: Some(OrdinalAnchor {
                op: anchor_op,
                stamps: vec![OrdinalStamp {
                    k: 0,
                    key: [1, 2, 3, 4, 5],
                }],
            }),
            seed_edge_id: None,
            ..seeded_item(step, refid)
        };
        let mut r = RepairState::new();
        r.seed([gate(3, "a", op), gate(4, "b", op), gate(5, "c", other)]);
        r.seed([seeded_item(6, "transform-gate")]); // a NON-ordinal gate
        assert_eq!(r.ordinal_gated_ops(), vec![op, other]);
        assert!(r.ordinal_anchor_for(op).is_some());

        assert!(r.clear_ordinal_gates(op));
        assert_eq!(r.ordinal_gated_ops(), vec![other]);
        assert!(!r.clear_ordinal_gates(op), "idempotent");
        assert_eq!(
            r.seeded_steps(),
            vec![5, 6],
            "the other op's gate and the transform gate are untouched"
        );
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
        assert_eq!(
            serde_json::to_value(RepairReason::OrdinalPermutation).unwrap(),
            serde_json::json!("ordinal-permutation")
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

    /// H6a: `scoringVersion` is an OPAQUE stamp, never a compatibility gate.
    ///
    /// The worker bumped `resolverVersion` 1 → 2, so a document saved before that
    /// carries `scoringVersion: 1` on its stored repair items while the running
    /// worker stamps `2`. Rust must load such an item VERBATIM — no clamp, no
    /// upgrade-rewrite, no rejection — because the field's only job is telling a
    /// repair UI which scheme produced the numbers it is looking at. A stale stamp
    /// is information, not an error; the document's geometry is rebuilt by replay
    /// anyway, which re-stamps every live item with the current version.
    #[test]
    fn a_stale_scoring_version_loads_verbatim_and_gates_nothing() {
        let mut v1_item = item(1, "op_1.input0");
        v1_item.scoring_version = Some(1); // as written by a resolverVersion-1 worker
        let mut v2_item = item(2, "op_2.input0");
        v2_item.scoring_version = Some(2); // as written by the current worker

        // `RepairState` is an ARRAY on the wire, so a stored mixed-version state is
        // exactly this — the shape a v1-era container hands the loader.
        let stored = serde_json::json!([
            serde_json::to_value(&v1_item).unwrap(),
            serde_json::to_value(&v2_item).unwrap(),
        ]);
        let state: RepairState = serde_json::from_value(stored)
            .expect("a mixed-version repair state still deserializes");
        let back: RepairState = serde_json::from_value(serde_json::to_value(&state).unwrap())
            .expect("and survives a re-save");
        let items = back.items();
        assert_eq!(items.len(), 2, "neither item was dropped as incompatible");
        assert_eq!(items[0].scoring_version, Some(1), "v1 stamp kept verbatim");
        assert_eq!(items[1].scoring_version, Some(2), "v2 stamp kept verbatim");
        assert_eq!(back, state, "the whole state round-trips unchanged");
    }
}
