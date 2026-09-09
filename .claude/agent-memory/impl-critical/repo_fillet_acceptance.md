---
name: repo-fillet-acceptance
description: Fillet blend classes and budgets, why the section residual is curvature-derived, the range-analyzer remnant floor, and which kernelbench rows move
metadata:
  type: project
---

The fillet acceptance envelope (SCHEMA §7.3 "Two blend classes, two budgets", WP-G).

**Why:** OCCT builds a blend either from a KPart closed form (torus/cylinder/cone —
ANALYTIC, exact to 2e-16) or by WALKING it and fitting a B-spline (APPROXIMATED). One
1e-9 gate for both refused every walked blend.

**How to apply:**

- `BlendEvidence.maximum_profile_error` is `|1/κmax − r|` — a CURVATURE quantity, a second
  derivative of the fitted surface, so it scales as `tol / h²`, NOT as `tol`. Measured on
  OCCT 8.0.1 it is 39x and 247x the blend face's `BRep_Tool::Tolerance`. Never write a
  fillet budget as a small multiple of a positional tolerance; that shape was tried and
  measured unsatisfiable.
- The acceptance constants live in `worker/src/kernel/fillet/BlendEvidence.h`:
  `kSectionRelative`/`kTangencyRadians`/`kConditioning` (exact, analytic) and
  `kRelSection`/`kSectionCeiling`/`kTangencyFactor`/`kTangencyBudgetCeiling` (adaptive).
  They are NORMATIVE SCHEMA constants — changing one needs a §14 entry with a new
  measurement.
- `fillet_section_radius_limit` / `fillet_tangency_limit` keep their exact form and must
  NOT widen: OffsetFace's blend, `BlendRecognizer`, `BlendReconstruction` consume them.
  The adaptive budget is a separate branch inside `validate_result` only.
- EXACT FIRST is the load-bearing rule: every contour is measured against the exact
  budgets whatever its representation, and only an APPROXIMATED contour that FAILS them
  falls to the adaptive budget. OCCT emits B-spline blends that are exact to 6e-16 (the
  whole `matrix.cylinder-cylinder` kernelbench family), so classifying by representation
  alone widened 92 rows' budgets and warned about exact geometry. With exact-first, t0 and
  m1 are BOTH byte-identical — a fillet-evidence change should move zero rows.
- Classification, measurement and budget are PER CONTOUR (`collect_fillet_contour_evidence`
  + `judge_contour_evidence`); an analytic contour beside an approximated one in the same
  op must never be judged by the other's budget. `validate_result` keeps an aggregate
  fast path: if the whole result passes the CONDITIONING-FREE exact budgets, every contour
  does, so exact geometry never pays for the second measurement pass.
- `approximationTolerance` is the max `BRep_Tool::Tolerance` over the blend FACES ONLY.
  Including their edges/vertices inflates it ~2x because those are shared with the support
  faces, so a coarse input would silently widen a tightly-fitted blend's budget.
- `worker/src/benchmark/SemanticValidation.cpp` hard-codes its OWN gates, independent of
  the fillet limits: `constantRadius` (1e-9), `cylindricalRadius` (1e-8),
  `g1BoundaryTangency` (1e-7) and the two recipe-agnostic `section_limit`/`tangency_limit`
  (1e-9). Four of those go through `effective_limit`, which applies the same exact-first
  rule so an exact B-spline blend still reports the exact limit; `constantRadius`
  deliberately does NOT — it compares `builder.Radius(contour)` to the requested radius, a
  LAW comparison that is exact for both classes.
- `FilletRangeAnalyzer` remnant floor: the scan MUST cover only the faces the probe
  generated or modified (`touched_faces`). Scanning the whole result makes a body that
  already carries a sub-resolution edge — an imported sliver, a boolean leftover 25 mm
  away — fail EVERY probe, and the verb reports `confidence: none` on geometry whose range
  is fine (measured: 0 successes vs 61 after the fix). A probe that BUILDS and passes
  Tier B but leaves a face under `res²` or an edge under `res` (1e-3 mm) is a `Refusal`,
  not a new enum value
  — adding a fourth `ProbeClassification` would change the §7.6 wire strings. This moved
  every pinned bound in `test_fillet_range.cpp` down one bisection step (9.99925 → 9.9985
  etc.) because the search refines only to within `authoring_resolution()` of the frontier,
  so the last success always left a ~7.5e-4 mm land.
- kernelbench blast radius of a fillet-evidence change: `fillet/matrix:m1`'s
  `matrix.cylinder-cylinder.*` rows are the APPROXIMATED family (92 of 94 publish);
  `plane-plane` / `plane-cylinder` / `plane-cone` are analytic and must stay byte-identical.
  `fillet/foundation:t0` (136 rows) must not move at all. Compare against
  `bench/robustness/baselines/digests.json` — that file IS the "before", so a before-run is
  not needed.
