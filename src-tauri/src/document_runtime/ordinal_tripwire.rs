//! VF-B6 — the split-ordinal identity tripwire.
//!
//! # The defect
//!
//! An N-body op mints its children as `body_<opId>:<k>` where `k` is the child's
//! rank in a **geometric** sort (`worker/src/ops/OpCommon.cpp` `ranked_solids`:
//! quantized volume, then centroid, then face count). The id is stable, but the rank
//! is not lineage: edit a cut so the two pieces cross in that order and `:0` now
//! names the *other* solid. Every downstream reference then re-resolves **cleanly** —
//! full confidence, no ladder complaint — onto the wrong body. That is precisely the
//! silent mis-bind class (H5-B) this stack exists to eliminate; a deterministic
//! `NeedsRepair` is always better than a confident wrong answer.
//!
//! # The detection: ordinal ↔ position must stay put
//!
//! The worker now publishes the rank key it ordered each child by (SCHEMA §7.2
//! `bodyEvents[].rankKey`), which regen stamps onto
//! [`BodyMeta::geom_stamp`](onecad_core::document::body::BodyMeta::geom_stamp). The
//! key's centroid `(cx, cy, cz)` gives a **second, independent** ordering of the same
//! children — and unlike volume it is a lineage proxy: under a parametric edit a
//! piece stays roughly where it was, while its size is exactly what the edit changes.
//!
//! So the invariant is: **the map `ordinal → positional rank` does not change.** If
//! child `:0` was the leftmost piece and is now the rightmost, `:0` changed which
//! solid it names, whatever the volumes did.
//!
//! ## Why not "match each child to its nearest rank key"
//!
//! Because on the canonical case it silently answers "no permutation". Take a plate
//! cut into pieces of length 17 and 19 (volumes 8500 / 9500, centroids 8.5 / 30.5),
//! then move the cut so the lengths swap to 19 and 17. The two volume VALUES are
//! identical before and after — they merely changed owner — so a nearest-key match
//! pairs each ordinal with the ordinal that has the same volume, i.e. with **itself**,
//! and reports identity. Volume is the component that *defines* the ordinal, so
//! matching on it is circular. Weighting it down does not fix the circularity, and
//! normalizing per-component makes it worse (with the two sets nearly equal, the
//! normalizer *is* the difference, so every gap saturates at 1.0). Comparing the two
//! ORDERINGS sidesteps distance entirely: no metric, no normalization, no thresholds,
//! and no dependence on how large the edit was.
//!
//! # Failure bias
//!
//! Missing or unusable evidence is **no claim**: an unstamped child, a changed
//! ordinal set, or two children sharing a quantized centroid all mean "say nothing" —
//! never "assume fine", never "assume broken". A legacy document therefore raises no
//! false alarm on its first regen; it adopts stamps and is protected from then on.
//! In the other direction the check is deliberately eager: children that genuinely
//! traded *positions* while keeping their ordinals would be reported as a permutation.
//! That is a spurious `NeedsRepair`, which the H5-B bar prefers over a silent
//! mis-bind, and the repair flow closes it permanently.

use std::collections::{BTreeMap, BTreeSet};

use onecad_core::document::body::BodyRegistry;
use onecad_core::document::repair::{
    OrdinalAnchor, OrdinalStamp, RankKey, RepairItem, RepairReason, RepairState,
};
use onecad_core::document::Document;
use onecad_core::ids::{BodyId, RecordId};

/// The centroid slice of a [`RankKey`] — the positional (lineage-proxy) ordering key.
type Position = [i64; 3];

/// One op's decision.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The ordinals permuted. `swaps` are `(was, is_now)` pairs: the solid that held
    /// ordinal `was` now holds ordinal `is_now`.
    Permuted { swaps: Vec<(usize, usize)> },
    /// The ordinal↔position map matches the anchor — any gate for this op self-heals.
    Identity,
}

/// What [`evaluate`] decided, ready to apply to the live session.
#[derive(Debug, Default)]
pub(crate) struct Tripwire {
    /// Gates to plant (already seeded + anchored).
    pub items: Vec<RepairItem>,
    /// Ops whose existing gates must be dropped (the ordering came home).
    pub healed: Vec<RecordId>,
}

