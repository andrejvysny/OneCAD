# OneCAD Modeling Correctness Risk Register

Baseline: `1c11d4958aeadea14dd8431ba78c41f14be12142`

Scales:

- Severity: 5 = data loss or silent wrong geometry; 1 = documentation only.
- Likelihood: 5 = normal exposed path; 1 = dormant/uncommon.
- Detectability: 5 = likely silent; 1 = immediate loud failure.
- Priority score: severity × likelihood × detectability. It is a sorting aid, not a substitute for dependencies.

| ID | Finding | Class | S | L | D | Score | Confidence | Dependency | Recommended phase |
|---|---|---|---:|---:|---:|---:|---|---|---|
| R-01 | Dirty Open bypasses unsaved guard | Defect | 5 | 4 | 5 | 100 | Proven source path | none | 0 |
| R-02 | Empty Boolean result can publish as modified body | Defect | 5 | 3 | 5 | 75 | Proven source path | product empty-result decision | 0 |
| R-03 | Partial circular-pattern ghost differs from worker commit | Defect | 5 | 3 | 5 | 75 | Proven formula mismatch | normative rule decision | 0 |
| R-04 | Resolved regen failures reported as success | Defect family | 5 | 4 | 4 | 80 | Proven at Pattern/Mirror; multiple cited paths | shared classifier | 0 |
| R-05 | Stale ToFace/Hole promotion becomes anchor-only authoring | Defect/risk | 5 | 3 | 5 | 75 | Proven degradation; wrong bind requires race topology | none | 1 |
| R-06 | PrepareOffsetFace adopts response without current-head fence | Race defect | 5 | 2 | 5 | 50 | Proven missing fence | none | 1 |
| R-07 | Mixed-body Fillet/Chamfer edge can descriptor-bind on target | Silent wrong-bind risk | 5 | 2 | 5 | 50 | Source path proven; geometry repro missing | red-first coincident-body test | 1 |
| R-08 | Repair candidate cache lacks snapshot identity | Silent wrong-bind risk | 5 | 2 | 5 | 50 | Source path proven; repro missing | response echo + cache key | 1 |
| R-09 | Intersected curves become polygon BRep edges | Geometry defect | 5 | 4 | 5 | 100 | Proven implementation and broad-area test | exact fragment representation | 2 |
| R-10 | Curved-wall Draft may succeed unchanged when no planar wall is eligible | Candidate geometry defect | 4 | 2 | 5 | 40 | Face filter proven; OCCT outcome repro required | red probe, then applied-face/semantic check | 0 |
| R-11 | Pattern/Mirror can publish multi-solid under one BodyId | Semantic debt | 5 | 4 | 4 | 80 | Proven current contract | output policy/versioning | 3 |
| R-12 | Hole can leave disconnected host as one multi-solid body | Semantic debt | 4 | 2 | 4 | 32 | Explicitly documented residual | output policy | 3 |
| R-13 | Revolve body-edge axis ElementId is resolved as TopoKey | Dormant defect | 4 | 1 | 2 | 8 | Proven contract mismatch | support/unsupported decision | 1 |
| R-14 | Revolve NewBody lacks common shape publication audit | Validation gap | 4 | 3 | 4 | 48 | Proven source path | common validator | 3 |
| R-15 | Chamfer/Shell/Hole validation is shallower than Fillet/OffsetFace | Systemic asymmetry | 4 | 4 | 4 | 64 | Proven | common validator + performance data | 3 |
| R-16 | Pattern/Mirror/Transform have no meaningful post-op audit | Systemic asymmetry | 4 | 4 | 4 | 64 | Proven | common validator | 3 |
| R-17 | `apply_history` never consults `Generated()` | Identity evidence gap | 3 | 3 | 3 | 27 | Proven | operation-specific lineage design | later/design |
| R-18 | Boolean Intersect lacks direct tests at every layer | Missing evidence | 5 | 4 | 5 | 100 | Proven search/inventory | Phase 0 behavior policy | 4 |
| R-19 | Pattern and body Mirror lack Playwright | Missing evidence | 4 | 4 | 4 | 64 | Proven inventory | Phase 0/3 semantics | 4 |
| R-20 | Mock Playwright does not exercise Tauri/worker composition | Missing evidence | 4 | 4 | 4 | 64 | Architectural fact | targeted Tauri smoke lane | 4/6 |
| R-21 | Kernelbench covers Fillet only | Robustness gap | 4 | 5 | 4 | 80 | Proven enum/suites | publication policy | 5 |
| R-22 | M3 metamorphs schema-only; M4 generic validators absent | Robustness gap | 3 | 5 | 3 | 45 | Proven TODO/code | finish M3 before M4 | 5 |
| R-23 | Corpus cases are not fully executable/discovered | Oracle drift risk | 4 | 3 | 4 | 48 | Proven docs/config | corpus runner | 4 |
| R-24 | Project ledger records checks as not required | Process risk | 4 | 4 | 3 | 48 | `TODO.md:39`; live GitHub ruleset not independently verified | dated settings/ruleset verification | 6 |
| R-25 | Linux Chromium Boolean pick unclassified | Platform risk | 3 | 3 | 3 | 27 | Proven CI comment | runner deps/probe | 6 |
| R-26 | No Windows lane | Platform risk | 4 | 4 | 3 | 48 | Proven CI inventory | Windows artifact strategy | 6 |
| R-27 | Import invalid solids are advisory and post-scale audit is absent | Policy risk | 4 | 2 | 4 | 32 | Proven intentional policy | product import decision | 3 |
| R-28 | Imported exact-tie body order can fall back to traversal order | Determinism risk | 3 | 1 | 4 | 12 | Source analysis; repro missing | identical-solid fixture | 4/5 |
| R-29 | Pattern loops and transforms have weak cancellation/workload ceilings | Reliability risk | 3 | 3 | 3 | 27 | Proven | resource policy | 3 |
| R-30 | Save backend can become clean without frontend adopting the state | UX correctness defect | 4 | 4 | 4 | 64 | Proven command/result shape | authoritative save result | 0 |
| R-31 | Exact preview bodies from multiple region sessions erase one another | Preview defect | 4 | 2 | 4 | 32 | Proven `clearPreviewBody()` behavior | per-session preview ownership | 0/4 |
| R-32 | Boolean failed commit re-arms without rebuilding actionable controls | Recovery defect | 3 | 2 | 2 | 12 | Proven code path | shared re-arm state | 0 |

