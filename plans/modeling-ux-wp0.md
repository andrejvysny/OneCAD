# OneCAD Modeling UX Implementation & Hardening — WP0 Plan

## Baseline

HEAD = `b022edf7` (pinned). No drift.

## Confirmed Defects (19 total)

| # | Defect | Location | WP |
|---|--------|----------|-----|
| D1 | Two confirmation vocabularies: ✓/✕ vs Apply-only (no cancel) | `ModelToolChips.tsx:133-143,652-736` | WP2 |
| D2 | Click-away commit for Extrude/Revolve/OffsetFace | `ModelToolController.ts:6182-6248` | WP2 |
| D3 | No `onPreview` on DimensionInput; Pattern spacing/angle commit on blur only | `DimensionInput.tsx`, `ModelToolChips.tsx:662` | WP3 |
| D4 | No type-to-enter for model tools | Missing | WP3 |
| D5 | TransformGizmo: 3 full torus rings R=82px cross/occlude | `TransformGizmo.ts:60,183-192` | WP4 |
| D6 | ToolChipState = monolithic nullable bag (909 lines) | `toolChipStore.ts:284-565` | WP2 |
| D7 | CircularPattern re-edit hardcodes `axisOrigin: [0,0,0]` | `ModelToolController.ts:4750` | WP6 |
| D8 | Mirror re-edit hardcodes `planePoint: [0,0,0]`, `fuseWithOriginal: false` | `ModelToolController.ts:4812-4814` | WP6 |
| D9 | `applyResult` ignores `errorMessage` — resolved failure reads as success | `ModelToolController.ts:7732-7760` | WP1 |
| D10 | `commitPattern` always selects source body, not children (non-fused) | `ModelToolController.ts:4830` | WP5 |
| D11 | `repairStore.applyEvent` doesn't compare `(revision, snapshotId)` lexicographically | `repairStore.ts:73` | WP6 |
| D12 | `rebindCandidate` ignores candidate's authoritative bodyId | `historyActions.ts:180-225` | WP6 |
| D13 | Shell re-edit auto-commits on blur | `ModelToolController.ts:3370-3373` | WP2 |
| D14 | `Under-constrained · DOF 0` possible when solver says "under" with DOF 0 | `constraintStatus.ts:19` | WP0 |
| D15 | No modeling interaction contract test | Missing | WP0 |
| D16 | Pattern count stepper 2-12 vs worker max 128 — three silent ranges | `modelToolMachine.ts:1182`, `patternPreview.ts:241` | WP5 |
| D17 | Boolean chip has no visible cancel | `ModelToolChips.tsx:716-736` | WP2 |
| D18 | No result summary for body-lifecycle operations | Missing | WP5 |
| D19 | `commitTransform` success selects sources only (copy:true should select copies) | `ModelToolController.ts:5567` | WP5 |

## User Decisions

- **Click-away**: Remove for ALL tools (spec choice)
- **Pattern count**: Dual range (stepper 2-12, direct entry up to 128)
- **Start**: WP0 — red evidence first

---

## WP0 Deliverables

### WP0.1 — Modeling Interaction Contract

**File**: `src/test/contracts/modelingInteractionContract.ts`

Frozen TARGET interaction matrix per tool. Fields: tool, primaryParameter, previewFidelity, enterSupport, visibleCancel, clickAwayPolicy, requiredSelections, successSelection, reeditCapabilities.

This is the TARGET state (not current). Probe tests compare production against it.

### WP0.2 — Red Tests

Each test proves a defect exists by asserting the TARGET behavior and failing.

#### WP0.2.1 — DOF 0 contradiction
**File**: `src/features/sketch/constraintStatus.test.ts`
**Test**: `sketchStatusText("under", 0)` should NOT return "Under-constrained"
**Current**: Returns `"Under-constrained · DOF 0"` (line 19 default branch)

#### WP0.2.2 — errorMessage false success
**File**: `src/tools/modelTools/ModelToolController.errorMessage.test.ts`
**Test**: When `applyOperation` returns `{errorMessage: "fail", features: [...], changedBodies: [...]}`, the controller must NOT show a success hint and must re-arm the tool
**Current**: `applyResult()` at line 7732 ignores `errorMessage` entirely; `commitPattern` at line 4829 calls `applyResult(res)` then shows success hint

#### WP0.2.3 — CircularPattern re-edit drops axisOrigin
**File**: `src/tools/modelTools/ModelToolController.patternReedit.test.ts` (extend)
**Test**: Re-editing a CircularPattern with stored `axisOrigin: [10, 20, 30]` must commit with `axisOrigin: [10, 20, 30]`, not `[0, 0, 0]`
**Current**: `commitCircular()` at line 4750 hardcodes `axisOrigin: [0, 0, 0]`

