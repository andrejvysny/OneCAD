---
name: repo-cut-effect-policy
description: Cut no-effect policy — the three floors in εV (semantic, measurement, representation), where it runs, and which tests changed verdict
metadata:
  type: project
---

`ops::cut_effect_policy` (`worker/src/ops/OpCommon.cpp`) refuses a `Cut` that cannot
demonstrate material removal.

**Why:** a Cut that removed exactly zero used to publish a body byte-identical to its
target and report success.

**How to apply:**

- `εV = a³ + U + κ·ulp(max(|V_before|,|V_after|))`, `κ = 8`. Three INDEPENDENT floors,
  summed, each bounding a different way `ΔV` can be nonzero with nothing removed:
  `a³ = minimum_volume()` (semantic, absolute, 1e−9 mm³, does not scale);
  `U = Σ e·|V̂|/(1−e)` from the relative error `BRepGProp::VolumeProperties` RETURNS
  (it returns the achieved error — that is what makes U measurable per call);
  and the ULP term, because binary64 resolution DOES scale — at V = 1e9 mm³ (a 1 m
  cube) spacing is 2^(29−52) ≈ 1.19e−7 mm³, a hundred times `a³`, and an imprint
  changes the topology so the two integrations run over different face sets.
  `e ≥ 1` bounds nothing ⇒ `CUT_VOLUME_UNMEASURABLE`.
- Measured on OCCT 8.0.1/arm64: a 100×60×25 box reports relErr 1.36e-16 and a 1 m
  cube relErr 0 with ΔV exactly 0 on an imprint — so the ULP floor is pinned through
  the `toleranceMm3` evidence, not through an observed ULP drift.
- It runs AFTER publication is otherwise decided, on the target/result pair, and is
  scoped to `BooleanMode::Cut` only.
- Behaviour changes it caused: `test_revolve_boolean_modes.cpp`'s `revolve-cut-miss`
  now asserts the refusal, and `test_preview_op.cpp`'s Cut fixture had to move onto a
  profile that genuinely overlaps its target.
