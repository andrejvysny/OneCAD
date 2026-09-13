// SurfaceNormals.h — per-node shading normals evaluated from the SUPPORTING
// SURFACE (VP-HARDENING WP09, requirement VP10, findings R04/R11). Design:
// docs/viewport-hardening/03-NUMERICS-AND-PROTOCOL.md §7 (binding).
//
// WHY this exists: the retired path derived every shading normal from the
// TRIANGLE MESH — an area-weighted sum of the incident facet normals, with a
// silent world +Z whenever that sum collapsed. Three consequences, all measured
// in `docs/qa/viewport-hardening/baseline/ctest-surface-normals-red.log`:
//
//   * a facet average is only as good as the facet density, so a fine cylinder
//     seam node was 2.5 deg off radial and a coarse sphere 10.3 deg — the shading
//     of a curved surface tracked the mesher's angular deflection instead of the
//     geometry (NUM §7.4 asks for 0.1 deg);
//   * a node whose incident facets do not surround it (a periodic seam, the row
//     next to a tangent blend boundary) is biased by half the local turn, so two
//     copies of the SAME seam point disagreed by a whole facet step;
//   * a reflected placement inverted every emitted triangle and every normal
//     with it — the winding rule looked only at the face orientation flag and
//     never at the sign of the location's determinant.
//
// The contract implemented here (NUM §7.1/§7.2):
//
//   n_world = orientationSign * sign(det A) * normalize(S_u^w x S_v^w)
//   reverseWinding = faceIsReversed XOR (det A < 0)
//
// where `S_u^w`/`S_v^w` are the derivatives `BRepAdaptor_Surface` returns. That
// adaptor applies the face's location EXACTLY ONCE for us ("It takes into
// account the local coordinates system... Value, D0, D1 ... apply the
// transformation automatically" — BRepAdaptor_Surface.hxx), so no second BRep
// location transform may be applied on top of it. The `sign(det A)` factor is
// the algebraic equal of NUM §7.1's inverse-transpose form, not an extra flip:
//
//   S_u^w x S_v^w = det(A) * A^-T (S_u x S_v)_local
//
// and a gp_Trsf's linear part is always `s * M` with `M` orthogonal, so
// `A^-T = (1/s^2) A` — a POSITIVE multiple of `A`. Multiplying the world cross
// product by `sign(det A)` therefore yields exactly NUM's `A^-T n_local`
// direction, while the winding rule keeps the separate determinant flip that a
// reflection needs. Applying the sign to only one of the two is what produces an
// inside-out mirrored solid.
//
// Singularities follow NUM §7.3 and are never answered with world +Z.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include <Poly_Triangulation.hxx>
#include <TopLoc_Location.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Pnt.hxx>

