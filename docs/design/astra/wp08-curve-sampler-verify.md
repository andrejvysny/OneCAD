# wp08-curve-sampler-verify
date: 2026-09-14
mode: verify
model: gpt-6-astra
effort: high
access: prompt-only as configured (`--skip-git-repo-check`, scratchpad cwd, read-only sandbox); the transcript shows Astra nevertheless read `worker/src/tess/CurveSampler.cpp`, `worker/tests/test_curve_sampler.cpp` and the scratchpad probe `probe_c2_new.cpp` to check the packet against source — recorded as a deviation from the declared access mode, not a defect in the answer
packet: sha256 058e3de6f7bd
calls: 3 of ~3 for WP08 (ephemeral, not resumable)
verdict: conditional — constructions 1, 2, 4 (exact J), 6 (quotient formula) proved sound; construction 3's singular exemption and construction 5's noise-suppression of an observed emitted-turn violation REFUTED; F6 refuted for subnormal homogeneous coordinates; D-a/D-c/D-d/D-f accepted, D-b accepted for exact J only, D-e K accepted through degree 32 with the corrected op count, 12 accepted only as a work limit
verified by: orchestrator recomputation — F6 multiplier c(1122) = (2·1122+1)u/(1−2·1122u) = 2.492451e-13 (match), Euclidean 4.317051e-4 mm (match), c(2) = 5u/(1−4u) = 5.55e-16; F5 rotation atan2(0.006, 0.048) = 7.125016° (match); C2 E/g for the weighted probe P=((M,0),(M,a),(M,2a)), w=(1,0.8,1): γ22·7.2e9 = 1.7586e-5, g = 1.6a → 0.1099 / 1.099 / 10.99 (match); reversal counterexample: poles (0, 1/3, 1/30, 11/30) unit weights → Q(t) = 1 − 3.8t + 3.8t² ≥ 0.05 > 0 yet degree-5 Bernstein coefficients (1, 0.24, −0.14, −0.14, 0.24, 1) change sign — recomputed (elevating the degree-2 Q to degree 5 gives exactly those); the F6 underflow example fl(2^-1074·1e-5) = 0 is immediate; the corrected op count L = n+8+2h ≤ 2n+9 and the coverage ratio ≥ 2.07 over n = 1…32 accepted as Astra's computation (not independently recomputed)
rejected: the "C2 mismatch" for the packet's unit-weight quadratic — the packet under-specified the fixture (it was the weighted probe), so that item is a packet error, not an implementation error; everything else accepted

## Consequences for the implementation (WP08 round 3, fix round)

1. Construction 5 — delete the `noise >= allowed → skip` branch in `apply_encoded_evidence`: the emitted turn is exact on the emitted points; an observed violation must set `turnViolation` and block `Certified`. Keep the quantization-conditioning number as a separate reported diagnostic. Independently found by the local adversarial review R1(c) (Blocker 1: r=0.1 mm circle at X=8192 mm published `Certified` with 180° turns and `encodedTurnMaxRad = −1`).
2. Construction 3 — `reversalPresent` is NOT evidence of a singularity (regular straight cubic counterexample). The floor-and-reversal exemption from the turn rule may remain only as an "unresolved region" label; the join is NOT exempt from the emitted-turn rule. Genuine singular exemption needs separate evidence (an independently established root bracket).
3. Construction 4 — J from computed tangents needs an interval: certify `θ_upper ≤ tol + J_lower`, violation iff `θ_lower > tol + J_upper`, else `Undefined`; at a subdivision boundary of one regular span use the proven `J = 0`.
4. Construction 6 — detect subnormal/underflowing homogeneous coordinates (`fl(w·P) == 0` with `P ≠ 0`, or any product below the normal range) and refuse certification; move `control_net_is_valid` ahead of the `< 2 poles → 0` shortcut in `hull_chord_bound`. The double charge is sharp for "exact chord vs computed chord" and conservative (one δ suffices) for "exact curve vs the fixed computed chord".
5. Construction 1 — replace the `2n + 9` comment with the corrected count (`L = n + 8 + 2h`, `h = max(0, 2n−57)`, `H = L + 4`; Pascal rows ≥ 57 round); K = 3n + 16 stands through degree 32.
6. Construction 2 — computed zeros must not mint singular endpoint labels (`CurveSampler.cpp:~468`); "β > tol/2" disproves the computed hull's containment, not the curve's.

## 1. Per-step verdicts: constructions 1–6

