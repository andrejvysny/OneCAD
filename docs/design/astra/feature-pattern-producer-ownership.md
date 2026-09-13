# feature-pattern-producer-ownership
date: 2026-09-13
mode: derive
model: gpt-6-astra
effort: xhigh
access: grounded: worker/src/ops/TopologyHistory.cpp, worker/src/session/TopologyOrigins.h, worker/src/ops/FilletChamferOp.cpp, worker/src/ops/HoleOp.cpp, worker/src/ops/ExtrudeOp.cpp, worker/src/session/FeaturePatternIdentity.cpp, worker/src/session/FeaturePattern.cpp, worker/src/session/PlanExecutor.cpp, worker/tests/test_feature_pattern.cpp, worker/tests/test_topology_origins.cpp, protocol/SCHEMA.md (2439-2500), worker/src/kernel/fillet/EdgeContour.cpp. Transcript shows one read outside the repo: Codex's own bootstrap scan for AGENTS.md in parent directories (none exist there); no other path was read.
packet: sha256 1b00fee1a827
calls: 1 of ~3 for WP-A2 (feature-pattern producer ownership); session 01a09a6e-735e-74e2-b7ce-41ee83a50d18 recorded for followup
verdict: n/a (derive). Astra's own confidence: medium — high on the conditional rule and the information-insufficiency proof, medium that boundary completion resolves the measured seed, low on unmeasured degeneracies. Three of its six named unknowns were measured by Fable the same day (below) and support the rule.
verified by: temporary stderr probes in worker/tests/test_feature_pattern.cpp (source restored, `git status` clean), worker built from HEAD 65b4c60 on OCCT 8.0.1; raw output preserved as docs/qa/evidence/ux-hardening-2026-09-11/fp-ownership-probes-2026-09-13.log. PROBE1/2: both failing assertions fail on the first conjunct with `UNSUPPORTED_OP … supports only straight-edge Fillet/Chamfer inputs` at instance 1. PROBE3: the i-2 Chamfer's closure = [line Known(record_add), circle Unknown, line Known(record_add)]. PROBE4/5 (BRepFilletAPI_MakeFillet, r 0.5, seed edge #21): 31 after-edges = 23 IsSame survivors + 4 `Modified` successors (e10→#1, e11→#4, e22→#12, e25→#18) + 4 orphans (#2 circle, #10 line, #13 line, #14 circle), every orphan on the single `Generated(e21)` face; 22 after-vertices = 18 survivors + 4 orphans. `Generated(vertex)` empty for every vertex. PROBE6 (BRepFilletAPI_MakeChamfer, d 0.5, same edge): identical structure with #2/#14 straight. PROBE7 (second fillet on the modified top edge, tangent chain e12→e14→e18 through the first fillet's end-arc): three generated faces, one per chain edge including `Generated(e14)` for the first blend's orphan arc; every vanished before-edge has non-empty `Modified` or `Generated`; every orphan edge lies on at least one generated face; orphans shared by two generated faces of the same op (#10, #21) occur once; one single-face orphan (#20, curve type 8, the blend seam). Astra's 23-survivor count (31 − 4 − 4) is confirmed by the dump.
rejected: (1) "Consumed same-producer twin" matcher counterexample — real, but outside this package's path (match_feature_pattern_produced_ref, resolver-side); recorded as a follow-up in TODO.md, not fixed here. (2) "Small-edit instability of the 1e-6/1e-9 thresholds" — predicate-level observation on epsilons this package must not re-derive; recorded, not acted on. (3) Extending boundary completion to boolean adapters (Hole/Extrude) — deferred until they carry their own provenance certificate, as Astra itself required.

## 1. Problem restatement (accepted)

Complete ownership downward from operation-born faces, only for topology lacking direct lineage. Generated-face containment proves incidence, not historical birth; on the four observables alone (IsSame, Modified, Generated, containment) a genuinely new blend boundary and an unreported replacement of a before-edge are indistinguishable, so unconditional attribution cannot be proved — a certificate is required.

## 2. Properties to guarantee (accepted, checkable)

