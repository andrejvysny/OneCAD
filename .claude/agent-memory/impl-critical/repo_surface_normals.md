---
name: repo-surface-normals
description: OneCAD worker face tessellation and normals — OCCT frame/determinant/continuity traps learned in WP09, and what tessellate_body now mutates and drops
metadata:
  type: project
---

Facts learned building the WP09 surface-normal + completeness pass
(`worker/src/tess/SurfaceNormals.*`, `EdgeClassification.*`, the face half of `Tessellate.cpp`).

**Why:** every one of these cost a build/run cycle or would have shipped a silent wrong answer,
and none is in CLAUDE.md.

**How to apply:** read before touching worker face tessellation, shading normals, edge
classification, or anything that reads a `TopLoc_Location`'s handedness.

- `BRepAdaptor_Surface` APPLIES the face location itself — its own header says "It takes into
  account the local coordinates system" and "Value, D0, D1 ... apply the transformation
  automatically". So `D1` gives WORLD derivatives and a second BRep location transform is a
  double application. Consequence: `S_u^w x S_v^w = det(A) * A^-T (S_u x S_v)_local`, i.e. it FLIPS
  under a reflection, so a world cross product needs a `sign(det A)` factor to equal NUM §7.1's
  `A^-T n_local`. For a `gp_Trsf` the linear part is always `s*M` with `M` orthogonal, hence
  `A^-T = (1/s^2) A` — a POSITIVE multiple — so `A^-T n` and `A n` are always the same direction.
- **`gp_Trsf::IsNegative()` is `scale < 0.0`, not the determinant** (`gp_Trsf.hxx:218`). Use
  `trsf.VectorialPart().Determinant() < 0.0` for NUM §7.2's `det(A) < 0`.
- `BRepBuilderAPI_Transform(shape, mirror, /*copyGeom=*/false)` does NOT keep the mirror as a
  location: the doc requires "direct and isometric (determinant = 1)" for that, so a reflection is
  BAKED into duplicated geometry and OCCT flips the face orientations itself. To get a shape that
  really carries a negative-determinant location, use `shape.Moved(TopLoc_Location(mirror))`. Both
  representations must produce outward normals — test outwardness, never a flip count.
- `BRep_Tool::IsClosed(const TopoDS_Shape&)` only measures free boundaries for a SHELL, WIRE or
  EDGE; "for other shape types returns S.Closed()", and `BRepPrimAPI_MakeBox` leaves that flag
  clear on its solid. To verify a solid is closed, explore its SHELLs and `IsClosed` each one.
