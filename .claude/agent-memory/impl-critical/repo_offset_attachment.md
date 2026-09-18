---
name: repo-offset-attachment
description: H10 (2026-09-17) — every OffsetFace distance type drags, the inner-Radius sign rule, the two-segment witness, the shared re-edit ladder, and the test doubles that break on them
metadata:
  type: project
---

`Total`, `Radius` and `Diameter` are no longer typed-only. `offsetFaceStep`'s `grab`
and `drag` no longer gate on `distanceType === "Offset"`; what the reducer owes an
absolute type is its DOMAIN, so `drag` refuses an out-of-domain frame whole (effect
`none`, `valueError` untouched) instead of clamping.

**Why:** `docs/design/astra/modeling-handle-attachment.md` §5. SCHEMA §7.3 already
fixes the reference each absolute dimension is read against (`d = σ(distance − R)`),
so "no zero to drag from" was a misframing — an absolute dimension needs a starting
VALUE, which is what `H(q)` is seated at.

**How to apply:**

- `src/tools/preview/faceOffset.ts` owns the paths: `planarOffsetPath`,
  `cylindricalOffsetPath` (`C + (R0 + σq)·r̂`), `radialDimensionPath`
  (`C + R·r̂` / `C + (D/2)·r̂`), `totalThicknessPath` (`B = P − t0·n`),
  `orientTowardReference` (γ₃-relative sign certification), plus
  `offsetTargetWitness` / `radialDimensionWitness` / `totalThicknessWitnesses`.
- **Increasing Radius/Diameter moves the wall OUTWARD on an inner wall too.** A 6 mm
  bore opened to 8 moves `+2·r̂` while material-outward is `−r̂`; driving it by the
  material normal inverts the drag (§7). σ belongs ONLY in the signed-`Offset`
  derivative. `dPointDValue` keeps Diameter's ½ un-normalized — that halves `Cq` and
  is why a Ø drag hands over to the proxy earlier (0.1598/0.1602 radial ⇒
  0.0799/0.0801 scalar ⇒ proxy/world).
- `ViewportEngine.showValueWitness` takes ONE witness or an ARRAY;
  `ValueWitnessLayer` keeps a line+label per segment (`__value_witness`,
  `__value_witness_1`, …) and RETIRES the extras when a shorter list arrives. `Total`
  draws two: `P↔B` is `measuredReference` only because `t0` is
  `PrepareOffsetFace.currentDims.thickness`, and `B↔H(T)` is always a target.
- `resolveOffsetAttachment(gen, distanceType, evidence)` takes EXPLICIT evidence, so
  the fresh arm (`armedOffsetEvidence`) and the re-edit (`storedOffsetEvidence`) share
  one ladder. A re-edit has no handshake and therefore no `currentDims` ⇒ **no Total
  path, ever** — it keeps a labelled control. `offsetSurfaceSample()` is never a
  classified plane's `origin` (that can sit far from the finite face).
- Witness share-counts come from `offsetPathFaceCount` / `edgeOpSharedCount`, NOT from
  `offsetFaces` / `filletEdges` — both are empty on a re-edit, which silently labelled
  a four-edge chain as one edge and a multi-face closure as a single face's target.
- The L1 ghost (`ghostOffsets`) is now `Offset`-ONLY: it translates each face by
  `distance`, which is a displacement only for a signed offset.
- Anchor ladders go through `resolveLadder` + `bodyCentre`; `[0,0,0]` means `none`,
  which HIDES the handle. The status lines are derived from the rung
  (`offsetControlPhrase`, `edgeOpControlVerb`): "drag the arrow" / "drag vertically" /
  "type a value". `OFFSET_REEDIT_HINT` is gone — it is `offsetReeditHint()` now.

**Test-harness traps:**

- ANY client double that arms `offsetFace` (or edits one) needs `classifyElement`. The
  method is called unconditionally, so an undefined one throws SYNCHRONOUSLY and the
  trailing `.catch(() => null)` does not cover it — the arm dies as an unhandled
  rejection and the tool stays `idle`.
- `bodyCentre` reads `view.bboxMin`/`bboxMax`; a partial `getEntry` double (`view:
  { faceRanges }`) has neither, and it refuses rather than throwing.
- `makeBoxMesh` emits TOPOKEY id tables. To exercise a re-edit's ElementId rung
  (`IDS_HAVE_ELEMENTIDS`), spread the parsed view and replace
  `idsHaveElementIds` + `edgeIdChars`/`edgeIdOffsets` (or the face pair) before
  `buildBodyObjects`. A stored SemanticRef carries an ElementId and no TopoKey, so
  without that patch every edge-op re-edit legitimately falls to the body rung.
- `makeBoxMesh()` default origin is `[0,0,0]`, so a "not the origin" assertion needs
  the offset box `makeBoxMesh(40, 40, 40, 0, [20, 20, 20])` (centre `[20,20,20]`).
- A `Total` switch with no `oppositeFace` in the handshake is REFUSED by
  `prepareOffsetClosure`, and the refusal RETAINS the previous arm's handle — do not
  write "the old handle went away" against that path.
