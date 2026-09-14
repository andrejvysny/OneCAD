// SurfaceNormals.h — per-node shading normals evaluated from the SUPPORTING
// SURFACE (VP-HARDENING WP09, requirement VP10, findings R04/R11). Design:
// docs/viewport-hardening/03-NUMERICS-AND-PROTOCOL.md §7 (binding) and
// docs/design/astra/wp09-surface-normals-break.md (the accepted adversarial
// review; findings F1–F11 and its §3 degeneracy contract are binding here).
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
//
// WHAT THE ASTRA BREAK CHANGED (all of it fails CLOSED — an answer this file
// cannot bound is reported, never rounded off):
//
//   * F3/F4 — a regular node is only `Surface` when the induced angular error of
//     its cross product is under NUM §7.4's 0.1 deg, measured against an
//     ABSOLUTE derivative-error budget derived from the surface's own
//     REPRESENTATION (`derivative_error_budget`). The retired relative floors
//     (`kSurfaceD1RelativeNoise`, `kMinDerivativeSine`, `kMinDerivativeSpanRatio`)
//     claimed a bound nobody had proved and were scaled by a world AABB diagonal
//     that is not rotation invariant. A node the budget cannot certify is
//     `Unresolved`, not silently `Surface`.
//   * F5/F9 — triangle usability is a symmetric, permutation-invariant predicate
//     with its own error bound, and only a CERTIFIED-zero triangle is ever
//     dropped. A triangle whose direction cannot be resolved is kept.
//   * F7 — the sphere-pole sign comes from a regular witness point of the same
//     face, never from the incident facets (they may only diagnose).
//   * F8 — a cone apex node is split per incident fan BEFORE the general 5 deg
//     averaging rule can invent an axial normal.
//   * F10 — a `Missing` node is not emitted at all, so no arbitrary +Z reaches
//     the wire, and a `Missing` node referenced by a kept triangle fails the face.
//   * F11 — the pairwise spread scan is bounded by `kMaxSpreadValence`; above it
//     the node is split deterministically in O(k).
//   * F2/§3 — face degeneracy is decided by `classify_face_degeneracy`, a
//     CERTIFICATE, never by an area threshold relative to the body.
#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <Poly_Triangulation.hxx>
#include <TopLoc_Location.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