- `BRep_Tool::Continuity(edge, f1, f2)` returns `GeomAbs_C0` when NO continuity is registered, so
  C0 is the ABSENCE of metadata, never evidence of a crease. `BRepFilletAPI_MakeFillet` DOES
  register G1: an all-edge-filleted 40x30x20 box at r=4 has 56 edges — 48 tangent, 8 degenerate
  (the corner blends' pole edges), 0 hard, 0 unknown.
- The four-argument `BRepMesh_IncrementalMesh` constructor already calls `Perform()` ("Automatically
  calls method Perform" in the header). The extra `mesher.Perform()` was measured byte-identical
  across the F08 gallery at coarse/medium/fine and is gone from `tessellate_body`; `tessellate_raw`
  still has it.
- `tessellate_body` MUTATES the caller's geometry: `Session::bodies_copy()` is a value copy whose
  `TopoDS_Shape`s are handle copies, and OCCT caches `Poly_Triangulation` on the TShape. A display
  job therefore replaces the triangulation the document and the STL/OBJ exporters see. Nothing
  races only because every geometry verb runs on the single kernel lane thread (`Dispatcher` has
  kernel/solver/status threads; only the five sketch-solver verbs go to the solver lane).
- `tessellate_body` now DROPS zero-area triangles while `tessellate_raw` (STL/OBJ) keeps them, so
  the two triangle counts intentionally differ: sphere 434 vs 436, cone 102 vs 103, all-edge
  filleted box 620 vs 628. Do not "fix" that by making them agree.
- A `TopoDS_Face` built by `BRep_Builder::MakeFace(f, surface, tol)` with NO wire is a reliable
  fixture for "a nondegenerate face the mesher covers with nothing": BRepMesh leaves it null, and
  `BRepGProp::SurfaceProperties` still reports a real area so it is correctly diagnosed as missing
  rather than excused as degenerate.
- `BRepPrimAPI_MakeBox` with 1e-9 mm sides throws `Standard_DomainError`; use 1e-3 mm against a
  large reference length when you need a relatively-degenerate face.
- Building a node->incident-triangle map as `vector<vector<uint32_t>>` is most of the cost of a
  per-node pass (21k heap allocations on one fine torus face): a CSR pair of offsets+items took
  `compute_face_normals` from 12.65x to 5.40x the retired area-weighted loop.
- Baseline red set for the whole worker suite at this tree: `feature_pattern` and
  `chamfer_reference_face` (both resolution-ladder, nothing to do with tessellation). A full run is
  ~55-65 s / 199 targets, so "no full ctest" is a lease-politeness rule, not a cost one.

Round 2 (the Astra `break`, `docs/design/astra/wp09-surface-normals-break.md`) added:

- `BRepAdaptor_Surface::BSpline()`/`Bezier()`/`Sphere()` return the TRANSFORMED geometry, not the
  basis: on a face `Moved` 1000 mm in X, `BSpline()->Pole(1,1)` comes back at x = 1000. So a
  pole-magnitude bound is already in world units and must NOT be scaled by the location again.
  (This is the opposite of `BRepAdaptor_Curve::GeomCurve()`, which IS untransformed.)
- `BRepTools::UVBounds` on a face with NO wire returns the surface's whole natural domain
  ([0,1]x[0,1] for a Bezier), not an empty box — so "no trim description" is a separate branch from
  "zero-measure retained domain", and treating it as a certificate would excuse the standard
  wireless-face fixture. `Bnd_Box2d::Get` THROWS on a void box, so a face whose pcurves are missing
  surfaces as an exception rather than an inverted box; guard on `u1 >= u0 && v1 >= v0` anyway.
- `BRepAlgoAPI_Fuse` of two r = 0.1 mm spheres whose centres are 0.0001 mm apart really does build:
  5 two-face edges, 3 of them between two faces carved from the SAME `Geom_SphericalSurface` (those
  are legitimately tangent) and 2 genuinely cross-surface. Any "distinct surfaces are never tangent"
  assertion must filter on surface-handle identity or it fails on the same-surface edges.
- A synthetic `Poly_Triangulation` handed straight to `compute_face_normals` is the cheapest way to
  test a node policy: `Poly_Triangulation(nbNodes, nbTriangles, hasUVNodes, false)` + `SetNode` /
  `SetUVNode` / `SetTriangle`, positions from `BRepAdaptor_Surface::Value(u,v)` (already world), and
  `TopLoc_Location()` passed as `loc` so nothing is transformed twice. It is how the cone-apex fan,
  the valence-10000 fan, the reversed pole facets and the cyclic-permutation sliver are all tested.
  A triangulation built WITHOUT UV nodes is how you force the triangle-only ladder.
- `BRepPrimAPI_MakeCone(R, 0, H)`'s apex sits at `v = -RefRadius/sin(SemiAngle)` in the adaptor's
  parameterization, and `|S_u|` there is exactly the local radius, so `|S_u| <= derivative budget`
  is a sound apex test that needs no separate distance-to-apex comparison.
- Cost on this machine, same harness, fine torus face (21 025 nodes, one `compute_face_normals`
  call): retired 1.14-1.17 ms, post-break 1.36-1.39 ms (+19%, +10 ns/node) — the extra is the
  symmetric triangle predicate (3 edge lengths instead of 2 magnitudes) plus one float32 round-trip
  per node. A valence-10 000 singular fan went 160 ms -> 0.70 ms once the pairwise spread scan is
  skipped above `kMaxSpreadValence = 64`.
- `src-tauri/target/` in this worktree still holds absolute paths from the PRE-RENAME worktree
  (`.../OneCAD-ai-agent/...`), so any `cargo test` that compiles the app crate dies in tauri's build
  script with "failed to read plugin permissions". The Rust worker-backed gates need that target dir
  rebuilt before they can run here; the CMake cache had the same problem and was fixed earlier.