### 1 — C2 coefficient uncertainty: conditionally sound; incomplete as an unrestricted certificate

**Proved:** `2n + 9` is a valid upper bound for the inspected implementation’s supported degrees 1–32, but the comment’s binomial accounting is incomplete. Pascal-table construction also rounds. [Source](worker/src/tess/CurveSampler.cpp:80)

A corrected count:

- Pascal rows through 56 are exact: their integer entries are below 2^53.
- For denominator row `2n−1`, let `h = max(0, 2n−57)`. Its relative error is bounded by γ_h; reciprocal error can conservatively use γ_(2h).
- Each Bernstein product coefficient contains at most **n**, not 2n, terms.
- A sufficient signed-coefficient count is `L = n + 8 + 2h`.
- Including norm evaluation and the final magnitude sum, a sufficient magnitude count is `H = L + 4`.

Thus `L ≤ 2n+9`; at n=32, `L=54`, `H=58`, `K=112`.

**Proved, conditional on normal-range arithmetic:** let S be the exact absolute majorant and Ŝ its computed value. Then

\[
\|\widehat q_k-q_k\|\le\gamma_L S,\qquad
\widehat S\ge(1-\gamma_H)S.
\]

Allowing downward rounding in γ_K and its final multiplication, sufficient coverage is

\[
(1-u)^2\gamma_K(1-\gamma_H)\ge\gamma_L.
\]

**Numeric:** the left/right ratio is at least **2.074074** over n=1…32. Consequently, **computed absolute sums need not themselves be outward-rounded here: the demonstrated K headroom covers their rounding.** Merely saying “headroom” was not the proof.

**Remaining gaps:**

- Relative γ bounds do not cover arbitrary underflow or overflow.
- This bounds Q for the **stored input net**. It does not automatically include errors inherited from subdivision, weight normalization, or conversion relative to the original curve.
- Subsequent normalization, dot products, angles and acceptance comparisons need their own enclosures.

**Smallest closing changes:** replace the comment with the corrected count; guard exceptional arithmetic or use interval fallback; propagate inherited homogeneous-coefficient errors when certifying the original curve.

### 2 — Retained mass and one global error charge: faithful and mathematically sound

**Proved:** with excluded set Z,

\[
s(t)=\sum_{k\notin Z}B_k(t)
=1-\sum_{k\in Z}B_k(t)
\ge1-\sum_{k\in Z}\max_t B_k(t)=s_{\min}.
\]

The bound can be negative although actual retained mass is positive. That costs certification coverage, not soundness.

Let

\[
T(t)=\sum_{\rm retained}B_k(t)\widehat q_k.
\]

Provided retained coefficients lie within β of the axis,

\[
\angle(T,d)\le\beta,\qquad
\|T\|\ge T\cdot d\ge p_{\min}s_{\min}=g.
\]

For coefficient errors bounded by ε_k,

\[
\left\|\sum_kB_k(t)(q_k-\widehat q_k)\right\|
\le\sum_kB_k(t)\epsilon_k
\le\max_k\epsilon_k.
\]

Therefore

\[
\|Q-T\|\le E,\qquad
\angle(Q,d)\le\beta+\asin(E/g),\quad E<g.
\]

**D-f is sound.** Retained coefficients need no additional individual β enlargement or pMin reduction; that would duplicate the already charged perturbation.

**Implementation gaps:**

- `sMin`, g must be lower bounds; E and β must be upper bounds. Ordinary rounded `pow`, dot products and angles do not establish this at decision boundaries.
- `β > tol/2` disproves containment of the **computed coefficient hull**, not necessarily containment of Q(t).
- A computed zero is not proof of an exact zero or singularity. The source nevertheless assigns singular endpoint labels from computed zeros. [Source](worker/src/tess/CurveSampler.cpp:468)

**Smallest change:** use enclosed predicates; describe failed hull tests as failed certificates; require separate evidence for singular labels. The dimensionless `1e-12` weight guard is a conservative downgrade policy, not a theorem establishing arithmetic accuracy.

### 3 — Two cones: sound publication policy; singular exemption is insufficiently justified

**Proved:** using the double chord for refinement and the emitted chord for publication is sound, conditional on valid cone certificates and the positional certificate. An emitted failure may honestly become `Undefined`; it need not force refinement.

**Proved:** two sound `Satisfied` certificates at a regular join imply the requested turn bound. With common exact tangent direction τ and emitted chord directions c−, c+,

