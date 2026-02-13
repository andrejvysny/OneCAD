# OneCAD Architecture Optimization & Implementation Plan (V1/V2 Spec)

> Goal: ship a **professional-feeling hybrid parametric + direct modeling CAD** with **Shapr3D-like flow**, while maintaining **deterministic regeneration**, **stable references**, and a **repair-first** UX when certainty is low.

This document is a **high-detail plan + specification** based on your locked decisions. It is designed to be executed iteratively.

---

## 0) Vocabulary & invariants

### 0.1 Key invariants (non-negotiable)

1. **History-first truth**: committed user actions become **timeline nodes** (`OperationRecord`).
2. **Stable IDs across modes**: ElementIds must not depend on preview vs determinism settings.
3. **No silent wrongness**: uncertain binding → **Needs Repair**, not guessing.
4. **Async everywhere helpful**: UI stays responsive; heavy work is off UI thread.
5. **Deterministic regen**: same inputs → same outputs + stable mapping.

### 0.2 Mode definitions

- **Determinism mode**: parallel off; recorded OCCT options applied; produces authoritative regen.
- **Fast mode**: faster preview/tessellation; **must not** change ID assignment/mapping rules.

### 0.3 Timeline model (strict linear)

- Timeline is linear, but supports:
  - **Rollback point** `k`
  - **Insertion at rollback**: new node inserted at `k+1`
  - Steps after `k` are **dirty/pending** until regenerated.

---

## 1) Architecture changes (high-level)

### 1.1 Introduce a single-writer Kernel Thread (mandatory)

**Why**: simplifies OCCT thread-safety and determinism.

**Rule**:
- All OCCT modeling operations happen on **Kernel Thread** only.
- UI thread does not mutate kernel objects.
- Rendering consumes **immutable snapshots**.

**Interfaces**:
- `KernelScheduler::Submit(RegenRequest)`
- `KernelScheduler::Submit(PreviewRequest)`
- `KernelScheduler::Cancel(JobId)`

### 1.2 Separate three pipelines

1. **Preview pipeline** (interactive): fast, transient.
2. **Regen pipeline** (authoritative): deterministic, updates Document truth.
3. **Render pipeline**: tessellation + GPU upload from snapshots.

### 1.3 Introduce explicit “Model Snapshot” objects

A snapshot is what rendering and picking operate on.

`ModelSnapshot` contains:
- `StepIndex` (which step is represented)
- `Bodies[]` (kernel BReps or serialized shapes)
- `BodyId` per body
- `ElementMap` partitions per body (referenced-only)
- `TopologySignature` per step
- `Diagnostics` (warnings, needs repair sets)

Snapshots are immutable once published.

---

## 2) Data Model Spec (Document, History, Bodies)

### 2.1 Document root

`Document`:
- `GlobalSchemaVersion`
- `OCCTFingerprint` (version + build hash)
- `ModelingTolerancePolicy` (doc-wide)
- `History: OperationHistory`
- `Milestones[]`
- `Checkpoints: CheckpointStore`
- `GlobalRegistry`:
  - `DocumentId`
  - `BodyRegistry` (BodyId lifecycle)
  - `NamedSelections`
  - `RepairState`

### 2.2 Body identity rules (locked)

- Each body has stable **BodyId**.
- Split: one child keeps original BodyId; others get new BodyIds.
- Merge: deterministic winner keeps BodyId; others retired/aliased.
- Boolean: preserve **target** body’s BodyId.

**Spec**: Each feature outputs an explicit body list.

`BodyLifecycleEvent`:
- `Created(BodyId)`
- `Modified(BodyId)`
- `Split(parentBodyId -> childBodyIds[])`
- `Merged(inputBodyIds[] -> winnerBodyId)`
- `Deleted(BodyId)`

### 2.3 OperationRecord spec

Each `OperationRecord` must be self-contained and replayable:

Common fields:
- `RecordId` (UUID)
- `StepIndex`
- `RecordSchemaVersion`
- `OpType` (enum)
- `Inputs` (refs + parameters)
- `Outputs` (explicit body list)
- `DeterminismSettings`:
  - `Parallel=false` (enforced in determinism)
  - OCCT algorithm options recorded (Glue, Fuzzy, UseOBB, ...)
  - `TolerancePolicyVersionHash`
