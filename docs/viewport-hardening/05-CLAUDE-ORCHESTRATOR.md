# Claude Code orchestrator prompt — OneCAD viewport hardening

Use the following prompt as the main Claude Code task. Select **Fable 5.1** using the model selection supported by your installed environment and verify the actual routing. No provider-specific model slug is assumed here. The optional project subagent templates use `model: inherit`; their configuration must not silently route implementation to a different model.

The task below assumes this package is available at `docs/viewport-hardening/`. When installed elsewhere, replace only that path in the opening instruction. The source repository remains the user's current checkout.

---

You are the implementation orchestrator for OneCAD VP-HARDENING 1.0. Read `docs/viewport-hardening/00-START-HERE.md`, then the normative specification, implementation guide, numerical/protocol appendix, acceptance plan, and decision matrix in that directory.

Your job is to implement and verify the selected design. The package already decides the architecture, product behavior, algorithms, precision policy, lifecycle rules, and acceptance targets. Do not start a new broad architecture brainstorming session. Do not replace its decisions with shortcuts that are easier to code.

## Mission

Make model rendering, surface shading, model edges, sketch display, camera navigation, sections, previews, and topology acquisition agree. Preserve OneCAD's imperative Three.js/React/Tauri/Rust/OCCT architecture, authoritative document model, and Z-up right-handed coordinates.

The quality target is a trustworthy professional CAD editing viewport. This is not permission to copy another product's source/assets or to claim complete Shapr3D/Fusion feature parity. Correct geometry and interaction are more important than decorative effects.

## First actions

1. Inspect the actual checkout: HEAD, branch, worktree status, pre-existing diffs, installed dependencies, and active build processes. Do not reset, pull, switch branches, or clean anything.
2. Read current root `AGENTS.md`, `CLAUDE.md`, relevant architecture/protocol sections, and current state/handoff/TODO heads. Preserve unrelated user work and historical failure evidence.
3. The reviewed baseline was `65b4c60eb226a201f2fa3eb565d7283b1c63b5ac`. Compare the relevant current files with that baseline without resetting the repository. Existing fixes must be tested and retained, not replayed destructively.
4. Copy the execution ledger template to the package-relative `execution/STATUS.md` if absent. Record environment and evidence, not invented completion percentages.
5. Start WP00. Establish a baseline and red reproductions. Then execute WP01–WP16 in the guide's dependency order, maintaining one working integrated slice at each checkpoint.

## Fixed implementation decisions

Keep WebGL2 as the supported production backend and the currently pinned Three.js/OCCT stack unless an actual documented defect requires a narrow exception. Do not add R3F, an ECS rewrite, a new renderer framework, an alternate native kernel, photorealism, SSAO, or production WebGPU during this program.

Screen-space fat lines use CSS widths, logical-pixel viewport resolution, and CSS pick tolerances. Other device-pixel operations remain explicit. Run the actual installed draw callback in width tests. No module-evaluation DPR constants and no global removal of all DPR handling.

GPU resource ownership is explicit. Whole-body highlights borrow the exact body geometry through a lease. Face/edge highlights own compact buffers and live in a bounded cache. Do not create never-disposed shared-attribute wrappers and do not dispose another owner's buffers. Retire resources after their last consumer, not after an arbitrary delay or global GPU stall.

Frame scheduling consumes dirty reasons before callbacks, preserves reentrant invalidation, and schedules no idle loop. Renderer submission is not compositor presentation. Submission acknowledgments must name the publication actually submitted.

Validate binary structure, semantic indices/ranges/IDs, finite values, bounds, and allocation budgets before installing a mesh. A failed replacement retains previous geometry as explicitly historical inspection-only state; it cannot be promoted to a current operation target.

Resolve materials from authored appearance plus explicit view state. Assembly mode intentionally overrides visible colors without changing stored colors. Ordinary sketch focus stays opaque; active sketch strokes have their deliberate x-ray policy. Fix face ordinal color lookup using `idOf`, not triangle-index `idAt`.

Use a canonical camera state with target, quaternion, apparent scale, projection and FOV. New model views use the specified 35° FOV; preserve existing saved views. Sketch entry aligns the full arbitrary plane basis orthographically and restores the complete prior model view. Clamped zoom uses the effective factor. Manual navigation cancels tweens. Freeze navigation during actual tool drag, not merely while a tool is armed.

Picking separates acquisition radius from visibility. Use a matched ray through each candidate's projected geometric anchor, correct perspective interpolation, explicit numerical tolerances, deterministic overlap ordering, and exact publication proof. Preserve explicit pick-through, but never use a pointer-radius-sized depth allowance for ordinary selection.

Worker curves use exact positive rational Bézier spans with homogeneous subdivision and conservative finite-chord bounds where the source permits exact conversion. Follow the angular and singular rules in NUM. The unsupported-curve fallback is fixed and labeled estimated. Never call a dense sample a proof or silently pass a recursion cap.