impl Tripwire {
    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty() && self.healed.is_empty()
    }
}

/// Decide the tripwire for a just-driven regen.
///
/// * `prior` — the registry the PREVIOUS regen left behind (the baseline when the op
///   carries no gate yet). Must be read **before** the runtime overwrites its regen
///   mirror; it is the only in-memory source that is both seeded from the persisted
///   registry at open and refreshed by every commit, so it survives save/reopen
///   without inheriting `adopt_regen_bodies`' insert-only staleness.
/// * `current` — the registry the just-driven regen produced.
/// * `document` — the authoritative document: its timeline places each op on a step,
///   and its `repair` supplies the anchors of gates already standing.
pub(crate) fn evaluate(
    prior: &BodyRegistry,
    current: &BodyRegistry,
    document: &Document,
) -> Tripwire {
    let prior_children = prior.ordinal_children();
    let current_children = current.ordinal_children();
    let mut out = Tripwire::default();

    for (op, now) in &current_children {
        // The gate's own anchor wins over the previous regen: it is the ordering the
        // user last saw as correct, and re-testing against it is what keeps a raised
        // gate raised across intervening regens that merely republish the same
        // permuted geometry (see `OrdinalAnchor`).
        let baseline: BTreeMap<usize, RankKey> = match document.repair.ordinal_anchor_for(*op) {
            Some(anchor) => anchor.keys(),
            None => match prior_children.get(op) {
                Some(before) => before.iter().map(|(k, (_, key))| (*k, *key)).collect(),
                // No anchor and no prior stamps ⇒ nothing to compare (a fresh
                // document, or a legacy one adopting stamps for the first time).
                None => continue,
            },
        };
        let live: BTreeMap<usize, RankKey> = now.iter().map(|(k, (_, key))| (*k, *key)).collect();

        match verdict(&baseline, &live) {
            None => {}
            Some(Verdict::Identity) => out.healed.push(*op),
            Some(Verdict::Permuted { swaps }) => {
                out.items
                    .extend(gate_items(document, *op, &swaps, now, &baseline));
            }
        }
    }

    // An op that no longer produces comparable ordinal children cannot permute;
    // leaving its gate standing would pin the regen ceiling on a record nothing could
    // ever satisfy.
    for op in document.repair.ordinal_gated_ops() {
        if !current_children.contains_key(&op) && !out.healed.contains(&op) {
            out.healed.push(op);
        }
    }
    out
}

/// The verdict for one op, or `None` when no claim can be made.
///
/// No claim when: fewer than two ordinals, the ordinal SETS differ, or either side
/// has two children at the same quantized centroid. A changed set is a genuinely
/// different partition, not a permutation — the existing no-candidate `NeedsRepair`
/// path already covers a reference to a child that stopped existing, and inventing a
/// correspondence across a count change would be exactly the confident guess this
/// module refuses to make.
fn verdict(
    baseline: &BTreeMap<usize, RankKey>,
    live: &BTreeMap<usize, RankKey>,
) -> Option<Verdict> {
    if baseline.len() < 2 {
        return None;
    }
    if baseline.keys().ne(live.keys()) {
        return None;
    }
    let before = positional_ranks(baseline)?;
    let after = positional_ranks(live)?;
    if before == after {
        return Some(Verdict::Identity);
    }
    // Both maps are bijections ordinal→slot over the same sets, so inverting `after`
    // turns "who sits where" into "the solid that WAS at ordinal `was` is now at
    // ordinal `is_now`".
    let by_slot: BTreeMap<usize, usize> = after.iter().map(|(k, slot)| (*slot, *k)).collect();
    let swaps: Vec<(usize, usize)> = before
        .iter()
        .filter_map(|(was, slot)| by_slot.get(slot).map(|is_now| (*was, *is_now)))
        .filter(|(was, is_now)| was != is_now)
        .collect();
    // `before != after` over equal key sets guarantees at least one moved.
    debug_assert!(!swaps.is_empty());
    Some(Verdict::Permuted { swaps })
}

