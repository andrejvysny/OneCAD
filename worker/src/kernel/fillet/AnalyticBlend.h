// AnalyticBlend.h — an EXACT TANGENCY CERTIFICATE for a cylindrical face
// against two supports. Read the next paragraph before using it: the name of
// the file is about blends, and the function is not.
//
// WHAT THIS PROVES, AND WHAT IT DOES NOT. `analytic_cylinder_blend` proves one
// thing: that the candidate face is an exact `GeomAbs_Cylinder` and that its
// axis stands in the exact tangency relation to each support that a rolling ball
// of the same radius would produce. It does NOT prove the face IS a blend. A rod
// fused into a corner, or a user-authored quarter-round rib, certifies exactly
// like the fillet it resembles — because it IS tangent, and the material side is
// deliberately not consulted here. Blend-ness is established by the surrounding
// stack, not by this call: sampled recognition, closure discipline, the
// trim/extent proof, the suppression postconditions, and the
// reconstruct-at-the-same-radius reproduction proof (WP3's five layers). This
// module is one of those five, never a substitute for the other four.
//
// WHY IT EXISTS ANYWAY. `BlendRecognizer` infers a constant section radius from
// a finite sample set (`BlendEvidence`: 9 edge samples, a 5x5 UV grid). That is
// adequate to describe geometry and inadequate to authorize destroying it: a
// surface can measure constant-radius at every sample and be something else
// between the samples. This module converts that one property from a 25-sample
// inference into an exact statement about `BRepAdaptor_Surface::Cylinder()`. A
// B-spline that reproduces a cylinder to machine precision is `Unknown`, never
// `Proved`, however well it samples — see `test_blend_analytic.cpp`, which
// measures a decoy the production sampler accepts and this gate refuses.
#pragma once

#include <TopoDS_Face.hxx>

namespace onecad::kernel::fillet {

// WHAT A CONSUMER MAY DO WITH EACH VERDICT:
//
//   `Proved`  — tangency holds exactly. NECESSARY for a destructive path and
//               never SUFFICIENT on its own; the other proof layers still have
//               to pass before anything may be suppressed or rebuilt.
//   `Failed`  — measured, and tangency does not hold.
//   `Unknown` — not measurable under these rules (a blend face that is not an
//               exact cylinder, or a support that is neither plane nor
//               cylinder). Absence of proof, never proof of absence.
//
// `Failed` and `Unknown` are EQUIVALENT for any destructive decision: both mean
// "do not engage the blend path". They differ only as diagnostic detail —
// `Failed` can name a measured mismatch, `Unknown` can only name what it could
// not measure. A consumer that branches differently on the two is wrong.
enum class AnalyticBlendProof { Proved, Failed, Unknown };

enum class BlendConvexity { Unknown, Convex, Concave };

// Certifies that `blend_face` is exactly tangent to both supports, as a
// constant-radius rolling ball would leave it.
//
// Blend face: MUST be `GeomAbs_Cylinder`, else `Unknown`. Radius R is the
// analytic cylinder radius. Per support, with tolerances from
// `precision_of(blend_face)`:
//   * plane     — blend axis perpendicular to the plane normal AND
//                 axis-to-plane distance == R.
//   * cylinder  — axes parallel AND axis-to-axis distance in {R_s + R,
//                 |R_s - R|} (the external and internal tangency branches),
//                 EXCEPT the co-surface case (distance 0 and R_s == R), which is
//                 the face certifying against itself and is `Failed`.
//   * otherwise — `Unknown`.
// `Failed` dominates `Unknown`: a measured mismatch on either support is
// reported as such even when the other support is unmeasurable.
AnalyticBlendProof analytic_cylinder_blend(const TopoDS_Face &blend_face,
                                           const TopoDS_Face &support_a,
                                           const TopoDS_Face &support_b);

// Sign of the face: does the solid bulge away from the cylinder axis (Convex,
// an outside round) or wrap around it (Concave, an inside fillet)? Exact for a
// cylindrical face — the outward normal of a cylinder is radial by construction,
// so this is a sign test, not a measurement. `Unknown` for anything else.
BlendConvexity blend_convexity(const TopoDS_Face &blend_face);

} // namespace onecad::kernel::fillet