- `SelectionRefs[]`:
  - `Primary`: `(BodyId, ElementId)`
  - `IntentQuery` (fallback)
  - `AnchorIntent` (clicked 3D point / UV)
- `SolverPolicyHash` (for sketches)
- `DiagnosticsPolicyVersion`

### 2.4 V1 operation set (locked)

- Sketch (planar)
- Push/Pull (face offset/local extrusion behavior)
- Move (transform)
- Rotate (transform)
- Fillet/Chamfer
- Revolve
- Mirror
- Split Body
- Booleans (in determinism scope)

Each must have its own `OpType` schema.

---

## 3) ElementId & ElementMap system (robust naming)

### 3.1 ID-on-demand policy (locked)

Store ElementIds only for:
- feature inputs
- constraints/dims
- named selections
- selection sets stored in history

### 3.2 Partitioned ElementMap per BodyId (locked)

`ElementMap[BodyId]`:
- `ElementId -> TopoKey` mapping
- `TopoKey -> ElementId` reverse mapping

Where `TopoKey` is a stable “address” for the current snapshot (not persistent across regen by itself).

### 3.3 The resolution ladder (locked)

When binding references during regen:

1. **OCCT history mapping**
   - If `oldTopo` modified/generated → map to newTopo
2. **IntentQuery fallback**
3. **Needs Repair** if ambiguous

### 3.4 Anchor-point intent (locked enhancement)

On user selection store:
- `AnchorPointWorld` (3D point)
- If available: `SurfaceUV` at click time
- `LocalFrame` (normal + tangent or axis)
- Optional adjacency hint (nearest edge id, boundary loop index)

Used to narrow candidates in descriptor matching.

### 3.5 Descriptor set (MVP, locked)

Face descriptors:
- surface type
- quantized centroid
- quantized area
- quantized normal OR axis+radius
- quantized bbox
- adjacency signature

Edge descriptors (if needed):
- curve type
- quantized length
- midpoint/endpoints quantized
- adjacent face context

### 3.6 Confidence policy (locked)

Auto-bind only if:
- `score1 ≥ 0.85` AND
- `(score1 - score2) ≥ 0.10`

Else:
- apply anchor narrowing
- else Needs Repair

### 3.7 Needs Repair storage

`RepairState` stores, per step:
- unresolved refs
- candidate lists + scores
- which ladder level failed
- UI-friendly labels

---

## 4) Deterministic Regen Pipeline

### 4.1 Regen entrypoints

- `RegenToEnd(fromStep=k)`
- `RegenToStep(k)` (fast preview for rollback edit)

### 4.2 Checkpoint system (locked)

**Purpose**: accelerate regen by starting from nearest checkpoint ≤ target.

Checkpoint placement:
- after expensive ops (booleans, fillets, split)
- after milestones
- auto heuristic based on elapsed regen cost

`Checkpoint` contains:
- `StepIndex`
- `Bodies[]` serialized (e.g., BRep)
- `BodyId list`
- `ElementMap partitions` for referenced elements
- `TopologySignature`

### 4.3 Regen algorithm (authoritative)

1. Choose start step `s`:
   - nearest checkpoint ≤ requested start
2. Load checkpoint snapshot into kernel context
3. For each step `i = s+1 .. target`:
   - Build **Input Binding Table**:
     - resolve stored refs using ladder
   - Execute op deterministically using recorded OCCT settings
   - Compute OCCT history maps (always for booleans)
   - Update BodyRegistry lifecycle events
   - Update ElementMap partitions:
     - for referenced elements, map old→new via OCCT history
     - for newly referenced elements, allocate new ElementIds
   - Compute per-step TopologySignature
   - Accumulate diagnostics:
     - warnings
     - NeedsRepair items
4. Publish `ModelSnapshot(step=target)`

### 4.4 Regen failure policy (locked)

If failure at step `m`:
- publish snapshot for `m-1` as **last valid body**
- mark step `m` and downstream as invalid/dirty

Optional overlay of cached final body is not required.

---

## 5) Rollback Editing UX Spec (locked behaviors)

### 5.1 Timeline UI states

