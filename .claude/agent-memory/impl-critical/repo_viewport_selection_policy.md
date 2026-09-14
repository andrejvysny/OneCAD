---
name: repo-viewport-selection-policy
description: How a viewport selection ref survives (or does not survive) a body mesh swap after D-5 removed the geometric rebind and the Astra break removed ordinal trust
metadata:
  type: project
---

`src/viewport/mesh/rebindPick.ts` no longer guesses, and since the WP-U4 Astra `break`
(2026-09-11) it no longer trusts a snapshot ordinal either. It exports `ordinalForRef`,
`reconcileSelectionForBody(bodyId, prev, next, client)` and `dropSelectionForBody(bodyId)`;
the geometric nearest-face/nearest-edge search (`rebindRef`, `REBIND_TOL_FRAC`,
`REBIND_DIR_MIN`) is gone.

**Why:** the search re-pointed a consumed edge's ref at its nearest neighbour. Keeping a ref
because "the new mesh still resolves its label" is the same defect in disguise: element counts
only GROW over the measured regens (chamfer 12 → 15 edges; each hole +2 faces / +3 edges), so a
stale `f:N`/`e:N` essentially always resolves — to a different element.

**How to apply:**

- `ordinalForRef` tries the promoted `elementId` FIRST (when `IDS_HAVE_ELEMENTIDS`), the
  transient `topoKey` second. Reversed, a nine-face table with an unrelated `f:7` at ordinal 6
  and the picked `el_x` at ordinal 8 draws ordinal 6.
- Across a REGEN a ref survives only on: (a) the new id table names its `elementId` → keep and
  rewrite `topoKey` to that label so the highlight resolves; (b) it has an `elementId` the mesh
  does not name → `client.elementInfo(bodyId, elementId, "")` (topoKey EMPTY = the elementId
  rung only); (c) no `elementId` → DROP, even when the old ordinal still resolves.
- A `confirmRef` answer is accepted only if `info.bodyId` matches (through `bareBodyId`),
  `info.kind === ref.kind`, and the returned `topoKey` resolves in the DISPLAYED mesh's index.
  The head can be ahead of the mesh on screen; an undrawable key would leave an invisible but
  still authorable selection.
- A MISSING index (a blob with no EDGE tables) is now a miss, not a "named": an edge highlight
  that cannot draw must not stay selected.
- Replies are applied by ref OBJECT identity (`selected.indexOf(ref)`), never by `id`:
  `${bodyId}#${topoKey}` is REUSABLE, so a deselect plus a fresh pick of the same label mints
  the same string and an in-flight reply would take its verdict. Same rule in
  `ViewportRoot.promotePick`, which writes back only `elementId` and leaves `topoKey` alone.
- Only a REGEN reconciles. `meshSync.loadBody` takes a `LoadReason`; `onDocumentChanged` is
  `"regen"`, every other call site (`onColorChanged`, `onVisibilityChanged`, `applyIsolation`,
  the `reconcile()` self-heal) is `"cosmetic"`. `MeshEntry.meshRev` bumps on EVERY load, so it
  cannot distinguish the two — that is why the reason is threaded rather than derived. A first
  load has no `prev` and reconciles nothing.
- `reconcileSelectionForBody` also `clearPickProof`s every ref of the regenerated body — the
  pick-time promotion proof is evidence about a publication that is gone. See
  [[repo-promotion-proof]].
- `meshSync.dropBody` calls `dropSelectionForBody`: a removed body's face/edge refs can never
  draw or author again.
- `clearConsumedSelection` runs for fillet/chamfer and shell ONLY. The HOLE call was removed:
  a hole perforates its seat rather than consuming it, and the reviewer's four-holes-on-one-face
  flow needs the pick to survive.
- Every authoring lane sends the persistent handle when the ref has one, because a reconciled
  ref's `topoKey` may be an `el_` label: `prepareEdgeOp` / `prepareOffsetFace` send `elementId`
  and omit `topoKey` (`edge_op_bodies` requires EXACTLY one address), `projectToSketch` sends
  both and Rust skips promotion for an elementId-bearing source, `placeComponent` already
  short-circuits on `targetElementId`.