\[
\angle(c_-,c_+)
\le\angle(c_-,\tau)+\angle(\tau,c_+)
\le\tfrac{\mathrm{tol}}2+\tfrac{\mathrm{tol}}2
=\mathrm{tol}.
\]

This requires certificates about the same exact curve, including endpoint tangents. Current enum values alone do not establish those premises.

**Refuted:** coefficient reversal proves a singularity. Consider the scalar cubic with unit weights and poles

\[
P=(0,\tfrac13,\tfrac1{30},\tfrac{11}{30}).
\]

Its derivative is

\[
Q(t)=1-3.8t+3.8t^2\ge0.05,
\]

but its degree-5 Bernstein coefficients are

\[
(1,\;0.24,\;-0.14,\;-0.14,\;0.24,\;1).
\]

The curve is a regular, monotonically traversed straight line, yet `reversalPresent=true`. Scaling poles by 10^-4 mm gives chord length **3.666667×10^-5 mm**, below the fine-budget floor **4.8828125×10^-5 mm**.

**Smallest change:** retain “possible unresolved singular region” as an uncertainty label, but do not treat it as proof permitting exemption from the regular emitted-turn rule. A genuine singular exemption needs separate evidence.

### 4 — Encoded-turn rule and genuine jump: correct exact formula; computed J needs an interval

**Proved:** for exact one-sided tangents τ−, τ+,

\[
\theta\le\angle(c_-,\tau_-)+J+\angle(\tau_+,c_+)
\le\mathrm{tol}+J.
\]

Thus allowing the genuine C0 jump is correct.

Let the recorded tangent directions have errors η−, η+, and angle evaluation contribute η_eval. Then

\[
|J-\widehat J|\le\eta_J=\eta_-+\eta_++\eta_{\rm eval}.
\]

For an endpoint numerator enclosure ε around nonzero \(\widehat q\),

\[
\eta\le\asin(\epsilon/\|\widehat q\|),\qquad \epsilon<\|\widehat q\|.
\]

This must include inherited input errors. Otherwise the tangent is unresolved.

**Crucial distinction:**

- An upper envelope implied by cones uses `tol + J_upper`.
- Certifying the exact excess contract `θ−J ≤ tol` requires  
  **`θ_upper ≤ tol + J_lower`**.
- A definite violation requires `θ_lower > tol + J_upper`.
- Overlapping intervals mean `Undefined`.

Simply increasing the allowed threshold by J’s uncertainty silently relaxes the exact excess contract.

**Smallest change:** record a J interval and use these comparisons. At a boundary known to subdivide one regular analytic span, use the proven value **J=0**; independently computed child tangents need not agree numerically.

Splitting only the current span is a bounded repair heuristic. Final checking must remain authoritative.

### 5 — Turn measurability: direction bound sound; suppression of emitted violations is not

**Proved:** write the original chord v, encoded chord v̂, `L=‖v̂‖`, and `R=e−+e+`. Since `‖v−v̂‖≤R`, for `R<L`,

\[
\angle(v,\widehat v)\le\asin(R/L).
\]

For generic perturbations, clamping at π/2 would fail when R>L. **For this particular `encode32`, it is valid:** coordinatewise rounding is monotone, so

\[
(b_j-a_j)(\operatorname{encode32}(b_j)-\operatorname{encode32}(a_j))\ge0.
\]

Consequently, nonzero original and encoded chords cannot make an obtuse angle. Summing the two bounds therefore bounds the change between original and encoded turns.

**Refuted:** `noise ≥ allowed` proves the emitted turn is unmeasurable or cannot be a violation. The emitted vertices determine that turn. A large upper bound establishes neither large actual rotation nor inability to measure an excess.

**Disposition:**

- Withdraw the curve-related angular claim: **accept**.
- Stop refinement as a declared policy: **accept**.
- Suppress an observed violation of the explicitly emitted-turn contract: **reject**.

**Smallest change:** retain separate emitted-excess and quantization-conditioning diagnostics; test emitted excess before the noise downgrade. Bound measured norms, lengths and angle evaluation outward.

**Source deviation:** subdivision currently performs its turn-driven split without the noise test; the noise test occurs afterward. Thus the packet’s “unmeasurable → no split” policy is not implemented during subdivision. [Subdivision](worker/src/tess/CurveSampler.cpp:1195), [post-emission check](worker/src/tess/CurveSampler.cpp:1439)

### 6 — F6 enclosure: quotient formula sound; full-domain sufficiency refuted

**Proved, conditional:** suppose the homogeneous recurrence establishes