- **Valid**
- **Dirty/Pending** (steps after rollback insertion)
- **Error** (failed step)
- **Needs Repair** (ambiguous refs)

### 5.2 Edit loop (debounced, locked)

- While editing parameters at step `k`:
  - regen only to `k` (fast)
- On debounce/commit:
  - regen to end

### 5.3 Insert-at-rollback semantics

If rolled back to `k`:
- new operation inserted at `k+1`
- steps `k+2..end` become Dirty
- editing later steps disabled until valid

### 5.4 Instant scrubbing (locked)

- Scrub uses cached checkpoints/snapshots for instant display
- Once scrubbing stops, system schedules refinement regen

---

## 6) Direct Modeling Features as History Nodes (V1)

For each direct modeling op, define:
- selection semantics
- parameter schema
- deterministic kernel algorithm
- preview algorithm
- reference binding behavior

### 6.1 Push/Pull (face offset / local extrusion)

**Inputs**:
- target face selection set
- distance
- direction mode:
  - face normal
  - explicit vector
  - gizmo axis
- propagation:
  - none
  - tangent chain (optional V1?)

**Kernel strategy** (choose one, implementable with OCCT):
- If planar face: use prism/extrude of a face patch and boolean fuse/cut depending on direction
- If general face: use `BRepOffsetAPI_MakeOffsetShape` or surface offset approach (careful: robustness)

**Spec requirement**:
- Store exact OCCT approach used and options (for determinism + migration).

### 6.2 Move / Rotate (transform)

**Inputs**:
- target body only (locked)
- selection set (faces/edges optional)
- transform matrix / axis-angle
- reference frame (gizmo frame)

**Kernel**:
- apply `BRepBuilderAPI_Transform` to body or subshape set

**Note**:
- if subshape transform would create invalid BRep, either:
  - restrict V1 to body-level transforms, or
  - implement robust “detach and heal” later (not V1)

### 6.3 Fillet / Chamfer

**Inputs**:
- edge selection set
- radius/width
- corner handling mode
- fillet type (constant radius)

**Kernel**:
- `BRepFilletAPI_MakeFillet`
- `BRepFilletAPI_MakeChamfer`

**Determinism**:
- record all OCCT builder flags that affect result

### 6.4 Revolve

**Inputs**:
- profile region reference (sketch region)
- axis definition
- angle
- operation kind (new body, add, cut)

**Kernel**:
- `BRepPrimAPI_MakeRevol`
- boolean if add/cut

### 6.5 Mirror

**Inputs**:
- mirror plane
- bodies list (selected body only in V1)
- keep original flag

**Kernel**:
- transform + optionally boolean merge

### 6.6 Split Body

**Inputs**:
- target body
- tool: plane / face / surface
- keep both sides

**Kernel**:
- boolean split / section + classification

**BodyId rule**:
- one output keeps original BodyId
- others new BodyIds

---

## 7) Per-step topology signature (drift detection)

### 7.1 Signature components (locked suggestion)

- counts: faces/edges/verts
- quantized bounding box
- adjacency hash
- optional histograms (area/length)

### 7.2 Storage

Store in:
- snapshot metadata
- headless regen report

### 7.3 Drift policy

- If drift detected compared to stored signature:
  - mark step as “non-deterministic drift”
  - trigger deterministic rebuild (if opening file)
  - include in headless report

---

## 8) OCCT fingerprint mismatch policy (locked)

On open:
1. detect mismatch
2. warn user
3. auto-run deterministic rebuild
4. if rebuild succeeds: store new fingerprint
5. if rebuild fails: open read-only, keep last valid snapshot

---

## 9) Migration & versioning system

### 9.1 Versions (locked)

- `GlobalSchemaVersion`
- `RecordSchemaVersion` per op

### 9.2 Lossless migration rules (locked)

- Prefer semantic mapping
- Avoid frozen nodes except last resort

### 9.3 Uncertain migration policy (locked)

- Open **read-only** if migration confidence < threshold
- Provide a guided migration report

### 9.4 Migration artifacts

- `MigrationPlan` object:
  - steps
  - confidence per record
  - required user decisions (rare)

---

## 10) Headless regeneration tool (locked)