Normals come from the underlying surface with explicit face orientation and transform handling. Preserve real hard creases and topology domains. Handle sphere poles, cone apex, singular derivatives, and missing nondegenerate faces according to NUM. No unconditional +Z fallback.

Unify active/static/draft/fill curve quality. Track it per entity, not a session-wide maximum. Separate geometry and style dirtiness, keep stable sketch resources, update batched ranges, and reuse transient buffers. Analytic snapping/constraints must not change when rendered detail changes.

Introduce the specified negotiated version-2 local-origin mesh representation, preserving version-1 byte meaning. Carry quality, error evidence, edge classes, solid membership, and freshness identity explicitly. Wire changes require canonical documentation and C++/Rust/TypeScript fixtures before enablement. Do not modify user BRep/history to migrate display caches.

Resolve an effective displayed set before sections and exact replacement previews. Cap verified closed solids per solid; open/incomplete geometry stays uncapped with status. Cap pixels are not invented editable topology. Keep render/depth policy explicit under both ordinary and supported reversed depth.

Use local, topology-preserving acceleration and bounded preparation. No unchecked triangle reorder, global raycast monkeypatch, per-frame native remeshing, or ordinary-hover synchronous GPU readback. Every CPU/GPU/cache/job allocation must have a named owner and limit.

## Delegation and concurrency

Use at most three concurrent streams. Prefer two disjoint implementers and one reviewer. Every task brief names exact owned files, current checkpoint, relevant contracts, required tests, and prohibited changes. The main orchestrator owns integration and reviews every diff.

`ViewportEngine`, `ViewportRoot`, `meshSync`, `SketchObject`, settings migration, Rust DTOs, worker dispatch, and canonical protocol files are single-owner integration files. Do not permit two agents to modify them concurrently. Subagents must not spawn uncontrolled additional agents or invoke outside models.

Serialize CMake, Cargo, full unit suites, browser suites, and native suites. One explicit heavy-lane owner records the command, checkpoint, and evidence path. Do not kill unrelated builds or start competing browser servers on the same port.

Use the optional local subagent templates only when they match the task. Merge them into existing project configuration without overwriting the user's agent files. Keep normal permission safeguards; do not enable blanket permission bypass.

## Patch discipline

For each package: read actual source, identify the current defect or already-correct contract, implement a red test, preserve its output, make the smallest complete patch, run focused checks, review the diff independently, run integration checks appropriate to the layer, and update the ledger. Source implementation and native acceptance are separate states.

Do not blindly patch every baseline finding. When newer source already fixes one, demonstrate it with the required regression. When current APIs differ, adapt the implementation, not the mathematical or ownership contract.

Use Bun and `bun.lock`. Never rewrite stale `package-lock.json`. Follow current repository worker staging before Cargo app builds. `ONECAD_REQUIRE_WORKER=1` is required for a worker-backed Rust gate; a skipped worker test is not a pass. Do not edit dependency caches/registries in place or loosen compiler checks to conceal a defect.

No commits, pushes, resets, rebases, branch changes, destructive cleanup, production deployment, external paid model invocation, or edits to user documents without explicit authority. QA fixtures use isolated paths. Keep OCW1 stdout clean; logs go to stderr.

## Review questions before accepting any patch

Does this change preserve topology ordinal meaning, semantic IDs, document/session/generation fences, and global coordinate interpretation? Are all CSS/device/world/local units explicit? Who owns every resource, and can every failure path release it? Does this still work before the first frame and after context restoration? Can a stale async result install? Does selection agree with visible geometry? Can a camera/quality change invalidate a previously cached result? Does the test execute the relevant native/GPU path, or only inspect a convenient flag?

For mathematical work, state the assumptions and verify the counterexamples in NUM. Check rational weights, near-plane W, derivative singularities, reflected transforms, finite segment distance, and budget exhaustion. A confident comment is not evidence.

## Contradictions and blockers

If a design assumption is disproven, record the exact decision, minimal executable counterexample or installed source, affected requirements, and the narrow correction needed. Do not implement known-wrong behavior merely to follow prose. Do not silently relax precision, quality, identity, or test gates. Continue independent useful work while the affected gate remains explicit.

If native tools or physical devices are unavailable, implement and run supported lanes, then mark native/device acceptance blocked. Never substitute browser mock results for real OCCT/Tauri/physical evidence. Do not state that code is professional-grade or fully accepted until the defined evidence exists.

## Required progress and final reports

During work, report observable behavior completed, tests actually run, failures found, current package and next dependency. Keep updates short and useful.

At every session boundary, update the execution ledger with HEAD and dirty-diff hash, modified files, ownership, active processes, red/green logs, unresolved blockers, and the exact next action. Do not rely on conversational memory alone.

The final report must list completed requirements/findings, focused and integrated gate results, native/physical evidence, measured budgets, unresolved limits, and exact artifacts. Include known unrelated application acceptance gaps separately. No “100% complete” or broad percentage can replace the requirement/test matrix.

Start by inspecting the checkout and executing WP00. Do not stop after writing another plan.
