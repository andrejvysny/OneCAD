# Lenses

Append the lens or lenses matching the question to the end of the packet under a `<lens>` block. Combine when the question spans domains; name them in the approval summary.

## Tolerance policy

Attack the choice of absolute versus relative versus curvature-relative versus kernel-tolerance-relative epsilon. Check behaviour under a similarity transform and a unit change. Ask what the residual actually measures (positional, angular, curvature) and whether one epsilon is being asked to do two jobs. Separate "topology is wrong" from "approximation is coarse"; those need different predicates and different diagnostics.

## Blend and G-continuity

Distinguish G0 (position), G1 (tangent), G2 (curvature) budgets and state each in the units it lives in. For a rolling-ball blend the positional residual is curvature-derived, not proportional to face tolerance; check any factor claimed against a cylinder-cylinder tee, an oblique elliptical rim, a cone rim, a valence-5 corner, and a near-seam edge. Ask what happens when the radius exceeds the local feature size or when the two supports are tangent.

## Robust predicates

Exact versus filtered versus floating-point predicates; where sign errors flip a decision. Conditioning of each formula near its degenerate configuration. Whether a threshold compares like with like (distance to distance, angle to angle). Whether the predicate is symmetric in its arguments when the geometry is.

## Sweep framing

Frenet, rotation-minimizing, fixed-normal, and the kernel's own frame: which one the profile orientation contract needs. Attack zero curvature, inflection points, closed paths (twist accumulation and closure mismatch), cusps, self-intersection when the radius of curvature is below the profile extent, and profile placement off the path.

## Loft correspondence

Profile-to-profile vertex correspondence, seam placement, orientation consistency, periodic profiles, differing vertex counts, and what changes when one profile is edited. Ask which choices are deterministic functions of the input and which need a stored user decision.

## Topology identity scoring

Treat matching as a scoring function over history lineage, geometry descriptors, anchors, adjacency, sidedness, and topology class. Attack the weights: find a symmetric tie that must produce `NeedsRepair`, a consumed element that must not re-bind, and a small edit that must not flip a bind. Check that the score and margin thresholds cannot be gamed by a near-duplicate.

## Constraint solver conditioning

Jacobian rank, redundant and conflicting constraints, scaling between distance and angle rows, convergence basin near a flip, and what the solver reports when the system is under- or over-determined. Ask for a configuration where the solver converges to the wrong root.

## Curve and region intersection

Curve-curve intersection refinement, periodic parameter intervals, tangential contacts, coincident and overlapping supports (refuse, do not invent), region closure under near-touching endpoints, and orientation of the resulting loops.

## Numerical optimisation

Objective conditioning, step control, termination criteria, and whether the reported optimum is a true minimum. Ask for the input on which the method stalls or diverges.

## Prior art and kernel behaviour

Separate what OCCT *specifies* from what it is merely *observed* to do here, and say which class each
claim belongs to. For a published algorithm, state its assumptions explicitly — exact arithmetic,
bounded degree, general position, closed or manifold input, a single length scale — and say which of
them this repository cannot guarantee. Prefer a source that documents its own failure cases over one
that reports only successes. Where the prior art disagrees with the current implementation, say which
one is making the stronger claim.

## Algorithmic performance

Where the cost actually sits, and what the asymptotic constant is hiding at the sizes that matter.
Distinguish a speed-up that changes only a duration from one that changes a decision — a cheaper
predicate that can flip a sign is a correctness change wearing a performance costume. Ask which stage
to measure first and what input isolates it, and whether the proposed change moves work from a place
that is cached to a place that is not, or from regen time to edit time.
