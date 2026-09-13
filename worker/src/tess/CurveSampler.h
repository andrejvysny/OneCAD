// CurveSampler.h — exact-span edge sampler (VP-HARDENING WP08, requirement VP11,
// finding R03). Design: docs/viewport-hardening/03-NUMERICS-AND-PROTOCOL.md §2.
//
// WHY this exists: the retired sampler split a span only when its MIDPOINT left
// the chord and its two ENDPOINT tangents disagreed. Both tests pass on the
// degree-five S-curve of NUM §2.6, whose midpoint sits exactly on the chord and
// whose endpoint tangents are parallel, while the curve leaves the chord by
// 1.125 mm at t=0.25. A sampled criterion cannot see an excursion it did not
// sample, so this sampler replaces it with a criterion that holds for the WHOLE
// span:
//
//   * every supported curve family is converted to EXACT positive-weight
//     rational Bézier spans (NUM §2.1/§2.2), so the curve provably lies inside
//     the convex hull of its control points;
//   * a span's chord error is bounded by the greatest distance from a control
//     point to the FINITE chord segment (NUM §2.3) — a certificate, not a sample;
//   * its angular error is bounded by a forward cone over the Bernstein
//     coefficients of the derivative numerator Q = X'W - XW' (NUM §2.4);
//   * subdivision is de Casteljau at t=0.5 in homogeneous coordinates on an
//     explicit stack, under depth / per-edge / per-body caps (NUM §2.5);
//   * a family that has no exact rational form (offset, procedural) goes through
//     the fixed GeomConvert_ApproxCurve fallback and is labelled KernelEstimated
//     — never Certified, whatever a dense sample check says (NUM §2.7).
//
// A cap is NOT an acceptance condition. Hitting one yields QualityLimited and
// the caller must discard `points`/`leaves` rather than draw a coarse polyline
// that claims to be within budget.
#pragma once

#include <cstddef>
#include <functional>
#include <string>
#include <vector>

#include <gp_Pnt.hxx>
#include <gp_XYZ.hxx>

class BRepAdaptor_Curve;