namespace onecad::tess {

// Where one emitted vertex's normal came from (NUM §7.3, Astra F3/F5). Published
// on the wire only as the QUALITY_INFO "some normal fallback" bit (WP10); here it
// is the per-vertex evidence the focused tests assert on.
enum class NormalProvenance : std::uint8_t {
    Surface,                // S_u x S_v on the supporting surface, regular node, CERTIFIED
    AnalyticSingular,       // analytic limit at a known singularity (sphere pole)
    TriangulationFallback,  // singular node, usable incident facets agree within 5 deg
    SingularSplit,          // singular node split per incident triangle
    Unresolved,             // an answer exists but no error bound certifies it (F3/F4/F5)
    Missing                 // no surface answer AND no incident triangle with a direction
};

// --- Named tolerances (NUM §1.1: no shared global epsilon) -----------------

// NUM §7.4 acceptance for a smooth analytic node, in radians (0.1 degrees).
inline constexpr double kNormalAcceptanceRad = 1.7453292519943295e-3;

// NUM §7.3: a singular node whose usable incident facet normals span at most
// this angle is answered by their area-weighted sum instead of being split.
inline constexpr double kSingularSpreadRad = 8.726646259971647e-2;  // 5 degrees

// The unit roundoff of IEEE binary64, u = 2^-53. Named here because every
// derivative and position bound below is expressed in ulps of a magnitude the
// REPRESENTATION actually carries, never as a claimed relative noise floor
// (Astra F3: "Comparing against another call to the same OCCT D1 cannot validate
// its error").
inline constexpr double kUnitRoundoff = 0x1p-53;

// Ulps of the largest magnitude a CLOSED-FORM analytic D1 evaluation handles,
// per derivative component. gp_'s elementary surfaces evaluate one sin/cos pair,
// two or three scalar products and one sum, so eight ulps of the largest
// quantity involved (the radius, the reference radius plus the axial travel, the
// transformed axis directions) bounds the component error with room to spare.
// Astra F3's "8u*(|centre| + R) style": few ulps of the largest magnitude
// involved. It is NOT a relative bound on the RESULT — a cone's `R + v sin(a)`
// cancels to zero at the apex and the budget stays proportional to `R`, which is
// exactly the conditioning the retired relative floor could not see.
inline constexpr double kAnalyticDerivativeUlps = 8.0;

// Astra F5: a triangle's normal DIRECTION is trustworthy when its first-order
// angular uncertainty stays under this bound. 1e-4 rad (0.0057 deg) is a
// twentieth of the 0.1 deg node budget, so a facet that passes cannot move a
// fallback normal out of NUM §7.4's window on its own.
inline constexpr double kTriangleDirectionAcceptanceRad = 1e-4;

// Astra F11: the pairwise spread scan is k(k-1)/2, so a collapsed spline row of
// valence 10 000 would cost 49 995 000 comparisons. Above this valence the node
// is split deterministically without measuring the spread — the answer a large
// fan would almost always reach anyway, and O(k) instead of O(k^2).
inline constexpr std::size_t kMaxSpreadValence = 64;

// --- Derivative error budget (Astra F3/F4) ---------------------------------

// An ABSOLUTE bound, in mm, on the Euclidean error of ONE surface D1 derivative
// vector, derived from the surface's own stored representation.
//
// WHY absolute and representation-derived: the retired `kSurfaceD1RelativeNoise`
// asserted that one D1 is accurate to 1e-12 RELATIVE to the derivative it
// returns. Astra F3 exhibits a binary64 derivative-control evaluation that loses
// half of one component (`M = 2^42`, `a = 2^-11`: the exact `Dx = a` is computed
// as `a/2`, an 18.43 degree normal error) while every retired floor passes,
// because the error is set by the POLE magnitudes the recurrence handled, not by
// the small derivative that came out of it. A budget that cannot see `M` cannot
// see that failure.
struct DerivativeBudget {
    // Bound on |computed S_u - exact S_u| (and the same for S_v), in mm.
    double absoluteMm = 0.0;
    // False when the representation carries no bound this file can derive — an
    // offset surface, a surface of revolution/extrusion, an unrecognised type, a
    // rational patch with a non-positive weight. Every node on such a face is
    // `Unresolved`; nothing is assumed.
    bool known = false;
    const char* reason = "uninitialised";
};

// The budget for one face's supporting surface. `surf` must already be
// initialised from the face, so every magnitude it reports is in the same WORLD
// placement as its own D1 (BRepAdaptor_Surface applies the location once, and
// `BSpline()`/`Bezier()`/`Sphere()` come back transformed with it — verified
// against OCCT 8.0.1).
DerivativeBudget derivative_error_budget(const BRepAdaptor_Surface& surf);

// First-order angular uncertainty, in radians, of `normalize(du x dv)` given the
// budget: `asin( (e*(|du| + |dv|) + e^2) / |du x dv| )`, and +infinity when the
// perturbation can reach the cross product at all (an unbounded direction) or
// the budget is unknown. Compare against kNormalAcceptanceRad.
double normal_angular_error_rad(const DerivativeBudget& budget, const gp_Vec& du,
                                const gp_Vec& dv);

// --- Face degeneracy (Astra §3) --------------------------------------------

// Astra §3: "A safe replacement is certificate-based and conservative. No
// area-only threshold separates the requested classes." The retired predicate
// compared the face area against 1e-12 times the BODY's squared bbox diagonal,
// which excuses a perfectly valid 0.005 x 0.005 mm face on a 10 m body
// (2.5e-5 mm^2 vs a 1e-4 mm^2 threshold), and treated all-degenerate boundary
// edges as proof — refuted by `f = 16u(1-u)v(1-v)`, `S = ((2u-1)f, (2v-1)f, f)`,
// whose whole boundary maps to one point while the centre Jacobian is 4 mm^2.
enum class DegeneracyClass : std::uint8_t {
    // The face's retained surface image provably has dimension <= 1 (zero area).
    CertifiedZero,
    // Reserved (Astra §3 "Kernel-collapsed sliver"). NEVER returned today: the
    // authoritative BRep sub-tolerance collapse rule and its tolerance-composition
    // semantics are the explicit PROOF GAP of the break — "If no authoritative
    // kernel-collapse rule grants zero-range status, return Unproved." Overlapping
    // tolerance tubes establish unresolved WIDTH, not permission to identify
    // topology, so a sliver falls to Unproved and is reported as a missing face.
    CertifiedKernelCollapse,
    // Not certified. Fails CLOSED: the caller must treat an uncovered face as a
    // MISSING face, never as an excused one.
    Unproved
};

struct FaceDegeneracy {
    DegeneracyClass cls = DegeneracyClass::Unproved;
    std::string reason;
    bool certified() const {
        return cls == DegeneracyClass::CertifiedZero ||
               cls == DegeneracyClass::CertifiedKernelCollapse;
    }
};

// Face-LOCAL and scale-free: nothing about the rest of the body enters. The
// sufficient certificates, each exact or symbolic:
//
//   (a) the retained 2D trim domain has zero measure — `BRepTools::UVBounds`
//       gives `umax <= umin` or `vmax <= vmin` under EXACT comparison, so the
//       retained parameter set lies in a line and its image has dimension <= 1;
//   (b) an elementary surface (plane/cylinder/cone/sphere/torus) whose OWN
//       parametric domain has an exactly zero extent in u or v, so the surface
//       image is a curve before any trimming;
//   (c) a polynomial (or constant-weight rational) Bezier/B-spline patch whose
//       derivative control net is rank one — every u-difference and v-difference
//       pole is parallel to one common direction, so `S_u x S_v` is identically
//       zero over the whole retained span.
//
// Everything else is `Unproved`, including: a face with no trim description at
// all (Astra §3: "Missing/malformed trimming is uncertainty, not an empty-domain
// certificate"), a rational patch with a non-positive or non-finite weight, a
// sub-tolerance sliver, and any face whose representation cannot be inspected.
// All-degenerate boundary edges are EVIDENCE recorded in `reason`, never a
// certificate. A regular interior sample with a certified nonzero Jacobian is
// recorded in `reason` as a positive disproof of degeneracy.
FaceDegeneracy classify_face_degeneracy(const TopoDS_Face& face);

// One face's shading normals, ready for the caller to append to its mesh arrays.
//
// The emitted vertex block is FACE-LOCAL and contiguous: entries `0 ..
// normals.size()/3-1` are appended in order, so a caller that adds its running
// `base` keeps one contiguous vertex range per face. `nodeRemap[i]` is the first
// emitted vertex of node `i`, or `kNoVertex` when the node was NOT emitted
// (Astra F10: a `Missing` node must not serialise an arbitrary +Z, so it gets no
// slot at all). Any singular-split copies are appended after every node, so
// triangle ORDER and face RANGES are untouched — only the vertex indices inside
// a triangle change (NUM §7.3).
struct FaceNormalResult {
    static constexpr std::uint32_t kNoVertex = 0xFFFFFFFFU;

