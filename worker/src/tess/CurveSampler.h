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

// Angular evidence for one span. READ THE THREE MEANINGS AS WRITTEN — they are
// three different epistemic states, not three degrees of badness.
enum class AngularStatus {
    // PROVED. Every derivative-numerator Bernstein coefficient lies inside the
    // half-cone about the EMITTED (float32) chord, and the proof is complete:
    // the retained Bernstein mass and the coefficient arithmetic uncertainty are
    // both charged (followup §2 C2, `beta + asin(E/g) <= angularTol/2`). Because
    // every tangent in the span is then within half the tolerance of the emitted
    // chord, two adjacent Satisfied leaves turn by at most the full tolerance.
    Satisfied,
    // EVIDENCE INCOMPLETE — emphatically NOT "the derivative vanishes". The
    // calculation did not establish the cone: the retained mass vanished, the
    // arithmetic uncertainty reached the cone margin, the weights were too
    // ill-conditioned to trust the numerator, or the span's emitted float32
    // chord cannot carry a direction the double chord can. The POSITION bound is
    // untouched; the leaf is published on its chord certificate. A computed
    // floating-point zero is not a proved zero, so this status never asserts a
    // property of the curve — only the absence of a proof.
    Undefined,
    // DISPROVED. Some coefficient provably lies outside the cone about the double
    // chord, so the span must be split. A span is never ACCEPTED with this
    // status.
    Uncertified
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
    // Certified hull distance to this chord, with the F6 roundoff enclosure of
    // the generated control net already charged to it (followup §2 F6).
    double chordBoundMm = 0.0;
    double chordLengthMm = 0.0;   // |P_n - P_0| for this segment
    AngularStatus angular = AngularStatus::Uncertified;
    // followup §2 C2: `angularTol/2 - (beta + asin(E/g))`, the CONTINUOUS margin
    // behind the discrete status. Non-negative exactly when `angular ==
    // Satisfied`; -1 when no cone was evaluated at all.
    double angularMarginRad = -1.0;
    // followup §2 F5: the angle between this leaf's double-precision chord and
    // the chord of its two EMITTED float32 endpoints. The cone is tested about
    // the ENCODED chord, so this is published evidence, not a correction.
    double encodedChordRotationRad = 0.0;
    // The curve's derivative vanishes exactly at this leaf boundary — a cusp or
    // stationary point, i.e. a point where no tangent exists. Distinct from
    // `angular == Undefined`, which only says the cone test established nothing.
    bool singularVertexAtStart = false;
    bool singularVertexAtEnd = false;
    // The curve's own ONE-SIDED tangent directions at t0 and t1, as unit vectors
    // read off the first and last derivative-numerator coefficients (Q(0) and
    // Q(1) ARE those tangents up to the positive factor W^2). Zero when the
    // derivative vanishes there. followup §2 C1 writes the observed turn as
    // `J + beta- + beta+ + ...` with J the curve's GENUINE tangent jump at the
    // join; these are how J is measured, so a real C0 corner is permitted its own
    // corner while the polyline is still held to the tolerance ON TOP of it.
    gp_XYZ startTangent = gp_XYZ(0.0, 0.0, 0.0);
    gp_XYZ endTangent = gp_XYZ(0.0, 0.0, 0.0);
    // Whether this leaf still touches the endpoint of the EXACT source span it
    // came from. A join with `endsAtSourceEnd == false` on its left is interior
    // to one analytic Bezier span, where the two sides are the same Q at the
    // same parameter and J = 0 is PROVED (verify §4: "at a boundary known to
    // subdivide one regular analytic span, use the proven J = 0").
    bool startsAtSourceStart = true;
    bool endsAtSourceEnd = true;
    // verify §4 (D-b): those directions are COMPUTED, so J is an interval, not a
    // number. This is the half-width each side contributes: asin(eps/||q||) for
    // the endpoint coefficient's own uncertainty enclosure, and pi/2 when the
    // enclosure covers the coefficient entirely — which is what a genuine cusp
    // looks like, and the only honest reason a join escapes the turn rule.
    double startTangentUncertaintyRad = -1.0;
    double endTangentUncertaintyRad = -1.0;
    // followup §2 C1: an UNRESOLVED SINGULAR REGION. The leaf sits below the
    // positional resolution floor AND its derivative-numerator coefficients
    // could not be separated from the origin by any tested direction, so Q may
    // vanish inside it and a tangent reversal there is real geometry. This is
    // the ONLY evidence besides an exactly vanishing endpoint coefficient that
    // exempts a join from the display-turn rule — a failed cone or a short leaf
    // authorises nothing on its own.
    bool unresolvedRegion = false;
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
    // followup §2 C1: the greatest turn between two consecutive EMITTED float32
    // chords over the joins the display-turn rule actually binds (singular joins
    // excluded). -1 when there is no such join. This is the number a faceted
    // silhouette is made of.
    double encodedTurnMaxRad = -1;
    // verify §5: a SEPARATE conditioning diagnostic, never a licence to suppress
    // a turn. Counts the joins whose two emitted chords are so short that the
    // float32 rounding of their own endpoints alone spans the allowance. The
    // turn at such a join is still measured, still published and still a
    // violation if it is one — the emitted vertices determine it exactly.
    std::size_t quantizationConditionedJoins = 0;
    // PR-07 / D9. What the caller ASKED for and what the emitted polyline
    // actually holds. They differ only when the caller's edge policy re-sampled
    // at a relaxed budget (`Tessellate.cpp`'s doubling ladder): a retry that
    // succeeded at 2x reports requested 0.05 / achieved 0.10 and certification
    // QualityLimited. The retry's own internal `Certified` never leaks out.
    double requestedChordToleranceMm = -1;
    double achievedChordToleranceMm = -1;
    double requestedAngularToleranceRad = -1;
    double achievedAngularToleranceRad = -1;
    CurveCertification certification = CurveCertification::Failed;
    // PR-07 / D9, sharpened by R1(c) MINOR 7: the requested display quality was
    // not met. Carried ALONGSIDE `certification` so a relaxed retry through the
    // approximate fallback keeps its provenance (`KernelEstimated`) instead of
    // having it overwritten by the quality verdict.
    bool qualityLimited = false;
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
    // Per-axis greatest |coordinate| of the SOURCE span this one descends from.
    // The F6 roundoff enclosure is proportional to the magnitudes the de
    // Casteljau recurrence actually handled, which are the ancestor's, not the
    // (smaller) child's. Zero on a hand-built span, where the span's own extents
    // are used instead.
    gp_XYZ sourceMaxAbs = gp_XYZ(0.0, 0.0, 0.0);
};

