// REWRITE (algorithm parity, NOT a line port) of OneCAD-CPP
// src/core/sketch/FaceBoundaryProjector.{h,cpp} @ b4ddcccc (frozen oracle).
//
// The oracle MUTATES a `Sketch` (addPoint/addLine/addCircle/addArc + Fixed
// constraints + reference-lock flags) and derives 2D through `sketch.toSketch()`.
// This one is a PURE function over a local buffer: no Sketch, no ids, no
// constraints. Rust owns identity — it mints EntityIds for the returned points and
// curves and authors the `Fixed` rows (SKETCH-ON-FACE decision D1/D3').
//
// Kept VERBATIM from the oracle (each cited against `FaceBoundaryProjector.cpp`):
//   * `GeomAbs_Line` → an exact segment from the curve's natural first/last
//     parameter (:306-318).
//   * `GeomAbs_Circle` → the full-circle vs arc split, `edge.Closed() || span≈2π`
//     (:319-349).
//   * arc CCW from the D1 tangent at mid-parameter crossed with the radial vector,
//     both expressed in the plane's UV basis (:374-397).
//   * the degenerate-sweep fallback: |cross| ≤ 1e-9, sweep ≤ 1e-9, or sweep ≈ 2π
//     ⇒ drop to the polyline branch (:392-393, :406-410).
//   * the `hasClosedBoundary` law (:464-470) — a circle closes its wire outright,
//     otherwise ≥3 curves AND (the wire carries OCCT's closed flag OR the walk's
//     first and last point merged onto the same buffer slot).
//   * the else-branch polyline sampling at `fallbackSegmentsPerCurve` (:247-263,
//     :445-459), and the point-merge / duplicate-curve suppression the oracle's
//     findOrCreatePoint / lineExists / circleExists / arcExists provide (:82-205).
//
// DEVIATIONS (each deliberate, all documented in SCHEMA §7.6):
//   * pure over a local buffer (above) — so `Options::preferExactPrimitives` and
//     `lockInsertedBoundaryPoints` are gone: exact primitives are unconditional and
//     locking/pinning is Rust's job.
//   * MULTI-FACE: the oracle projects one face. This walks the seed face and then
//     the caller-supplied coplanar faces, emitting each `TopoDS_Edge` ONCE
//     (`IsSame`) so an interior edge shared by two coplanar faces does not double.
//   * ORDERING is normative here (the oracle left it to `TopExp_Explorer`): seed
//     face first, its OUTER wire first, then its holes; then the coplanar faces in
//     the order supplied; edges in `BRepTools_WireExplorer` order; points numbered
//     by first use. Byte-identical output across fresh runs depends on it.
//   * degenerate edges are skipped before `BRepAdaptor_Curve` is constructed. A
//     planar face carries none, so this cannot change any output — it just refuses
//     to construct an adaptor over an edge with no 3D curve.
#ifndef ONECAD_WORKER_SKETCH_FACEBOUNDARYPROJECTOR_H
#define ONECAD_WORKER_SKETCH_FACEBOUNDARYPROJECTOR_H

#include "sketch/Sketch.h"  // SketchPlane (origin/xAxis/yAxis/normal + toSketch)

#include <TopoDS_Face.hxx>

#include <string>
#include <vector>

namespace onecad::core::sketch {

class FaceBoundaryProjector {
public:
    struct Options {
        /// Two boundary points closer than this merge onto one buffer slot.
        double pointMergeTolerance = 1e-5;
        /// Radius agreement for the duplicate-circle/arc suppression (oracle default;
        /// deliberately NOT wire-configurable — it is a dedup detail, not a policy).
        double radiusTolerance = 1e-5;
        /// Segments emitted per unsupported curve by the polyline fallback.
        int fallbackSegmentsPerCurve = 24;
    };

    enum class EntityKind { Line, Circle, Arc };

    /// A boundary point in the SUPPLIED plane's UV. Its index IS its response ref.
    struct Point {
        double u = 0.0;
        double v = 0.0;
    };

    /// One emitted curve. Point fields are indices into `Result::points`.
    struct Entity {
        EntityKind kind = EntityKind::Line;
        int p0 = -1;      ///< Line: start point
        int p1 = -1;      ///< Line: end point
        int center = -1;  ///< Circle / Arc: center point
        double radius = 0.0;
        double startAngle = 0.0;  ///< Arc: radians CCW from the plane's +U axis
        double endAngle = 0.0;    ///< Arc: always the CCW-ordered partner of startAngle
        bool ccw = true;          ///< Arc: the UNDERLYING curve's direction (D1 rule)
    };

    struct Result {
        bool ok = false;
        std::string errorMessage;
        bool hasClosedBoundary = false;
        std::vector<Point> points;
        std::vector<Entity> entities;
    };

    /**
     * @brief Project a planar face's boundary (plus any coplanar companions) into
     *        the UV of `plane`.
     *
     * @param seedFace       the picked face. MUST be planar — a non-planar seed is
     *                       refused (`ok=false`), never approximated.
     * @param coplanarFaces  additional faces whose edges join the projection. Faces
     *                       that are null, or `IsSame` as the seed, or repeated, are
     *                       dropped. Pass empty for a seed-face-only projection.
     * @param plane          AUTHORITATIVE. Every `at`/center is expressed in this
     *                       basis; the projector never substitutes one of its own.
     */
    static Result project(const TopoDS_Face& seedFace,
                          const std::vector<TopoDS_Face>& coplanarFaces,
                          const SketchPlane& plane,
                          const Options& options);
};

}  // namespace onecad::core::sketch

#endif  // ONECAD_WORKER_SKETCH_FACEBOUNDARYPROJECTOR_H
