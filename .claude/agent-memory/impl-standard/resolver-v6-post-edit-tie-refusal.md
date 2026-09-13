---
name: resolver-v6-post-edit-tie-refusal
description: resolver v6 refuses an anchor-exact descriptor tie on post_upstream_edit even for an UNPROMOTED (unminted) seed ref, before any op-built pair/legacy halt is reached
metadata:
  type: project
---

`Ladder.cpp:296-382` (`anchor_decided_a_tie = descriptor_tie` when `post_upstream_edit` is set) makes v6 (`kResolverVersion = 6`, `Scoring.h`) refuse a post-edit descriptor tie even when the leading candidate scores ≥0.85 with margin ≥0.10 — SCHEMA §10 "Post-edit exact-anchor refusal (resolverVersion = 6)".

Consequence for tests that build an `element_input` seed ref WITHOUT minting it (`primary.elementId` present but never `part.mint()`-ed, so both ops take the descriptor+anchor ladder): with `post_upstream_edit = true`, the seed ref's OWN resolution halts (`reason: "ambiguous"`, `ladderFailed: "descriptor"`, `refId` naming the seed input slot) before any op-built pair/legacy `NeedsRepair` item (e.g. `legacyReferenceFace`) is ever reached. The realistic path is unaffected because a real document's seed ref is already partition-tracked (minted by a prior op), so rung-1 identity resolves it before the descriptor stage.

To exercise the "op halts on op X" behavior under `post_upstream_edit` in a from-scratch probe test, mint the seed element into the `ElementMapPartition` first (`part.mint(body, elem_id, kind, sub_shape, body_shape, anchor_json)` — signature in `ElementMapPartition.h:116-118`, usage example `test_feature_pattern.cpp:374-375`). See `worker/tests/test_chamfer_reference_face.cpp` (`ChamferSpec::promote_seed`) for the pattern: two lanes — unpromoted (asserts the v6 refusal itself) and promoted (asserts the op's own pair contract).
