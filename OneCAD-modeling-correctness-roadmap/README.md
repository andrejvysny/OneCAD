# OneCAD Modeling Correctness Roadmap Bundle

Reviewed baseline: `1c11d4958aeadea14dd8431ba78c41f14be12142`  
Generated: 2026-08-10  
Scope: baseline analysis and planning. Baseline files remain read-only; the
live implementation delta records subsequent repository changes.

## Reading order

1. `04-live-implementation-delta.md` — current repository evidence; supersedes baseline claims when they conflict.
2. `00-current-state-assessment.md` — evidence-backed baseline review and highest-priority findings.
3. `01-operation-correctness-matrix.md` — every supported operation and meaningful mode across test layers.
4. `03-risk-register.md` — severity, likelihood, detectability and dependencies.
5. `02-master-roadmap.md` — phased 3–6 month core and stretch sequencing.
5. `phase-0-safety-and-truth.md`
6. `phase-1-semantic-reference-integrity.md`
7. `phase-2-exact-profile-geometry.md`
8. `phase-3-publication-policy.md`
9. `phase-4-vertical-evidence.md`
10. `phase-5-robustness-breadth.md`
11. `phase-6-required-cross-platform-gates.md`

## Important scope notes

- Phases 0–4 are the recommended committed 3–5 month core for one founder.
- Phase 5 is the preferred robustness stretch when the core lands near the lower estimates.
- Full completion including Phase 6 is more realistically 5–8 months without additional capacity.
- Loft, Sweep, assemblies, FEM, TechDraw, CAM, addon kernel extensions and production advanced-Fillet rescue strategies are excluded.
- The accepted ordinary-edit teleport residual remains unchanged.
- Hole's documented split-host residual is not silently redefined; any new behavior must be versioned.

## Evidence limitations

The review used the connected GitHub repository and a read-only clone. The sandbox did not build OCCT, the worker, Rust, Bun, Playwright or Tauri. Runtime gates in the specifications are instructions for the provisioned developer Mac and trusted Linux CI; they are not claims that those gates passed here.

## Source hierarchy

For implementation status, use `04-live-implementation-delta.md`, then live
repository state (`CURRENT_STATE.md`, `TODO.md`, source and gates). The remaining
bundle files are a reviewed baseline and roadmap, not live-status authority.

## Highest-value first gate

Begin with `MODEL-CORRECTNESS-P0` in `phase-0-safety-and-truth.md`: dirty Open, authoritative save/result truth, zero-solid Boolean refusal, circular-pattern preview parity, multi-session exact previews, actionable Boolean re-arm, and a curved-wall Draft red probe.
