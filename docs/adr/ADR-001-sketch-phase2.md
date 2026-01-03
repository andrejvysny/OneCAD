# ADR-001: Sketch Phase 2 Decisions

Status: Proposed
Date: TBD
Owner: TBD
Sign-off required by: TBD (before Phase 2 kickoff)

## Scope
This ADR captures open architectural decisions that block Phase 2 implementation.

## Decisions

### 1) Threading model for solver
- Options:
  - QtConcurrent::run with QFutureWatcher
  - std::async + std::future
  - Custom thread pool (Qt or std)
- Decision: TBD
- Rationale: TBD
- Trade-offs: TBD
- Implementation notes: TBD
- Acceptance criteria: Smooth drag at 30 FPS; no UI freezes on >100 entities.

### 2) Undo/Redo integration
- Options:
  - Integrate with global command stack
  - Sketch-local undo stack merged on exit
  - Hybrid (sketch-local with periodic snapshots)
- Decision: TBD
- Rationale: TBD
- Trade-offs: TBD
- Implementation notes: TBD
- Acceptance criteria: Constraint edits are undoable and deterministic.

### 3) Selection persistence across solve
- Options:
  - Keep selection by entity ID only
  - Re-evaluate selection after solve based on geometry proximity
  - Clear selection on solve
- Decision: TBD
- Rationale: TBD
- Trade-offs: TBD
- Implementation notes: TBD
- Acceptance criteria: Selection remains stable during drag and solve.

### 4) Construction geometry storage
- Options:
  - Boolean flag on SketchEntity (current)
  - Separate container for construction entities
  - Dual-layer sketch graphs
- Decision: TBD
- Rationale: TBD
- Trade-offs: TBD
- Implementation notes: TBD
- Acceptance criteria: Construction entities never form faces; constraints can reference them.

### 5) Sketch origin behavior
- Options:
  - Fixed at plane origin
  - User-placeable origin on plane
  - Face-centered origin for face sketches, fixed for standard planes
- Decision: TBD
- Rationale: TBD
- Trade-offs: TBD
- Implementation notes: TBD
- Acceptance criteria: Origin behavior is predictable for both plane and face sketches.

## Notes
- Update this ADR with final decisions, owners, and dates before Phase 2 begins.
- Link any supporting experiments or spike results here.