## Proven sound protections

- Candidate execution snapshots and rolls back on any non-OK outcome.
- Fresh promotion uses atomic snapshot-fenced `BindElementIds`.
- Rust remains the persistent-id/hash authority.
- WorkerEpoch and expectedBaseHash are the correct worker fences.
- Body split ranking and ordinal tripwire are deterministic and explicit.
- Resolver score, margin and edit-scoped tie policy are conservative.
- OffsetFace has strong operation-specific postconditions and history adaptation.
- Fillet has strong semantic diagnostics and deep audit evidence.
- Linux self-hosted jobs are guarded against untrusted fork code.

## Findings deliberately not promoted to defects

- Import BRep invalidity is warning-only by explicit policy. It needs a product decision, not a silent hardening patch.
- The ordinary-edit teleport residual is explicitly accepted. Closing it is outside this roadmap unless reopened.
- Circular Pattern `angle/count` is normative worker/protocol behavior. The defect is frontend divergence, not the worker formula by itself.
- Loft and Sweep are unsupported by design.
- Datum has no kernel op because it is document-side.

## Triage order

1. R-01, R-02, R-03, R-04, R-10, R-30, R-31, R-32.
2. R-05 through R-08 and R-13.
3. R-09.
4. R-11, R-12, R-14 through R-16, R-27, R-29.
5. R-18 through R-23.
6. R-24 through R-26.
7. R-17 and R-28 as design/research riders.