/// `ordinal → positional rank`, ranking children by their quantized centroid
/// (lexicographic `cx, cy, cz`).
///
/// `None` when two children share a centroid: the positional proxy then carries no
/// information, and their relative ordinal is decided by face count alone — a
/// genuinely fragile ordinal about which this module declines to make any claim
/// rather than guess. (Disjoint solids sharing a centroid to 1e-6 is a nested/shelled
/// configuration, not a plain split.)
fn positional_ranks(keys: &BTreeMap<usize, RankKey>) -> Option<BTreeMap<usize, usize>> {
    let mut by_pos: Vec<(Position, usize)> = keys
        .iter()
        .map(|(&k, key)| ([key[1], key[2], key[3]], k))
        .collect();
    by_pos.sort_unstable();
    let distinct: BTreeSet<Position> = by_pos.iter().map(|(p, _)| *p).collect();
    if distinct.len() != by_pos.len() {
        return None;
    }
    Some(
        by_pos
            .into_iter()
            .enumerate()
            .map(|(slot, (_, k))| (k, slot))
            .collect(),
    )
}

/// The seeded gates for one permuted op: every downstream record standing on one of
/// the moved children, walked by the SAME [`gate_items_for_moved`] the edit-lane
/// transform gate uses, then stamped with the permutation reason + anchor.
///
/// [`gate_items_for_moved`]: onecad_core::edit::gate_items_for_moved
fn gate_items(
    document: &Document,
    op: RecordId,
    swaps: &[(usize, usize)],
    children: &BTreeMap<usize, (BodyId, RankKey)>,
    baseline: &BTreeMap<usize, RankKey>,
) -> Vec<RepairItem> {
    let Some(step) = document.timeline.index_of(op) else {
        return Vec::new();
    };
    // Only the ordinals that actually moved are compromised; a child that kept its
    // rank still names the same solid, and gating its dependents would be noise.
    let moved: Vec<BodyId> = swaps
        .iter()
        .filter_map(|(was, _)| children.get(was).map(|(b, _)| *b))
        .collect();
    let pairs = swaps
        .iter()
        .map(|(was, is_now)| format!(":{was}→:{is_now}"))
        .collect::<Vec<_>>()
        .join(", ");
    let label = format!(
        "split children of step {step} changed geometric order ({pairs}); this reference \
         would now bind a DIFFERENT solid — re-pick it"
    );
    let anchor = OrdinalAnchor {
        op,
        stamps: baseline
            .iter()
            .map(|(&k, &key)| OrdinalStamp { k, key })
            .collect(),
    };
    onecad_core::edit::gate_items_for_moved(document, &moved, step + 1, &label)
        .into_iter()
        .map(|mut item| {
            item.reason = RepairReason::OrdinalPermutation;
            item.ordinal_anchor = Some(anchor.clone());
            item
        })
        .collect()
}