- Exact lineage first: containment never changes an output already classified survivor / successor / birth / conflict.
- Certified completion: an omitted edge or vertex becomes `Known(O)` only with a live operation-born face witness AND a certificate that omitted predecessor history is not hiding a replacement.
- Shape-local conflicts: direct birth plus predecessor → `Ambiguous`; incompatible predecessor producers → `Ambiguous`.
- No ownership propagation through adjacency: an adjacent Unknown host face neither supplies host ownership nor contaminates a certified birth.
- No inferred ownership without evidence: missing witness or missing certificate → `Unknown`.
- Shared boundaries: several witnesses from one operation produce one claim, never ambiguity.
- Live membership; canonical consumption (producer output sets are built from the EFFECTIVE claim per IsSame class, i.e. `lookup()`, not raw rows); determinism (reordering equivalent history lists cannot change effective claims); transactionality; binding isolation (Unknown/Ambiguous never enables host support or whole-body fallback).
- `inherit_origin` must be order-insensitive: scan all predecessors; any distinct Known producers → Ambiguous; otherwise any Unknown → Unknown; otherwise the common producer. (Today `[Known(A),Known(B),Unknown]` → Ambiguous but `[Unknown,Known(A),Known(B)]` → Unknown.)

## 3. Predicates (accepted; epsilon: none — purely combinatorial on IsSame)

```
P(o) = { b ∈ before[K] : IsSame(o,b) or o ∈ Modified(b) }
D(o) = direct birth (Generated of any before root; or Generated/Modified/self of birth_roots)
F    = { f ∈ after[FACE] : D(f) and P(f) = ∅ }                 -- clean birth faces
C(o) = kind(o) ∈ {EDGE, VERTEX} and ∃ f ∈ F : o ∈ MapShapes(f, kind(o))
Omitted(o) = P(o) = ∅ and not D(o)

Claim(o) = Ambiguous      if D(o) and P(o) ≠ ∅
         | Known(O)       if D(o) and P(o) = ∅
         | inherit(P(o))  if not D(o) and P(o) ≠ ∅
         | Known(O)       if Omitted(o) and C(o) and Certified(o)
         | Unknown        otherwise
```

**Certificate adopted for the blend adapters (Fillet, Chamfer), Fable's decision grounded in PROBE5/6/7:**

```
Certified_blend(o) =
      the witness face f ∈ Generated(e) for a before-edge e in THIS op's accepted contour set
  and o is not IsSame any before sub-shape
  and AccountedVanishing(before, after): every before FACE and before EDGE absent from `after`
      (not IsSame any after shape) has non-empty Modified(b) or Generated(b)
```

Rationale: in all three measured configurations every before-edge that changed was reported (trimmed neighbours as `Modified`, consumed contour edges as `Generated` → face) and every orphan lay on a generated face. `AccountedVanishing` is the fail-closed guard against the unproved adversary (a vanished edge with empty history = a possible unreported replacement): when it fails, no completion happens for that op and downstream references become `NeedsRepair` with `unknown-origin`. Vertices carry no OCCT history in blends (PROBE5: `Generated(vertex)` empty everywhere), so vertex completion is derived from edges: an orphan vertex is `Known(O)` when it is a vertex of a certified orphan edge or lies on a witness face and the edge-level guard passed. Booleans (Hole, Extrude Add/Cut) keep their existing `birth_roots` rule and get no boundary completion in this package.

Consequences (proved by Astra): an orphan shared by a generated face and a Modified host face satisfies C (the host face is not an edge predecessor); membership in a conflicted face alone is insufficient; completion only visits omitted outputs so it cannot create conflicts with inherited children.

## 4. Degeneracy classes (accepted; "certified birth" includes the certificate)