\[
|\widehat X-X|\le gMW,\quad
|\widehat W-W|\le gW,\quad |X|\le MW.
\]

Including final division rounding gives

\[
|\widehat P-P|
\le M\frac{2g+u(1+g)}{1-g}.
\]

Ancestor extents supply M even when descendant coordinates cancel. For midpoint subdivision, exact powers-of-two scaling and normal arithmetic permit an operation count including midpoint additions, rehomogenization and weight normalization. That is the required foundation for `k=d(n+3)+2`.

**Refuted without arithmetic-range restrictions:** take degree 1,

- `w=(2^-1074,1)`;
- `P=(10^-5,2×10^-5) mm`.

The first homogeneous coordinate rounds to zero:

\[
\operatorname{fl}(2^{-1074}10^{-5})=0.
\]

After subdivision, its retained endpoint becomes `0/w0=0`, although the exact endpoint is 10^-5 mm. Computed weights remain positive and poles finite.

At d=1, k=6, the proposed enclosure is only

\[
\frac{13u}{1-12u}\,2\cdot10^{-5}
=2.886579864\times10^{-20}\ {\rm mm}.
\]

Even its double charge misses the **10^-5 mm** discrepancy.

**Double charge — precise verdict:** if every pole error is at most δ, then

\[
|h(P,\text{endpoint chord})-
h(\widehat P,\text{computed endpoint chord})|\le2\delta.
\]

One δ covers pole movement; one covers chord movement. The factor 2 is sharp for these independently perturbed objects.

However, for distance of the exact curve to the **fixed computed chord**, one δ already suffices:

\[
\operatorname{dist}(C,\widehat{\text{chord}})
\le h(\widehat P,\widehat{\text{chord}})+\delta.
\]

Therefore double charging is conservative for that target, not universally necessary. Neither argument covers unbounded roundoff in evaluating the hull-distance formula itself.

**Smallest changes:** detect unsupported homogeneous arithmetic or use interval/exponent-scaled fallback; enclose hull-distance evaluation; initialize and preserve ancestor extents.

**Additional source mismatch:** `hull_chord_bound` returns zero for fewer than two poles before validating the net. Move validation before that shortcut to make “invalid → infinity” universally true. [Source](worker/src/tess/CurveSampler.cpp:360)

## 2. Deviation verdicts D-a … D-f

| Deviation | Verdict |
|---|---|
| **D-a** | **Accept**, conditional on the final positional budget including emission error. `Undefined`, no invented singularity, continued turn checking is honest. Numeric: `2^-32=2.328306437×10^-10`; resolving a 10^-12 parameter scale by halving generally needs 40 levels. |
| **D-b** | **Accept exact J; reject unchecked computed J.** Use the interval comparisons in construction 4. |
| **D-c** | **Accept.** Separates geometric refinement from certification of emitted directions. Cost: some positionally valid leaves remain angularly `Undefined`. |
| **D-d** | **Accept as heuristic, not convergence guarantee.** It is traversal-asymmetric. Refining the current span cannot necessarily repair the committed chord’s error. |
| **D-e** | **K accepted conditionally:** the corrected derivation above supplies a bound through degree 32. **12 accepted only as a work limit.** No universal successful split count follows from the supplied inputs. Depth/segment caps prove termination; “12 attempts, then unresolved if still failing” is honest. |
| **D-f** | **Accept.** Bernstein partition of unity justifies one maximum coefficient-error charge. Individual retained-coefficient inflation is unnecessary. |

**Proved, tolerance-policy check:** with consistent length-unit scaling, E and g scale together, positional bounds and budgets scale together, and angles remain dimensionless. The floor cannot certify angular quality. Translation leaves exact Q unchanged but can enlarge its arithmetic majorant; float32 emission likewise depends on coordinate origin. General unit changes need not preserve identical rounding decisions.

## 3. Recomputed numbers

### F6

**Numeric — match.** Algebraically,

\[
c(k)=\frac{(2k+1)u}{1-2ku}.
\]

| Case | Recomputed result |
|---|---|
| n=32, d=32; k=1122 | `c=2.492450690284097×10^-13` |
| 10^9 mm per axis | `c√3×10^9 = 4.317051230932176×10^-4 mm` |
| Double positional charge | `8.634102461864353×10^-4 mm` |
| n=3, d=0; k=2 | `c=5u/(1−4u)=5.551115123125785×10^-16` |
| Corresponding 10^9 mm per-axis Euclidean enclosure | `9.614813431917822×10^-7 mm` |