namespace onecad::tess {

// Where one emitted vertex's normal came from (NUM §7.3). Published on the wire
// only as the QUALITY_INFO "some normal fallback" bit (WP10); here it is the
// per-vertex evidence the focused tests assert on.
enum class NormalProvenance : std::uint8_t {
    Surface,                // S_u x S_v on the supporting surface, regular node
    AnalyticSingular,       // analytic limit at a known singularity (sphere pole)
    TriangulationFallback,  // singular node, incident facets agree within 5 deg
    SingularSplit,          // singular node split per incident triangle
    Missing                 // no surface answer AND no usable incident triangle
};

// --- Named tolerances (NUM §1.1: no shared global epsilon) -----------------

// NUM §7.4 acceptance for a smooth analytic node, in radians (0.1 degrees).
inline constexpr double kNormalAcceptanceRad = 1.7453292519943295e-3;

// NUM §7.3: a singular node whose usable incident facet normals span at most
// this angle is answered by their area-weighted sum instead of being split.
inline constexpr double kSingularSpreadRad = 8.726646259971647e-2;  // 5 degrees

// Relative error budget for ONE double-precision surface D1 evaluation. OCCT
// evaluates an analytic surface in closed form (a few ulp) and a B-spline by de
// Boor, whose relative error grows like O(degree * u) with u = 1.1e-16; 1e-12 is
// that bound with about two orders of magnitude of headroom.
inline constexpr double kSurfaceD1RelativeNoise = 1e-12;

// Floor on sin(angle(S_u, S_v)) for the cross product to carry a trustworthy
// DIRECTION. The cross product's absolute error is about
// `2 * noise * |S_u| * |S_v|`, so the induced angular error is `2 * noise / sin`.
// Requiring that to stay under kNormalAcceptanceRad gives
// `sin >= 2 * 1e-12 / 1.745e-3 = 1.15e-9`; 1e-8 keeps a factor of ~9 in hand.
inline constexpr double kMinDerivativeSine = 1e-8;

// Floor on a partial derivative's magnitude, expressed as the fraction of the
// face's world bbox diagonal that the derivative moves the point across the
// WHOLE parameter span (`|S_u| * uSpan >= ratio * diagonal`). A derivative below
// this is indistinguishable from the roundoff of the position itself, which is
// about 1e-16 of the diagonal.
inline constexpr double kMinDerivativeSpanRatio = 1e-9;

// A triangle carries a usable normal direction when its two edge vectors are
// separated by at least this sine and its longest edge is at least
// kMinTriangleEdgeRatio of the face diagonal. Position roundoff is ~1e-16 of the
// diagonal, so both floors keep the facet direction error near 1e-4 rad
// (0.006 deg), well inside the 0.1 deg budget.
inline constexpr double kMinTriangleSine = 1e-12;
inline constexpr double kMinTriangleEdgeRatio = 1e-12;

// A face whose surface area is below this fraction of the body's squared bbox
// diagonal cannot carry a drawable triangle at any display tier, so an empty
// triangulation on it is expected rather than a producer defect (NUM §9.2:
// "A known zero-triangle degenerate face can have bound 0 but must have explicit
// producer diagnostics; a missing nondegenerate face cannot use that escape").
inline constexpr double kDegenerateFaceAreaRatio = 1e-12;

// One face's shading normals, ready for the caller to append to its mesh arrays.
//
// The emitted vertex block is FACE-LOCAL and contiguous: entries `0 ..
// vertexNode.size()-1` are appended in order, so a caller that adds its running
// `base` keeps one contiguous vertex range per face. Node `i` always emits at
// index `i` first (`nodeRemap[i] == i`); any singular-split copies are appended
// after every node, so triangle ORDER and face RANGES are untouched — only the
// vertex indices inside a triangle change (NUM §7.3).
struct FaceNormalResult {
    std::vector<float> normals;                       // 3 per emitted vertex, unit
    std::vector<NormalProvenance> provenance;         // one per emitted vertex
    std::vector<std::uint32_t> vertexNode;            // emitted vertex -> source node
    std::vector<std::uint32_t> nodeRemap;             // node -> first emitted vertex
    std::vector<std::uint32_t> triangleVertexIndices; // 3*T face-local, emitted winding
    std::vector<gp_Pnt> worldNodes;                   // node positions with `loc` applied once
    std::uint32_t fallbackCount = 0;   // nodes answered by TriangulationFallback
    std::uint32_t splitCount = 0;      // nodes split per incident triangle
    std::uint32_t missingCount = 0;    // nodes with no normal source at all
    std::uint32_t droppedTriangles = 0;
    bool reverseWinding = false;
    // At least one nondegenerate triangle survived. The CALLER decides whether a
    // false here is a defect: a face that `face_is_degenerate` accepts keeps a
    // zero range with an explicit reason instead (spec §14, NUM §9.2).
    bool complete = false;
    std::string diagnostic;
};

// Shading normals for one already-triangulated face. `reversed` is
// `face.Orientation() == TopAbs_REVERSED`; `loc` is the location
// `BRep_Tool::Triangulation` handed back with `tri`.
FaceNormalResult compute_face_normals(const TopoDS_Face& face,
                                      const Handle(Poly_Triangulation)& tri,
                                      const TopLoc_Location& loc, bool reversed);

// True when the face can carry no drawable triangle at all: every one of its
// edges is `BRep_Tool::Degenerated`, or its exact surface area is below
// kDegenerateFaceAreaRatio of `bodyDiagonalMm` squared. A face whose area cannot
// be evaluated is reported NOT degenerate, so an unknown face is diagnosed
// loudly rather than excused.
bool face_is_degenerate(const TopoDS_Face& face, double bodyDiagonalMm);

}  // namespace onecad::tess
