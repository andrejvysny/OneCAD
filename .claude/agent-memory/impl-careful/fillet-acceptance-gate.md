---
name: fillet-acceptance-gate
description: Measured behaviour of the shipped fillet blend-acceptance gate (1e-9 relative section radius / 1e-9 rad tangency) and how to probe it out of tree
metadata:
  type: project
---

The Fillet acceptance path is `kernel::fillet::FilletBuilder::build` -> `accept_result` -> `validate_result` (`worker/src/kernel/fillet/FilletSemanticChecks.cpp:85,89`); `ops::execute_fillet` just wraps it (`worker/src/ops/FilletChamferOp.cpp:189-195`). The gate constants are `kTangencyRadians`/`kSectionRelative` = 1e-9 with a 1e-14 conditioning term (`worker/src/kernel/fillet/BlendEvidence.cpp:33-36`), sampled at a FROZEN budget of 9 edge samples / 5x5 uv grid (`BlendEvidence.h:42-43`).

Measured 2026-09-04 against OCCT 8.0.1: analytic (KPart) blends — box edges, cone-on-box base rims, cylinder top circles — land at 1e-16 residual and pass. Walked/approximated blends fail by ~1e6x: cylinder-cylinder T-junction (r=2) gives section residual 1.97e-3 against an allowance of 2.0e-9; an oblique elliptical rim (r=1) gives 2.87e-3 against 1.0e-9. Bare `BRepFilletAPI_MakeFillet` succeeds on both, so those are OUR refusals, not OCCT failures. A cylinder seam edge is refused by OCCT itself (`Contour()` returns 0).

**Why:** the gate was written for analytic blends and nobody had measured what it does to walked ones; the refusal is `GEOMETRY_INVALID` / `FILLET_SEMANTIC_CHECK_FAILED`, which the user sees as a broken fillet.

**How to apply:** any change to the fillet tolerance policy must keep the analytic cases at 1e-16 while admitting walked blends around 1e-3 relative — a single scalar loosening to 1e-2 would also admit genuinely wrong geometry, so prefer classifying the blend (analytic vs walked) over widening one constant. Two traps when probing: pick edges by geometry, never ordinal, and reject coplanar fuse seams (dihedral ~0) or the probe measures nothing; and the resolution ladder can return `NeedsRepair` on the full op path for an edge `FilletBuilder` accepts, so measure the builder directly to isolate acceptance from binding.
