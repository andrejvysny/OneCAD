---
name: repo-ipad-model-lane
description: iPad shared modeling ops lane (feat/ipad-model, 2026-10-22) — binder + staging, anchors on staged refs, a11y ids incl. compact catalog, Metal tap-by-id tests, fmt-hook + TransformBody edit-gate + zsh traps, ENOSPC
metadata:
  type: project
---

Facts from wiring Fillet/Chamfer/OffsetFace creation (ONECAD-316/319) for desktop + iPad.

**Why:** each one cost time to discover; none is in CLAUDE.md.

**How to apply:**

- Creation binder lives in `onecad-interaction::operand_binding` (pure). It takes `api::StagedEdgeOp` /
  `api::StagedOffsetFace` and returns an `OperationRecord`; `begin_bound` builds the `BeginOperation`.
  The UI starts them in `onecad-ui/src/operation_start.rs` (reply polled in `App::poll`, i.e. LOGIC only).
- Staged refs MUST carry anchors: every commit replays from an empty partition, so a staged id rebinds by
  descriptor + anchor (SCHEMA §10). Without one, a corner edge next to an earlier fillet's boundary edge
  (rival differs only by `outward`, ~0.06 margin) commits NeedsRepair "ambiguous". `stage_edge_op` /
  `stage_offset_face` pass the PrepareEdgeOp/PrepareOffsetFace evidence anchor via `stage_anchored`.
  Plain `stage_selection` / `promote_selection` still pass `None`.
- The reducer stamps the field text as the scalar expr: a committed record = bound record with
  `Scalar::with_expr(v, "v")`, so compare after patching that, not raw equality.
- `EditCommand` and `CommitStatus` are not PartialEq: pattern-match / `matches!`.
- Real-worker plate tests: the XY sketch frame maps (u,v) → world (−v, u); use
  `WorldPlane::XY.plane()` axes to find a corner edge (plate lands at x∈[−40,0], y∈[0,60]).
  `MassPropertiesDto` has volume + exact centroid, no bbox; bounds come from the f32 mesh.
- AccessKit author ids: `crate::automation::name(ctx, response.id, ..)`. GPU-free check:
  `automation::testing::author_ids(|ui| ..)`. Full-App ids (viewport, history, tabs, planes) are tested by
  ignored Metal tests in `model_operation_delivery_tests.rs` that `tap(host, id)` by author id.
- Host test harness: `Host::ui` runs only `eframe::App::ui`, never `logic`, so a staging reply is not
  consumed until `pump_until` — use that to test races deterministically. Sketch Finish is disabled until
  `SketchPhase::Active`; pump to Active before tapping it.
- Lane worktrees can get a SECOND writer (twin agent): check `git status` + mtimes before editing; use
  exact-match edits so a concurrent change fails loudly.
- The PostToolUse format hook runs rustfmt in a NON-project style (`a.1 .1`, reordered imports) and also
  rewrites `#[path]` child test modules you never touched; always `cargo fmt --all` before diff/commit.
- SCHEMA §7.3 edit-safety gate: editing a TransformBody seeds NeedsRepair on EVERY downstream ref, a later
  TransformBody included (repro'd on a bare plate). A real-worker transform edit must target the last one.
- Bash tool shell is zsh: `$FILES` does not word-split in `for`/args and `PIPESTATUS` is empty; do
  multi-file backup/restore probes in a Python script.
- Pill compact mode (`pill/layout.rs compact_tools`) draws only [preferred, More]; other tools live as
  More-catalog rows (`pill/catalog.rs row`), which carry the same `onecad.tool.<id>`.
- Disk can fill (ENOSPC) from many worktree `target/` dirs; check `df -h /System/Volumes/Data` first and
  export `CARGO_INCREMENTAL=0 CARGO_PROFILE_{DEV,TEST}_DEBUG=line-tables-only` (lane target ≈1–2 GB).