### 10.1 CLI behavior

`onecad regen --input file.onecad --mode deterministic --report out.json`

Report includes:
- per-step signature
- warnings
- needs repair items
- mapping stats
- fingerprint info

### 10.2 CI usage

- Fail CI if:
  - drift detected
  - needs repair count > 0 (optional policy)

---

## 11) Rendering & picking architecture

### 11.1 Tessellation LOD (locked targets)

- LOD tiers: coarse/medium/fine
- While manipulating: coarse
- On idle: refine

### 11.2 Async tessellation

- background mesh generation
- double-buffer mesh swap
- picking uses stable ElementId tags

### 11.3 Picking mapping

- each rendered primitive maps to `(BodyId, ElementId)`
- selection returns:
  - primary ref
  - anchor intent
  - intent query seed

---

## 12) Performance targets (locked)

- 300–800 features
- 50k–200k faces
- 1–5M visible triangles

**Engineering knobs**:
- checkpoints
- incremental regen (feature-level)
- caching by `(historyHash, meshSettings)`

---

## 13) Implementation roadmap (iterative)

> Each phase ends with measurable acceptance criteria.

### Phase 0 — Instrumentation & scaffolding (1–2 weeks)

Deliverables:
- Kernel Thread + scheduler skeleton
- Snapshot publishing + render consumption
- Minimal deterministic regen loop over stub operations

Acceptance:
- UI never blocks on regen
- snapshot swap is stable

### Phase 1 — History + rollback insertion + dirty timeline (2–4 weeks)

Deliverables:
- OperationHistory linear timeline
- Insert-at-rollback
- Dirty step state propagation
- Regen-to-step k and regen-to-end

Acceptance:
- rollback insertion works
- later steps marked dirty and disabled

### Phase 2 — Checkpoints + instant scrubbing (2–4 weeks)

Deliverables:
- Checkpoint store
- heuristics for checkpoint placement
- scrub uses snapshots/checkpoints
- idle refinement regen

Acceptance:
- scrub is instant for medium models

### Phase 3 — ElementMap v1 + history mapping (4–8 weeks)

Deliverables:
- referenced-only ElementIds
- OCCT history mapping integration
- descriptor matching MVP
- confidence policy
- Needs Repair state + report

Acceptance:
- classic naming-break edits produce Needs Repair rather than wrong attachments

### Phase 4 — Direct modeling ops as history nodes (8–12 weeks)

Deliverables:
- V1 operations implemented with preview+commit:
  - push/pull
  - move/rotate
  - fillet/chamfer
  - revolve
  - mirror
  - split body
- deterministic option recording per op

Acceptance:
- each op replayable and editable

### Phase 5 — Determinism drift detection + headless tool (2–4 weeks)

Deliverables:
- per-step topology signature
- fingerprint mismatch rebuild
- CLI regen tool + report

Acceptance:
- CI run can validate deterministic regen

### Phase 6 — Migration framework (2–4 weeks)

Deliverables:
- global+per-record versioning
- migration planner
- read-only open on uncertainty

Acceptance:
- round-trip open old files with migration

---

## 14) Detailed task backlog by subsystem

### 14.1 KernelScheduler
- job queue
- cancellation
- priority: preview > regen? (preview throttled)
- snapshot publish semantics

### 14.2 History & Records
- schema definitions
- JSON/CBOR serialization
- migration hooks
- unit tests per op schema

### 14.3 ElementMap
- per-body partitions
- descriptor computation library
- scoring function (deterministic)
- needs-repair database

### 14.4 Checkpoints
- placement heuristic
- storage format
- memory budget policy

### 14.5 Diagnostics
- warnings/errors taxonomy
- UI binding
- headless report mapping

---

## 15) Spec appendices

### A) Deterministic scoring function requirements
- stable ordering
- quantization rules fixed and versioned
- tie-breakers deterministic

### B) Quantization policy
- define grid step sizes relative to tolerance
- store quantization version

### C) Body merge winner rule
- deterministic: prefer target body; else lowest creation index; else lowest BodyId

---

## 16) Next iteration notes

This spec is intentionally V1-focused. V2 placeholders (delete/heal, shell/draft, richer topology naming, plugin system) remain out of scope.