**Numeric — 0.0524 match using locally recovered fixture.** I recomputed its homogeneous reference with exact dyadic arithmetic. The largest discrepancy against the reference rounded to binary64 was

\[
\frac{2.384185791015625\times10^{-7}}
     {4.551914451034221\times10^{-6}}
=0.0523776493751.
\]

It occurs at translation 10^9 mm, depth 3. Comparing against the unrounded exact reference instead gives ratio **0.0471398844380**.

**Source-description mismatch:** the inspected test uses **double-double**, then rounds the reference back to binary64 for this comparison; it is not a long-double comparison. This remains empirical coverage, not a universal enclosure proof. [Fixture and comparison](worker/tests/test_curve_sampler.cpp:2399)

### F5

**Numeric — match.**

\[
\widehat A=(100000,0,0),\quad
\widehat B=(100000,0.04800000041723251,0).
\]

\[
L=\sqrt{0.005999999993946403^2+0.048^2}
=0.04837354648904045\ {\rm mm}.
\]

Endpoint displacements are **0.00299999999697320 mm** and **0.00299999999697323 mm**.

\[
\text{rotation}
=\atan2(0.005999999993946403,0.048)
=7.125016341787019^\circ.
\]

This exceeds the emitted cone’s 2.5° allowance, so publication as `Undefined` matches the policy.

### C2

**Numeric — mismatch for the packet’s unit-weight quadratic.** Writing its vertical offset as a,

\[
q=((0,2a),(40/3,2a/3),(80/3,-2a/3),(40,-2a)).
\]

The double chord points along x. Hence `q0·axis=0`, triggering reversal and `Uncertified`; the advertised E/g decision is never reached.

**Numeric — matches recovered weighted probe.** The local probe instead specifies

\[
P=((M,0),(M,a),(M,2a)),\quad w=(1,0.8,1),\quad axis=(0,1).
\]

[Probe inputs](scratchpad probe_c2_new.cpp:10)

To the reported precision,

\[
E=\gamma_{22}(7.2\times10^9)
=1.7585932710062524\times10^{-5},\qquad g=1.6a.
\]

| a (mm) | E/g | Verdict |
|---|---:|---|
| 10^-4 | 0.1099120794379 | **Match**; `asin(E/g)=6.31024736° > 2.5°` |
| 10^-5 | 1.099120794379 | **Match**; E≥g |
| 10^-6 | 10.99120794379 | **Match**; E≥g |

The packet alone lacked the historical weighted fixture and axis; the local probe supplies them.

### Remaining measurements

**Not computable from the supplied measurement summaries:** C1’s 34 segments, 2.9660° and singular-label counts; cusp’s 70 points and bracket endpoints; S-curve’s 150 segments and 4.6429°; circle’s actual 128-segment emitted maximum. Reproducing these requires the corresponding accepted parameter partitions/emitted vertices and run configuration.

**Numeric diagnostic, not a mismatch:** a uniformly spaced 128-gon on a radius-5 mm circle has turn `360°/128=2.8125°` and sagitta `5(1−cos(π/128))=0.001505906519 mm`. Uniform angular spacing was not specified, so this does not refute 2.9660°.

## 4. Verified versus inferred

**Verified**

- **Proved:** retained-mass lower bound, one maximum error charge, conditional cone implication, regular-join theorem and exact tangent-jump allowance.
- **Proved:** quantization direction bound, including the monotonicity justification for its clamp.
- **Proved:** conditional F6 quotient enclosure and the precise scope of its double charge.
- **Refuted:** reversal implies singularity; large noise erases an emitted excess; finite positive weights alone suffice for the F6 enclosure.
- **Numeric:** F6 multipliers, recovered discrepancy ratio, F5 geometry and rotation, recovered weighted C2 ratios.
- **Source-checked:** the specific implementation discrepancies cited above.

**Inferred or unverified**

- Historical faithfulness to session `01a09bb1-4f33-7512-b342-3e15bae38282`: its derivation text was not available.
- End-to-end certification of original-curve tangents: inherited-error and predicate enclosures remain incomplete.
- Universal success after 12 splits: unsupported.
- Remaining reported sampler counts and maxima: not independently reproduced.
- Deterministic output bytes, MESH1 compatibility and parity/canonical targets: not rerun.

## 5. If nothing qualifies

Several mathematical constructions qualify conditionally. **An unconditional, fully faithful certification of all six implementations does not.**