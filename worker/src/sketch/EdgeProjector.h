// EdgeProjector.h — the curve rules behind `ProjectToSketchPlane` (SCHEMA §7.6,
// WP-P: project body edges into the active sketch).
//
// WHY THIS IS NOT `FaceBoundaryProjector`. That projector answers "this face's
// own boundary, seen from its own plane", so a circular edge always lies in a
// plane PARALLEL to the sketch plane and `circle.Radius()` is the projected
// radius verbatim. WP-P projects an ARBITRARY body edge into an ARBITRARY sketch
// plane, where that assumption is false: a tilted circle projects to an ELLIPSE
// and an edge-on circle projects to a LINE. Reusing `emitCircularEdge` would
// emit a full-radius Circle for both — geometry the model does not contain,
// which then BOUNDS A REGION (reference-locked entities are full participants in
// loop detection). So the circular-edge branch is rewritten here against the
// two normals, and only the buffer discipline is shared in spirit:
//
//   * point MERGE within `pointMergeTolerance` (`findOrCreatePoint`, oracle
//     :85-110 via FaceBoundaryProjector.cpp:88-96) — the merge is what lets a
//     projected outline close into a region;
//   * the position-based duplicate suppression for Line/Circle/Arc
//     (FaceBoundaryProjector.cpp:99-151), here scoped PER SOURCE (below);
//   * the arc CCW rule: the D1 tangent at mid-parameter crossed with the radial
//     vector, both in the plane's UV basis (FaceBoundaryProjector.cpp:261-286);
//   * the wire order for `faceOutline`: OUTER wire first, then holes, edges in
//     `BRepTools_WireExplorer` order (FaceBoundaryProjector.cpp:317-349).
//
// Consequently, for a face COPLANAR with the supplied plane, `faceOutline`
// produces the identical entity set and point numbering `ProjectFaceBoundary`
// `scope:"faceOnly"` produces — the parallel branch below IS that projector's
// branch, gated by the parallel test. SCHEMA §7.6 makes that equality normative
// and `protocol/fixtures/project_to_sketch_plane.ndjson` round L pins it by
// running both verbs over the same face and plane in one file.
//
// DEDUP IS SCOPED PER SOURCE, deliberately, and that is the one place the two
// projectors differ by construction. `FaceBoundaryProjector` walks one seed plus
// its coplanar companions as a single unit and dedups over the whole walk;
// WP-P answers a BATCH in which every source must get its own answer, so a
// second source that happens to project onto a curve source one already emitted
// still emits — otherwise a source would vanish from `entities[]` with no
// refusal to explain it. With one face in one source the two scopes coincide,
// which is why round L holds.
//
// Identity: nothing here mints anything. Point "refs" are buffer INDICES and are
// response-local (`p<N>`); Rust mints the sketch EntityIds and authors the
// `Fixed` rows, exactly as for `ProjectFaceBoundary`.
#ifndef ONECAD_WORKER_SKETCH_EDGEPROJECTOR_H
#define ONECAD_WORKER_SKETCH_EDGEPROJECTOR_H

#include "sketch/Sketch.h"  // SketchPlane (origin/xAxis/yAxis/normal + toSketch)

#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>

#include <cstddef>
#include <string>
#include <vector>

namespace onecad::core::sketch {

class EdgeProjector {
public:
    struct Options {
        /// Two projected points closer than this merge onto one buffer slot, and
        /// the edge-on circle test `r*|n_c . n_s| <= this` is taken against it.
        double pointMergeTolerance = 1e-5;
        /// The plane-parallel circle test `r*(1 - |n_c . n_s|) <= this`, and the
        /// radius agreement of the duplicate-circle/arc suppression.
        double radiusTolerance = 1e-5;
    };

    enum class EntityKind { Line, Circle, Arc, Ellipse };

    /// The closed refusal set of SCHEMA §7.6. Refusals are per-source ANSWERS
    /// (`ok:true`), never whole-call errors.
    enum class RefusalCode {
        None,
        Absent,
        KindMismatch,
        UnsupportedCurve,
        TrimmedTiltedArc,
        Degenerate,
        FaceNotPlanar,
    };

    /// camelCase wire token for `refusals[].code`; "" for `None`.
    static const char* refusalToken(RefusalCode code);

    struct Refusal {
        RefusalCode code = RefusalCode::None;
        std::string message;

        bool refused() const { return code != RefusalCode::None; }
    };

    /// A projected point in the SUPPLIED plane's UV. Its index IS its `p<N>` ref.
    struct Point {
        double u = 0.0;
        double v = 0.0;
    };

    /// One emitted curve. Point fields are indices into `Buffer::points`.
    struct Entity {
        EntityKind kind = EntityKind::Line;
        int p0 = -1;      ///< Line: start point
        int p1 = -1;      ///< Line: end point
        int center = -1;  ///< Circle / Arc / Ellipse: center point
        double radius = 0.0;
        double startAngle = 0.0;  ///< Arc: radians CCW from the plane's +U axis
        double endAngle = 0.0;    ///< Arc: the CCW-ordered partner of startAngle
        bool ccw = true;          ///< Arc: the UNDERLYING kernel curve's direction
        double majorR = 0.0;      ///< Ellipse: semi-major, post-normalization
        double minorR = 0.0;      ///< Ellipse: semi-minor, post-normalization
        double rotation = 0.0;    ///< Ellipse: major-axis angle from +U (radians)
        /// Index into the request's `sources[]` that produced this entity.
        std::size_t sourceIndex = 0;
    };

    /// The response-local buffer shared by every source of one request.
    struct Buffer {
        std::vector<Point> points;
        std::vector<Entity> entities;
    };

    /**
     * @brief Project one body edge into `plane`'s UV, appending to `buffer`.
     *
     * On a refusal the buffer is left UNTOUCHED — a refused source contributes
     * neither an entity nor a point, so `points[]` numbering never records a
     * source that produced nothing.
     */
    static Refusal projectEdge(Buffer& buffer, const TopoDS_Edge& edge, const SketchPlane& plane,
                               const Options& options, std::size_t sourceIndex);

    /**
     * @brief Project every boundary edge of a PLANAR face — outer wire first,
     *        then holes — into `plane`'s UV, appending to `buffer`.
     *
     * The face outline is a UNIT: if any of its edges refuses, the whole source
     * refuses with that code and the buffer is left untouched. A half-projected
     * outline would not close, and non-closing reference-locked geometry that
     * LOOKS like an outline is worse than a named refusal.
     */
    static Refusal projectFaceOutline(Buffer& buffer, const TopoDS_Face& face,
                                      const SketchPlane& plane, const Options& options,
                                      std::size_t sourceIndex);

    /**
     * @brief `projectedHash` (SCHEMA §7.6): FNV-1a 64-bit over the entity type
     *        token then that type's shape scalars, each `llround(v / 1e-6)` as
     *        i64 little-endian — the `quantizationVersion = 1` grid.
     *
     * Covers the projected UV geometry ONLY: not the source, not the point refs,
     * not the emission order. Two entities describing the same 2D curve hash
     * identically, deliberately.
     */
    static std::string projectedHash(const Buffer& buffer, const Entity& entity);
};

}  // namespace onecad::core::sketch

#endif  // ONECAD_WORKER_SKETCH_EDGEPROJECTOR_H