| Class | Detection | Ledger result |
|---|---|---|
| Whole input edge consumed | input absent from after; `Generated(input)` = blend face | old claim removed; generated face and certified orphan boundaries → current op; the consumed reference must never rebind |
| Edge split | several live outputs in `Modified(edge)` | each inherits that edge's producer; splitting is not birth |
| Two blends share an end-arc, same op | one output arc on the boundaries of two clean birth faces | one `Known(O)` (PROBE7 #10, #21) |
| Sequential blends share an end-arc | later output is an exact survivor or reported successor of the earlier arc | keep the earlier producer; direct later birth + predecessor → Ambiguous. Measured: the later blend CONSUMES the earlier arc (`Generated(e14)` → face) and its offset arc is a certified orphan of the later op |
| Adjacent face Unknown vs Add-owned | same edge history, different face claims | identical edge classification; face ownership never inherited downward |
| Chamfer vs Fillet | actual builder history yields a clean birth-face witness | same rule; measured identical (PROBE6) |
| Multi-edge tangent contour | union of accepted contour edge sets | classify every resulting shape independently; never assign the contour to its seed's producer |
| Blend seam / single-face edge | orphan on exactly one generated face (PROBE7 #20, curve type 8) | certified orphan → `Known(O)` |
| Blend removes a face | before-face absent, no live successor | drop the face claim; classify its surviving children independently |
| Boolean rebuilds only host boundaries | host face has Modified output; orphan has no clean birth-face witness | reported successors inherit; omitted edges stay Unknown |
| Boolean mixes host and tool lineage on one edge | direct tool birth AND host predecessor | Ambiguous, even when the host predecessor is Unknown |
| Vanished before-edge with empty history | `AccountedVanishing` false | no completion for this op; orphans stay Unknown (fail closed) |

## 5. Algorithm sketch (accepted)

1. Collect direct P/D relations exactly as today (`classify_kind`). 2. Classify faces first; collect clean, live birth-face witnesses for THIS op. 3. Evaluate the certificate once per op (AccountedVanishing over faces and edges). 4. Index witness faces' edges and vertices by IsSame. 5. Complete only omitted edge/vertex outputs with a witness; vertices via certified edges. 6. Leave everything else omitted (Unknown). 7. Producer refresh consumes effective `lookup()` claims. Location: `modified_body_history` (TopologyHistory.cpp) with the certificate supplied by the blend adapter, before `apply_topology_history`. **Never pass generated-face children as `birth_roots`** — that would turn inherited children into births and manufacture conflicts. Complexity ≤ A × G (after sub-shapes × witness faces), plus one pass over before faces/edges for the certificate.

## 6. Test vectors (accepted, pinned by PROBE5)

After the seed Fillet (A = Known(record_add), F = Known(record_modifier)): #1 A (successor of e10) · #2 F (orphan circle, (−7.75, 9.75, 5)) · #4 A (successor of e11) · #10 F (orphan line, (−7.5, 10, 7.5)) · #12 A (successor of e22) · #13 F (orphan line, (−8, 9.5, 7.5)) · #14 F (orphan circle, (−7.75, 9.75, 10)) · #18 A (successor of e25) · the other 23 edges carry `L0.lookup(body_host, survivor)` (host edges Unknown, Add-owned edges A). Orphan vertices (−7.5,10,5), (−8,9.5,5), (−7.5,10,10), (−8,9.5,10) → F. The Chamfer's three resolved producers become `[record_add, record_modifier, record_add]`, and at instance k `[virtual(add,k), virtual(fillet,k), virtual(add,k)]`; the circle's translated centre at spacing 20 is x = −7.75 + 20k (12.25, 32.25 mm). Spacing 90: instance 2 translates by 180 mm, the seed corner x = −8 → 172 mm, beyond the 100 mm host edge; the test must assert `Failed`, the context `instance 2 source record_add index 1`, and exact restoration of geometry signature, body ids and shapes, partition bindings (`same_partition_body`), the ownership ledger, resolved-input evidence, sketch state and `last_sketch_id`, with empty `body_events`/`body_ids`/`delta`.

## 7. Counterexample search (accepted)

Unreported host replacement is indistinguishable from a new boundary on the four observables — hence the certificate. The blend certificate's `AccountedVanishing` clause turns that adversary into a refusal rather than a wrong bind. Membership in a Modified host face alone must never confer ownership.

## 8. Implementation handoff (accepted, C++20 worker)

- `TopologyHistory.cpp`: `modified_body_history(body_id, before, result, builder, birth_roots, const BoundaryCompletionCertificate* certificate = nullptr)`; when a certificate is supplied and `AccountedVanishing` holds, complete omitted edges/vertices lying on witness faces as births. `struct BoundaryCompletionCertificate { std::vector<TopoDS_Shape> contour_edges; }` — the op's accepted contour edges; witness faces = `Generated(e)` for those edges.
- `TopologyOrigins.h`: `inherit_origin` order-insensitive as in §2.
- `FeaturePatternIdentity.cpp`: `feature_pattern_refresh_producer_topology` builds `produced[]` from `lookup()` (effective claim) per IsSame class; `FeaturePattern.cpp`: evaluate resolved-input origin (Unknown/Ambiguous → `NeedsRepair` with reason `unknown-origin`/`ambiguous-origin` through the existing `FEATURE_PATTERN_PRODUCER_BIND` shape) BEFORE the straight-edge capability refusal (`UNSUPPORTED_OP`), which stays for genuinely unsupported curve types.
- Logging: bounded stderr (`WLOG_*`) per op: body, output kind + ordinal, predecessor ordinals, witness faces, certificate result, final claim. No stdout, no pointer identities, no geometric guesses.
- Diagnostics: no new top-level code; no wire change.

## 9. Confidence and unknowns

Astra: medium overall. Fable after measurement: the rule plus the blend certificate is adopted for Fillet/Chamfer. Remaining unknowns, carried to the `break` call: (a) a blend terminating exactly on a pre-existing coincident boundary (not constructible in the box seed; the certificate's vanishing guard is the defence); (b) whether the spacing-90 instance fails at the Add step with the expected context (measured only after the fix); (c) the consumed same-producer twin in the matcher (follow-up, separate package).

---

## break record (call 2 of ~3)
date: 2026-09-13 · mode: break · model: gpt-6-astra · effort: xhigh · access: grounded (the derive allowlist plus the two test files and the diff inline) — transcript shows one extra read of a repo-local skill file `.agents/skills/caveman-review/SKILL.md` by Codex's bootstrap; no expected values, no contamination · packet: sha256 fc0f11eaf38d · session 01a09a97-b9c0-7040-b0f4-f4547b08e2f5
verdict: defective as an UNCONDITIONAL safety derivation — the certificate does not establish predecessor completeness, and the retained-host bind bypasses current ownership. Fable's disposition follows.

| # | Finding | Severity | Ruling | Closure |
|---|---|---|---|---|
| F1 | `accounted_vanishing` checks non-empty, not complete, history: a vanished edge whose `Modified` reports one of two successors lets the second (a host piece) be completed as blend-born; vertices are never examined; non-live acknowledgements pass | high (proof gap; OCCT reachability inference) | accepted in part | Add the live-output clause (a vanished before face/edge must have at least one reported output present in `after`). The partially reported split is not detectable from IsSame/Modified/Generated/containment and is an **accepted residual**: PROBE5/6/7 show complete reporting in the measured configurations, the mislabelled piece would be the host remainder adjacent to the instance's own blend, so a reference bound through it replays the seed's geometry under the instance transform rather than teleporting; recorded in TODO.md as a residual with the test Astra asks for (PartialHistoryBuilder partial-split case pinned as "must stay refused" if provenance ever becomes available). Vertex replacement: blends report no vertex history, so a vertex guard would disable completion always; residual. |
| F2 | Retained-host branch in `feature_pattern_bind_instance_output_refs` accepts a face on historical evidence `Known(unselected)` without checking the live face's effective `lookup()` | high (proved bypass) | accepted | Require `lookup(body, existing->shape)` to be `Known` with the same unselected producer as the evidence; otherwise `ownership-mismatch` repair. Test: live ledger Unknown and Ambiguous variants → `FEATURE_PATTERN_PRODUCER_BIND`, full outer rollback. |
| F3 | Descriptor centres are bounding-box centres; `transform_descriptor` rotates them; for a quarter arc r = 0.5 mm rotated 45° the error is r(1−1/√2)/2 = 0.0732 mm ≫ 1e-6, so the intended arc is rejected and a perpendicular twin with equal length/size can match uniquely under a Circular layout | medium (numeric; wrong-bind construction inference) | accepted | Fillet-produced circular inputs (the class this package newly admits) are supported only under `Linear` layouts (translation is bbox-equivariant); under `Circular` they refuse with `UNSUPPORTED_OP`. Hole circles (full circles, bbox centre = circle centre) keep the rotational branch. Test: Circular layout with a fillet-arc chamfer input → refusal, never a bind. |
| F4 | Witnesses come only from `Generated(contour edge)`; a corner face reported under `Generated(vertex)` (three incident blends) is not a witness, so its exclusive seam/pole orphans stay Unknown | medium (proved conditional gap) | accepted | Witnesses = every clean generated face of THIS op (Generated of any before face/edge/vertex, not IsSame a before face, not a Modified output), still gated by the op certificate — this is the derivation's own F = {D(f) ∧ P(f)=∅}. Tests: synthetic vertex-generated corner face in `PartialHistoryBuilder`; kernel probe 10 mm cube with 1 mm fillets on three incident edges asserting every orphan edge lies on a generated face and none stays Unknown. |
| — | Refresh cost 127·(1+2+3+4)·A·L ≈ 1.27e9 row inspections at A = L = 1000 | medium (numeric, no timing) | accepted | Build an IsSame-keyed index of the body's effective claims once per refresh (O(A + L)). |
| — | Non-manifold incidence, replaced vertex at an existing termination | low | recorded | Solids only in this repository; residual noted. |

verified by Fable: F2 read at `FeaturePatternIdentity.cpp` retained branch (historical evidence only, no `lookup`); F3 arithmetic recomputed (bbox centre of a 0..90° quarter arc of r = 0.5 is (0.25, 0.25); rotated 45° → (0, 0.3536); true rotated bbox centre → (0, 0.4268); Δ = 0.0732 mm); F4 read at `certified_completion` (witness loop iterates `certificate.contour_edges` only). F1's live-output clause and the residual argument are Fable's.