namespace onecad::tess {

// How much the emitted polyline's accuracy is actually KNOWN (NUM §1.3).
enum class CurveCertification {
    Certified,        // hull bound holds for every emitted span; exact conversion
    KernelEstimated,  // approximate conversion; sampled evidence only, no proof
    QualityLimited,   // a depth / segment / body cap was reached before acceptance
    Failed            // not samplable, or the approximation was rejected
};

// Angular evidence for one span.
enum class AngularStatus {
    Satisfied,    // every significant derivative-numerator pole is inside the cone
    Undefined,    // the derivative vanishes at a span endpoint (stationary/singular)
    Uncertified   // the cone criterion does not hold — the span must be split
};

// One emitted segment. `t0`/`t1` are EDGE parameters: every source piece is
// mapped affinely from its converted domain onto the edge interval it covers, so
// the leaves of one edge are contiguous and strictly increasing end to end. For
// line/Bézier/B-spline/fallback pieces that map is the identity, so the values
// are the exact edge parameters (and land exactly on knots). For conics it is a
// monotone affine image of the exact converted parameter, not the conic's own
// angular parameter.
struct CurveLeaf {
    double t0 = 0.0;
    double t1 = 0.0;
    double chordBoundMm = 0.0;    // certified hull distance to this chord
    double chordLengthMm = 0.0;   // |P_n - P_0| for this segment
    AngularStatus angular = AngularStatus::Uncertified;
    // The curve's derivative vanishes exactly at this leaf boundary — a cusp or
    // stationary point, i.e. a point where no tangent exists. Distinct from
    // `angular == Undefined`, which only says the cone test established nothing.
    bool singularVertexAtStart = false;
    bool singularVertexAtEnd = false;
};

struct CurveSampleResult {
    std::vector<gp_Pnt> points;   // world (transformed) doubles; exact first/last endpoints
    std::vector<CurveLeaf> leaves;  // one per emitted segment (points.size() - 1)
    double certifiedChordBoundMm = -1;  // greatest leaf bound when Certified, else -1
    double sampledMaxErrorMm = -1;      // fallback's independent check only, else -1
    // MEASURED float32 rounding of the emitted points (NUM §1.2): the greatest
    // Euclidean |double - float(double)| over `points`, not a guessed ULP
    // multiple. -1 when nothing was emitted.
    double quantizationErrorMm = -1;
    CurveCertification certification = CurveCertification::Failed;
    bool angularUndefinedSomewhere = false;
    // Edge parameters where the curve's derivative vanishes exactly. Each is a
    // leaf boundary: the singularity is explicit in the output, never spanned.
    std::vector<double> singularParameters;
    std::string diagnostic;  // typed reason text for stderr / WP10 wire metadata
};

// NUM §2.5 initial limits. `remainingBodySegments` is the caller's RUNNING total
// for the whole body — one edge never owns the whole budget.
struct CurveSampleLimits {
    int maxDepthPerSpan = 32;
    std::size_t maxSegmentsPerEdge = 262144;
    std::size_t remainingBodySegments = 2000000;
};

struct CurveSampleRequest {
    double chordToleranceMm = 0.05;
    double angularToleranceRad = 0.08726646259971647;  // 5 degrees
    // An ADDITIONAL allowance the caller wants reserved out of the chord budget,
    // on top of the per-span per-axis float32 allowance the sampler computes for
    // itself from each span's pole extents. Normally 0.
    double representationAllowanceMm = 0.0;
};

// Sample one edge.
//
// OUTCOME CONTRACT — the caller MUST branch on `certification` and on whether
// `points` is empty:
//   * Certified / KernelEstimated -> `points` is a complete polyline for the
//     whole edge and may be drawn.
//   * QualityLimited with a NON-EMPTY `points` -> the polyline is complete and
//     certified in double precision, but the output representation (float32
//     world coordinates) cannot hold the requested accuracy. It is the best the
//     format allows; draw it, and report the quality.
//   * QualityLimited with an EMPTY `points` -> a WORK cap (depth, per-edge
//     segments, per-body segments) was reached before the whole edge was
//     accepted. There is no valid polyline; never draw a straight chord in its
//     place and call it in budget.
//   * Failed -> nothing usable; `points` and `leaves` are empty and `diagnostic`
//     names the reason.
//
// ORIENTATION: `BRepAdaptor_Curve` is orientation-blind — `BRep_Tool::Range`
// does not flip for a REVERSED edge — so an edge and its reversal sample to the
// identical point sequence in the identical order. That matches the MESH1
// contract: EDGE_POSITIONS is an undirected polyline, and no consumer reads a
// direction from it. Do not "fix" this by reversing on orientation; it would
// change published bytes for no reader.
//
// `cancelled` may be empty; when set it is polled once per stack pop.
// Deterministic: identical input produces identical output, with no threading
// and no recursion (Invariant 5).
CurveSampleResult sample_edge_curve(const BRepAdaptor_Curve& curve,
                                    const CurveSampleRequest& request,
                                    const CurveSampleLimits& limits,
                                    const std::function<bool()>& cancelled);

// ---------------------------------------------------------------------------
// Pure helpers — exposed for unit tests (NUM §2.2-§2.4).
// ---------------------------------------------------------------------------

// One exact positive-weight rational Bézier span. `poles` are DEHOMOGENIZED
// control points in world coordinates; `weights` are the matching positive
// homogeneous weights. `t0`/`t1` carry the source parameter interval.
struct RationalBezierSpan {
    std::vector<gp_XYZ> poles;
    std::vector<double> weights;
    double t0 = 0.0;
    double t1 = 1.0;

    // Subdivision bookkeeping. `at_source_start`/`at_source_end` stay true only
    // while the span still touches the endpoint of the EXACT span it came from,
    // which is where an undefined tangent is a property of the curve rather than
    // of where the subdivision happened to cut.
    int depth = 0;
    bool at_source_start = true;
    bool at_source_end = true;
};

// de Casteljau at t=0.5 in homogeneous coordinates. The shared split point is
// computed ONCE and handed to both children, so the two halves meet exactly.
void subdivide_homogeneous(const RationalBezierSpan& span, RationalBezierSpan& left,
                           RationalBezierSpan& right);

// NUM §2.3: greatest distance from a dehomogenized pole to the FINITE segment
// [P0, Pn] (distance to P0 when that chord is degenerate). With positive weights
// the curve lies in the pole hull, so this bounds the whole span's chord error.
double hull_chord_bound(const RationalBezierSpan& span);

// NUM §2.4: Bernstein coefficients of Q = X'W - XW' at degree 2n-1, where
// X = sum(B_i w_i P_i) and W = sum(B_i w_i). C'(t) = Q(t)/W(t)^2, so Q carries
// the tangent direction exactly. Empty for a degree-0 span.
std::vector<gp_XYZ> derivative_numerator_bernstein(const RationalBezierSpan& span);

// NUM §2.4 forward-cone test against the span's chord direction. `roundoffBound`
// is DIMENSIONLESS: the coefficients are scaled by their greatest magnitude
// first, so it compares against relative sizes (NUM §1.2 suggests 64*eps).
AngularStatus tangent_cone_status(const RationalBezierSpan& span, double angularTol,
                                  double roundoffBound);

// Same test, additionally reporting WHICH endpoint has a vanishing derivative.
AngularStatus tangent_cone_status(const RationalBezierSpan& span, double angularTol,
                                  double roundoffBound, bool& undefinedAtStart,
                                  bool& undefinedAtEnd);

}  // namespace onecad::tess
