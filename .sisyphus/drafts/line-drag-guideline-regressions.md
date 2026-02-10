# Draft: Line Drag + Guideline Click Regressions

## Requirements (confirmed)
- [user report]: Latest changes broke drag/move functionality for lines.
- [user report]: When moving any point, other points/entities do not stay in place as expected.
- [user report]: Visual guidelines render and cursor snaps, but on horizontal/vertical guides user cannot click to set/create line point.
- [user request]: Proceed iteratively.
- [user request]: Perform thorough analysis of current implementation.
- [user request]: Ask additional questions for more context.
- [user answer]: drag regression context is **no active tool** (selection/point-drag mode).

## Technical Decisions
- [mode]: Refactoring/bug-fix planning mode before implementation.
- [approach]: Run deep regression analysis first (code-path + behavior-level), then produce focused execution plan.

## Research Findings
- [user answer]: drag regression reproduced in **Edit existing sketch** flow (not only active drawing).
- [user answer]: test strategy preference = **TDD**.
- [explore finding]: line click commit path is `Viewport::mousePressEvent` -> `SketchToolManager::handleMousePress` -> `LineTool::onMousePress`.
- [explore finding]: guide-first policy on mouse press can force H/V snap with `pointId` equal to line start point, triggering `startId == endId` guard and preventing line creation.
- [explore finding]: likely safest fix is split policy by phase: keep guide-first for move preview, use canonical/filtered snap for commit press/release.
- [explore finding]: existing infra is prototype-target based (`tests/CMakeLists.txt`) with strong `SnapManager` coverage in `tests/prototypes/proto_sketch_snap.cpp`.
- [explore finding]: gap exists at `SketchToolManager`/`LineTool` integration level — no direct tests for guide-first behavior during press/release commit.
- [explore finding]: drag regressions are currently only indirectly covered (`proto_sketch_solver`, `proto_sketch_fixed_and_move`), not through UI interaction path.
- [explore finding]: **edit-point drag path bypasses SketchToolManager/SnapManager** and goes through `Viewport` drag state machine -> `Sketch::beginPointDrag/solveWithDrag/endPointDrag` -> `ConstraintSolver::solveWithDrag`.
- [explore finding]: guide-first snap changes are still a high-confidence trigger for geometry placement/commit regressions in drawing flow, which can later appear as drag instability.
- [explore finding]: current guide-first policy selects nearest `hasGuide` candidate without respecting geometry snap priority at commit phase.

## Open Questions
- [expected drag behavior]: Clarify exact expected behavior for unconstrained vs constrained geometry during point move.
- [repro details]: Need exact tool/workflow sequence to reproduce each issue reliably.
- [severity scope]: Need whether regressions happen only in Line tool or also Arc/Rectangle/Ellipse flows.

## Interim Hypotheses (ranked)
- [high]: Commit-phase guide-first override creates invalid endpoint identity reuse (`startId == endId`) in Line tool.
- [medium]: Shared guide-first logic now also affects edit-drag path where preserving anchor points needs different snap semantics.
- [medium]: getReferencePoint overrides + expanded H/V candidate set increase probability of selecting same-origin guide point during commit.
- [high]: true point-drag path itself likely remains solver-driven and should keep non-dragged points fixed; observed "others move" may originate from prior mis-committed geometry caused by snap policy during creation.

## Coverage Gaps + Proposed Tests
- [gap]: no regression test simulates `SketchToolManager::handleMousePress/Move/Release` + `LineTool::onMousePress` commit with guide snaps.
- [gap]: no targeted regression for H/V guide commit where endpoint must be created and not collapse to start point ID.
- [proposed]: add tool-manager integration prototype test for click-click line creation under guide-first preview.
- [proposed]: add drag-focused regression test that validates `beginPointDrag` -> `solveWithDrag` -> `endPointDrag` preserves expected geometry anchors.

## Scope Boundaries
- INCLUDE: line drag/move regression root cause, H/V guideline click-commit regression root cause, required tests/QA plan.
- EXCLUDE: unrelated feature additions, non-snap UI redesign, kernel/ElementMap changes.

## Test Strategy Decision
- **Infrastructure exists**: YES (prototype test executables in repo)
- **Automated tests**: YES (**TDD**, user selected)
- **Agent-Executed QA**: YES (mandatory for all tasks)
