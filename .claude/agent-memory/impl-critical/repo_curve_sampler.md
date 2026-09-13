---
name: repo-curve-sampler
description: OneCAD worker edge sampling and MESH1 edge tables — the traps that cost time in WP08 (OCCT conversion behaviour, section codes, gp_XYZ comparison, build layout)
metadata:
  type: project
---

Facts learned building the WP08 exact-span edge sampler (`worker/src/tess/CurveSampler.*`).

**Why:** each of these cost a build/run cycle to discover and none is stated in CLAUDE.md.

**How to apply:** read before touching worker tessellation, MESH1 edge tables, or OCCT curve conversion.

- MESH1 section codes: `EDGE_RANGES` is **7**, `EDGE_POSITIONS` is **8** (`worker/src/tess/Mesh1.cpp`,
  `protocol/mesh_format.md` §4). `worker/tests/test_tessellation_quality.cpp`'s
  `greatest_edge_point_count` reads type **8** and calls it `kEdgeRanges`, i.e. it interprets f32
  bit patterns as u32 point counts and returns 0 whenever `12*P % 8 != 0`. Its two edge assertions
  are therefore accidental, not meaningful — do not treat that target as edge-count coverage.
- `gp_XYZ::IsEqual(other, tol)` compares with strict `<`, so `tol = 0.0` is ALWAYS false even for
  bit-identical values. For an exact-equality assertion use `gp_Pnt(a).Distance(gp_Pnt(b)) == 0.0`.
- `BRepAdaptor_Curve`: `GeomCurve()` returns the UNTRANSFORMED basis curve (`GeomAdaptor_Curve`
  already strips `Geom_TrimmedCurve`), `Trsf()` is the edge location, `Is3DCurve()` is false for a
  curve-on-surface edge, and `First/LastParameter()` are orientation-independent — `e` and
  `e.Reversed()` sample to the identical point sequence.
- `GeomConvert_ApproxCurve(Handle(Adaptor3d_Curve), ...)` returns a B-spline in WORLD coordinates
  (the adaptor applies the location) and PRESERVES the edge's parameter domain — verified by an
  explicit domain check in `approximate_span_set`, which has never fired.
- `GeomConvert_BSplineCurveToBezierCurve(bs, U1, U2, tol)`'s `Knots()` really does return
  `NbArcs()+1` strictly increasing clipped knots, so leaf intervals land exactly on knots.
- `Geom_OffsetCurve` throws `Standard_ConstructionError: Offset on C0 curve` at construction — an
  offset fixture needs a C1+ basis.
- `worker/build` on this machine is configured with **Unix Makefiles**, not Ninja: the `-G Ninja`
  line in CLAUDE.md fails against the existing cache. Test binaries land in `worker/build/tests/`,
  not `worker/build/`.
- To link a scratch probe against `worker/build/libworker_core.a` by hand you must also set
  `DYLD_LIBRARY_PATH=$HOME/.onecad-occt/8.0.1/lib` (no rpath on a hand-linked binary; ctest-built
  targets already carry one).
- Fine-tier linear deflection is bbox-relative (`diag * 0.0005`, clamped to `[0.0001, 0.05]`), so a
  small edge gets a very tight budget: the S-curve edge lands at 0.00161 mm, not 0.05 mm. The
  float32 allowance competes with that budget, so a far-from-origin edge is how you reach the
  quality-limited path without a pathological curve — but it must be computed PER AXIS
  (`0.5*||ulp(maxX), ulp(maxY), ulp(maxZ)||`) and MEASURED afterwards, never as a blanket
  `maxAbs * 2^-21`: x = 1e9 is exactly representable in float32 (1953125 * 2^9) so a circle there
  costs almost nothing, while the blanket bound reduced it to two endpoints.
- Edge sampling contracts that round 2 nailed down (see `worker/src/tess/CurveSampler.h`):
  `QualityLimited` with EMPTY points means a WORK cap (depth / per-edge segments / per-body
  segments) and nothing may be drawn; `QualityLimited` WITH points means the float32 output format
  cannot hold the requested accuracy and the polyline is the best it can carry, so it IS drawn.
  Only the first case runs the tolerance-doubling ladder.
- `BRepAdaptor_Curve` is orientation-blind (`BRep_Tool::Range` does not flip for a REVERSED edge),
  so an edge and its reversal sample to the identical points in the identical order. MESH1's
  EDGE_POSITIONS is undirected and no consumer reads a direction from it — do not "fix" this.
- OCCT traps that cost a red run each: `GeomConvert::CurveToBSplineCurve` on an UNTRIMMED periodic
  conic throws the edge's starting phase away (a full circle over [1, 1+2pi] then mismatches its
  own endpoint by a chord's worth); `SetNotPeriodic()` collapses a periodic B-spline to ONE period,
  so an edge range crossing the seam must be unrolled, never clamped; and
  `BRepBuilderAPI_MakeEdge(gp_Lin)` yields a range of +/-`Precision::Infinite()` (1e100), which is
  finite to `std::isfinite` — guard with `Precision::IsInfinite`.
- An angular criterion needs its OWN independent oracle: the greatest turn between consecutive
  emitted segments, measured from the published points. A position-based acceptance shortcut passed
  every chord assertion while letting 67-degree creases through the 5-degree fine tier; only the
  turn measurement caught it.