#### WP0.2.4 — Mirror re-edit drops planePoint/fuseWithOriginal
**File**: `src/tools/modelTools/ModelToolController.patternReedit.test.ts` (extend)
**Test**: Re-editing a Mirror with stored `planePoint: [5, 10, 0]` and `fuseWithOriginal: true` must commit with those values preserved
**Current**: `commitMirror()` at lines 4812-4814 hardcodes `planePoint: [0, 0, 0]` and `fuseWithOriginal: false`

#### WP0.2.5 — repairStore snapshot ordering
**File**: `src/stores/repairStore.test.ts`
**Test**: Given revision=5/snapshotId=10 already applied, an event with revision=5/snapshotId=8 must NOT replace it
**Current**: `repairStore.ts:73` only checks `event.revision < s.revision`, ignoring snapshotId

#### WP0.2.6 — rebindCandidate ignores candidate bodyId
**File**: `src/features/inspector/historyActions.test.ts`
**Test**: When `ResolveRefResult` echoes `bodyId: "bodyA"` for a candidate, `rebindCandidate` must promote against `bodyA`, not the operated body
**Current**: `historyActions.ts:190` uses `deriveOperatedBody(item)`, ignoring the candidate's authoritative body

#### WP0.2.7 — Boolean/Pattern/Mirror have no visible cancel
**File**: `src/features/toolbar/ModelToolChips.test.tsx` (extend)
**Test**: When `kind === "booleanOp" | "linearPattern" | "circularPattern" | "mirror"`, the rendered chip must contain a cancel button (✕ or Cancel)
**Current**: These chips render `ApplyButton` only — no cancel control

#### WP0.2.8 — Click-away commit exists
**File**: `src/tools/modelTools/ModelToolController.commit.test.ts` (extend)
**Test**: Assert that the controller has NO click-away commit path for Extrude/Revolve/OffsetFace (the `isArmedForClickAway()` method should not exist or always return false)
**Current**: `isArmedForClickAway()` at line 6216 returns true for armed extrude/revolve/offsetFace

#### WP0.2.9 — Shell re-edit blur-commit
**File**: `src/tools/modelTools/ModelToolController.edgeShellPreview.test.ts` (extend)
**Test**: Shell re-edit chip must NOT auto-commit on blur; value changes must go to edit state, Enter/confirm commits
**Current**: Line 3372: `void this.commitShell()` fires on chip value commit (Enter/blur)

#### WP0.2.10 — Pattern post-commit selection
**File**: `src/tools/modelTools/ModelToolController.patternReedit.test.ts` (extend)
**Test**: Non-fused LinearPattern commit must select generated children, not source body
**Current**: Line 4830: `selectionStore.getState().set([{ kind: "body", id: bodyId }])` always selects source

#### WP0.2.11 — Transform copy selection
**File**: `src/tools/modelTools/ModelToolController.commit.test.ts` (extend)
**Test**: Transform with `copy: true` must select created copies, not sources
**Current**: Line 5567: `selectionStore.getState().set(params.targets.map(...))` always selects sources

### WP0.3 — Interaction Contract Probe

**File**: `src/tools/modelTools/modelingInteraction.golden.test.ts`

Golden probe that checks production against the contract. Initially RED for:
- visibleCancel (Boolean/Pattern/Mirror fail)
- clickAwayPolicy (Extrude/Revolve/OffsetFace fail)
- successSelection (Pattern/Mirror fail)

---

## Files to Create/Modify (WP0 only — tests/contracts, no production)

### New files:
1. `src/test/contracts/modelingInteractionContract.ts` — frozen TARGET contract
2. `src/features/sketch/constraintStatus.test.ts` — DOF 0 red test
3. `src/tools/modelTools/ModelToolController.errorMessage.test.ts` — errorMessage red test
4. `src/stores/repairStore.test.ts` — snapshot ordering red test
5. `src/features/inspector/historyActions.test.ts` — candidate body red test
6. `src/tools/modelTools/modelingInteraction.golden.test.ts` — contract probe

### Extended files (add red tests):
7. `src/tools/modelTools/ModelToolController.patternReedit.test.ts` — D7, D8, D10
8. `src/tools/modelTools/ModelToolController.commit.test.ts` — D8, D11, D19
9. `src/features/toolbar/ModelToolChips.test.tsx` — D17
10. `src/tools/modelTools/ModelToolController.edgeShellPreview.test.ts` — D13

---

## Gate Commands

```bash
bun run test                                          # all new tests RED, existing green
bunx tsc --noEmit                                     # clean
bun run build                                         # clean
```

## Expected Outcome

- 11+ new red tests, each failing for the intended reason
- All existing tests remain green
- Contract file documents the TARGET state
- No production code changes