    std::vector<float> normals;                       // 3 per emitted vertex, unit
    std::vector<NormalProvenance> provenance;         // one per emitted vertex
    std::vector<std::uint32_t> vertexNode;            // emitted vertex -> source node
    std::vector<std::uint32_t> nodeRemap;             // node -> first emitted vertex, or kNoVertex
    std::vector<std::uint32_t> triangleVertexIndices; // 3*T face-local, emitted winding
    std::vector<gp_Pnt> worldNodes;                   // node positions with `loc` applied once
    std::uint32_t fallbackCount = 0;   // nodes answered by TriangulationFallback
    std::uint32_t splitCount = 0;      // nodes split per incident triangle
    std::uint32_t unresolvedCount = 0; // nodes whose answer carries no error bound (F3/F5)
    std::uint32_t missingCount = 0;    // nodes with no normal source at all
    std::uint32_t droppedTriangles = 0;        // CERTIFIED-zero triangles only (F5/F9)
    // Astra §4's last class: a triangle with positive double-precision area whose
    // EMITTED float32 positions collapse it to exactly zero. A representation
    // failure, recorded and warned — never a degeneracy exemption.
    std::uint32_t float32CollapsedTriangles = 0;
    bool reverseWinding = false;
    // At least one triangle survived AND every surviving triangle references a
    // node that actually has a normal. The CALLER decides whether a false here is
    // a defect: a face `classify_face_degeneracy` CERTIFIES keeps a zero range
    // with an explicit reason instead (spec §14, NUM §9.2, Astra F1/§3).
    bool complete = false;
    std::string diagnostic;
};

// Shading normals for one already-triangulated face. `reversed` is
// `face.Orientation() == TopAbs_REVERSED`; `loc` is the location
// `BRep_Tool::Triangulation` handed back with `tri`.
FaceNormalResult compute_face_normals(const TopoDS_Face& face,
                                      const Handle(Poly_Triangulation)& tri,
                                      const TopLoc_Location& loc, bool reversed);

}  // namespace onecad::tess