/// Applies a decided [`Tripwire`] to a [`RepairState`] — used for BOTH the
/// authoritative document copy and the regen mirror, which must not diverge.
pub(crate) fn apply(repair: &mut RepairState, tripwire: &Tripwire) {
    for op in &tripwire.healed {
        repair.clear_ordinal_gates(*op);
    }
    repair.seed(tripwire.items.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(volume, cx, cy, cz, faceCount)` at 1e-6, mm³ / mm.
    fn key(vol: f64, cx: f64) -> RankKey {
        [
            (vol * 1e6) as i64,
            (cx * 1e6) as i64,
            10_000_000,
            12_500_000,
            6,
        ]
    }
    fn keys(pairs: &[(usize, RankKey)]) -> BTreeMap<usize, RankKey> {
        pairs.iter().copied().collect()
    }

    /// The canonical VF-B6 case, in the exact numbers `ordinal_tripwire.rs` (integration)
    /// drives: a 40-long plate cut into 17 + 19, then the cut moved so it is 19 + 17.
    /// The two VOLUMES merely change owner, which is why a nearest-key match reports
    /// identity here — the centroids are what give the lineage away.
    #[test]
    fn a_cut_edit_that_swaps_piece_sizes_is_a_permutation() {
        // ordinal 0 = the 17-long piece (vol 8500, centroid 8.5); ordinal 1 = 19-long.
        let before = keys(&[(0, key(8500.0, 8.5)), (1, key(9500.0, 30.5))]);
        // The cut moved 2mm: the left piece is now the BIG one and takes ordinal 1.
        let after = keys(&[(0, key(8500.0, 31.5)), (1, key(9500.0, 9.5))]);
        assert_eq!(
            verdict(&before, &after),
            Some(Verdict::Permuted {
                swaps: vec![(0, 1), (1, 0)]
            }),
            "the piece that held :0 now holds :1"
        );
    }

    #[test]
    fn a_resize_that_preserves_the_order_is_not_a_permutation() {
        let before = keys(&[(0, key(1000.0, 10.0)), (1, key(5000.0, 90.0))]);
        // Both pieces grow; neither overtakes the other, and neither moves past the
        // other.
        let after = keys(&[(0, key(2000.0, 12.0)), (1, key(6000.0, 92.0))]);
        assert_eq!(verdict(&before, &after), Some(Verdict::Identity));
    }

    #[test]
    fn an_unchanged_republish_is_identity() {
        let same = keys(&[(0, key(1000.0, 1.0)), (1, key(2000.0, 9.0))]);
        assert_eq!(
            verdict(&same, &same),
            Some(Verdict::Identity),
            "a regen that republishes the same geometry must not raise a gate"
        );
    }

    /// A whole-part translate moves every centroid by the same amount — the ordinal↔
    /// position map is unchanged, so no gate.
    #[test]
    fn a_rigid_translate_of_the_whole_op_is_identity() {
        let before = keys(&[(0, key(1000.0, 10.0)), (1, key(5000.0, 90.0))]);
        let after = keys(&[(0, key(1000.0, 60.0)), (1, key(5000.0, 140.0))]);
        assert_eq!(verdict(&before, &after), Some(Verdict::Identity));
    }

    #[test]
    fn a_changed_ordinal_set_makes_no_claim() {
        let two = keys(&[(0, key(1.0, 1.0)), (1, key(2.0, 2.0))]);
        let three = keys(&[(0, key(1.0, 1.0)), (1, key(2.0, 2.0)), (2, key(3.0, 3.0))]);
        assert_eq!(
            verdict(&two, &three),
            None,
            "3 children where there were 2 is a different partition, not a permutation"
        );
        assert_eq!(verdict(&three, &two), None);
        assert_eq!(verdict(&keys(&[(0, key(1.0, 1.0))]), &two), None, "n < 2");
    }

    #[test]
    fn coincident_centroids_make_no_claim() {
        let mut nested = keys(&[(0, key(1.0, 5.0)), (1, key(2.0, 5.0))]);
        // Same centroid on all three axes ⇒ position orders nothing.
        assert_eq!(verdict(&nested, &nested), None);
        nested.insert(1, key(2.0, 6.0));
        assert!(verdict(&nested, &nested).is_some(), "distinct ⇒ comparable");
    }

    #[test]
    fn a_three_way_rotation_maps_every_ordinal() {
        // Slots at x = 0 / 10 / 20 hold ordinals 0,1,2 (volumes 1 < 2 < 3).
        let before = keys(&[(0, key(1.0, 0.0)), (1, key(2.0, 10.0)), (2, key(3.0, 20.0))]);
        // Volumes rotate between the slots, so the ordinals rotate with them.
        let after = keys(&[(0, key(1.0, 10.0)), (1, key(2.0, 20.0)), (2, key(3.0, 0.0))]);
        let Some(Verdict::Permuted { swaps }) = verdict(&before, &after) else {
            panic!("a full rotation must be flagged");
        };
        assert_eq!(swaps, vec![(0, 2), (1, 0), (2, 1)]);
        let froms: BTreeSet<usize> = swaps.iter().map(|(f, _)| *f).collect();
        let tos: BTreeSet<usize> = swaps.iter().map(|(_, t)| *t).collect();
        assert_eq!(
            froms, tos,
            "the mapping is a bijection over the same ordinals"
        );
    }
}