// followup §2 F6. Every GENERATED control net — source conversion and every
// subdivision — must pass this before any bound computed from it means anything.
// A net with a non-finite pole or a non-positive/non-finite homogeneous weight
// breaks the convex-hull argument outright, and `std::max(worst, NaN)` would
// quietly return a ZERO chord bound for it.
bool control_net_is_valid(const RationalBezierSpan& span);

// verify §6 (REFUTED: finite positive weights are not enough). The homogeneous
// coordinates w_i * P_i are what the de Casteljau recurrence actually carries,
// and `fl(w_i * P_i) == 0` for a NONZERO coordinate destroys the curve without
// making any weight non-positive or any pole non-finite: w = (2^-1074, 1),
// P = (1e-5, 2e-5) mm dehomogenizes its first endpoint to 0 while the exact
// endpoint is 1e-5 mm, an error fifteen orders of magnitude past the enclosure.
// False whenever a homogeneous product underflows to zero or leaves the normal
// range, in which case NOTHING computed from the net may be certified.
bool homogeneous_arithmetic_is_supported(const RationalBezierSpan& span);

// followup §2 F6 positional roundoff enclosure. With u = 2^-53, K = depth*(n+3)+2
// and g = gamma_K = K*u/(1-K*u), a computed pole of a span subdivided to `depth`
// from a degree-`n` source carries a per-axis error of at most
// `[(2g + u(1+g)) / (1-g)] * M_j`. At n = 32, depth = 32 (K = 1122) the
// multiplier is 2.492e-13, i.e. 4.317e-4 mm Euclidean when every |coordinate| is
// at most 1e9 mm. Weight-ratio independent.
double pole_roundoff_multiplier(int depth, int degree);
double pole_roundoff_enclosure(const RationalBezierSpan& span);

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

// What the NUM §2.4 forward-cone test established about a span, with the
// followup §2 C2 derivation supplying the acceptance condition. Writing
// Q = G + H with G the RETAINED (computed-nonzero) coefficients and H the
// excluded ones, s(t) the retained Bernstein mass, p_min the least retained
// dot(q_i, axis), g = p_min * s_min and E the excluded mass plus the coefficient
// arithmetic uncertainty:
//
//     angle(Q(t), axis) <= beta + asin(E/g)   whenever E < g and s_min > 0
//
// so `Satisfied` is justified exactly when `beta + asin(E/g) <= angularTol/2`.
// There is no derivation of any universal "too small to test" threshold, and
// this struct carries none: only an EXACTLY zero coefficient is excluded, and
// its uncertainty is still charged through E.
struct ConeVerdict {
    AngularStatus status = AngularStatus::Uncertified;
    double coneHalfAngleRad = 0.0;   // beta: greatest angle from a retained coefficient to `axis`
    double uncertaintyRad = 0.0;     // asin(E/g), or pi when the evidence does not close
    double marginRad = -1.0;         // angularTol/2 - (beta + uncertainty)
    double retainedMassMin = 0.0;    // s_min, the retained Bernstein mass lower bound
    double excludedMass = 0.0;       // E
    double retainedGap = 0.0;        // g = p_min * s_min
    bool singularAtStart = false;    // q_0 computed exactly zero
    bool singularAtEnd = false;      // q_m computed exactly zero
    // No tested direction separates the coefficient hull from the origin, so Q
    // MAY vanish inside this span. A necessary condition for a singularity, never
    // by itself sufficient — which is why it only ever combines with the
    // positional resolution floor (followup §2 C1).
    bool reversalPresent = false;
    // Unit one-sided tangent directions at the span's two ends, zero where the
    // derivative numerator vanishes. Independent of `axis`.
    gp_XYZ startDirection = gp_XYZ(0.0, 0.0, 0.0);
    gp_XYZ endDirection = gp_XYZ(0.0, 0.0, 0.0);
    // asin(eps / ||q||) for the corresponding endpoint coefficient: how far its
    // computed DIRECTION may be from the true one. pi/2 when the enclosure
    // swallows the coefficient, i.e. the direction is not resolved at all.
    double startDirectionUncertaintyRad = 0.5 * 3.14159265358979323846;
    double endDirectionUncertaintyRad = 0.5 * 3.14159265358979323846;
    const char* incompleteReason = nullptr;
};

// NUM §2.4 forward-cone test about an EXPLICIT axis. The sampler passes the
// EMITTED float32 chord (followup §2 F5): a cone proved about the double chord
// says nothing about the segment a viewer actually sees.
// `poleEnclosureMm` is the positional roundoff enclosure of the span's own
// computed poles (`pole_roundoff_enclosure`). verify §6 / R1(c) MAJOR 4: the
// coefficients are built FROM those poles, so a pole perturbation of delta moves
// q_k by at most 4 n wmax^2 delta and that term belongs in E. Pass 0 only for a
// net whose poles are exact by construction.
ConeVerdict tangent_cone_verdict(const RationalBezierSpan& span, double angularTol,
                                 const gp_XYZ& axis, double poleEnclosureMm);

// The same test about the span's own double-precision chord.
AngularStatus tangent_cone_status(const RationalBezierSpan& span, double angularTol);

// Same, additionally reporting WHICH endpoint has an exactly vanishing
// derivative numerator.
AngularStatus tangent_cone_status(const RationalBezierSpan& span, double angularTol,
                                  bool& undefinedAtStart, bool& undefinedAtEnd);

}  // namespace onecad::tess
