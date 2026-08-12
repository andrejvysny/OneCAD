# Handoff — Component Library (start-screen browser)

Session 11 · 2026-08-12

> Continues Session 10 directly — same worktree, same uncommitted delta.
> User-requested (with a screenshot), not a plan WP: browse the library
> from the START screen, not only inside an opened project.

## Done and why

Full detail in `TODO.md` § COMPONENT-LIBRARY start-screen browser. New
`StartNavKey = "library"` (`StartScreen.tsx`), a fourth `StartSidebar` row,
and two new read-only components (`StartLibraryPanel.tsx`,
`ComponentDetails.tsx`). Zero Rust changes — `list_library_components`/
`reindex_library` never needed an open document, confirmed by reading
`library.rs` first. Deliberately does NOT reuse the editor `LibraryPanel`'s
placement-arm gesture (no `ViewportEngine`/`DocumentRuntime` exists on this
screen) — a selected card's details pane says "Open a project to place this
component" rather than faking a drag with nowhere to land it.

## How to resume

1. Run the `handoff` skill with "resume".
2. This is complete and gated (`TODO.md`'s entry has the numbers). Next
   task is whichever of WP-2.5/WP-2.6 the user picks (see Session 10's own
   entry below), UNLESS the user wants the natural follow-up this session
   flagged: "browse → jump into a new project with the component pre-armed"
   — that needs `newProject()` to resolve before the editor's module
   registration can arm anything, a real sequencing problem not designed
   here.

## Pointers

- Tasks → `TODO.md` § COMPONENT-LIBRARY start-screen browser
- Snapshot → `CURRENT_STATE.md` § COMPONENT LIBRARY — LIVE DELTA (session 11)

---

# Handoff — Component Library (WP-2.4, Configurator UI)

Session 10 · 2026-08-12

> Continues Session 9 directly (below) — same worktree, same uncommitted
> delta, P0/P1/WP-2.1–2.3 unchanged. This entry covers WP-2.4 only.

## Goal

WP-2.4 per Session 9's own "How to resume": the configurator UI, spec §294
— free params as editable controls, live designation string, committed
through WP-2.3's `SetComponentParams` command.

## Done so far (and why)

Full detail in `TODO.md` § COMPONENT-LIBRARY WP-2.4. Worth restating:

- **Deliberately scoped to POST-placement editing only.** Spec §294 also
  describes a pre-placement configurator (opened from the library card,
  before dragging) and a live 3D preview while editing. Neither is built —
  building a real pre-placement flow means touching `LibraryPanel`'s card
  interaction and the ghost-preview lane, and a live edit-time 3D preview
  means wiring a `PreviewOp` session into the inspector section, both
  genuinely separate surfaces. The slice that builds directly on WP-2.3
  (an inspector section for an ALREADY-PLACED instance) is what shipped.
- **Backend DTO had to widen first.** `IndexEntry`/`LibraryComponentDto`
  only carried `parameter_keys` (names) before this session — nothing told
  the frontend which keys are `role: "free"` (editable) vs `"table"`/
  `"computed"` (display-only, backend-derived). Added the full
  `parameters: BTreeMap<String, ParameterSpec>` + `designation:
  Option<String>` to both.
- **Found a real bug reading the file this WP needed anyway**:
  `tauriClient.ts::placeComponent` never forwarded `rotate` to the real
  `place_component` Tauri command. WP-1.5's handoff claimed "commit via
  `CadClient.placeComponent` (now carrying rotate, not just translate)" —
  true only on the mock lane. Fixed; flagged that nothing automated proves
  the real-backend half (no real-worker Playwright lane exists here).
- **mockClient's `setComponentParams` is a REAL implementation**, unlike
  `detachComponent`'s honest "not yet" stub — `commitPlaceComponent` now
  stores `featureParams` (it never did before; `PlaceComponent` bypasses
  `commitOp`'s generic path entirely), so the mock's role=free check and
  merge are the genuine logic, not a fake pass-through. What's NOT
  simulated: the mock's mesh is a fixed demo shape regardless of size, so an
  edit changes the stored value + designation but not the rendered geometry.
- **A real spec-example inconsistency, worked around not copied**: spec
  §2.1's designation example (`"ISO 4762 M{thread}x{length}"`) pairs a
  literal `M` with a thread VALUE that, in this codebase's actual
  convention (WP-2.1/2.2, both keyed `"M6"`-style), already carries the `M`
  — doubling it. Every designation string this session authored drops the
  literal `M`. No real `component.toml` content exists yet to conflict with.

## How to resume

1. Run the `handoff` skill with "resume".
2. No worker rebuild needed — Rust changes are app-crate/library-crate DTO
   widening only, no wire/C++ changes.
3. Next task: **WP-2.5** (three-level thread detail: cosmetic default/
   simplified/modeled) or **WP-2.6** (kernelbench cases from table
   extremes) — independent of each other and of this WP per the plan's
   dependency graph, pick either. If the user wants spec §294 covered in
   full, the pre-placement configurator + live 3D preview this WP cut are
   the other candidates — not yet a numbered WP, would need scoping first.
4. Full P2 WP breakdown is in the plan file referenced below.

## Open questions

Unchanged from Session 9 — Q4 (template content, P3), Q5 (`document`
source nested-replay, P3), the embedded-source reopen-without-library test
gap. New from this session: whether pre-placement configuring + live 3D
preview get their own WP number, or stay folded into a later pass — not
decided, not blocking.

## Pointers

- Tasks → `TODO.md` § COMPONENT-LIBRARY WP-2.4 (gate entry, full detail)
- Snapshot → `CURRENT_STATE.md` § COMPONENT LIBRARY — LIVE DELTA (session 10)
- Plan → `~/.claude/plans/resume-implementation-of-component-twinkling-glade.md`
- Spec → `TheComponentLibrary/onecad-component-library-spec.md`

---

# Handoff — Component Library (WP-2.3, SetComponentParams)

Session 9 · 2026-08-12

> Continues Session 8 directly (below) — same worktree, same uncommitted
> delta, P0/P1/WP-2.1/2.2 unchanged. This entry covers WP-2.3 only.

## Goal

WP-2.3 per Session 8's own "How to resume" and the plan's dependency graph:
the `SetComponentParams` command in `library.rs`, mirroring
`place_component_at`/`detach_component_at`'s `*_at`-split shape, enforcing
every requested param key is `role: free` on the component's actual
signature.

## Done so far (and why)

Full detail in `TODO.md` § COMPONENT-LIBRARY WP-2.3 (gate entry). Worth
restating:

- **The role check needed the FULL `component.toml`, not the index.**
  `IndexEntry` only carries `parameter_keys` (names, from WP-1.4/1.5) — no
  `role`. New `component_package_at` helper walks `IndexEntry.path` back to
  the package directory and re-parses `component.toml` directly (reusing
  `onecad_library::package::parse` — no crate change needed, everything it
  touches was already `pub`).
- **`source.params` deliberately does NOT carry a fully resolved parameter
  set.** The doc comment on `ComponentSourceRef::Generator::params` calls it
  "free values merged with table/computed derivations", which read like it
  should include `role: table` values (e.g. `head_d`) too. Checked against
  what the worker (WP-2.1) actually reads first: `resolve_source_and_publish`
  only looks up `source.params.thread`/`.length` by name, both `role: free`
  on the ISO 4762 package, and derives every table value itself from its own
  copy of the dimension table. Shipping table-derived values into
  `source.params` would be inert data the worker never reads — same class of
  mistake WP-1.5's `mate` field decision flagged. Kept the scope to the
  merged free-param map only; a comment on `set_component_params_at`
  explains why, so a future reader doesn't "fix" it into scope creep.
- **A record that isn't a placed component needs no dedicated check** —
  `serde_json::from_value::<PlaceComponentParams>` on a `DetachComponent`
  record's params (which lacks `componentId` etc.) fails the deserialize
  itself, surfaced as `InvalidCommand`. Simpler than `detach_component_at`'s
  field-by-field `.get(...)` reads, and correct here because this WP needs
  every field to reconstruct the record, not just two.
- **Zero frontend/e2e changes this session** — confirmed via `git status`.
  WP-2.4 is the first WP that gives a caller any way to actually invoke this
  command from the UI.

## How to resume

1. Run the `handoff` skill with "resume".
2. No worker rebuild needed — this WP is Rust app-crate only, no C++/wire
   changes. The sidecar staged for WP-2.1 (20:34) is still current.
3. Next task: **WP-2.4 — configurator UI** (param edit → live designation).
   Needs a `CadClient.setComponentParams` method added to the `CadClient`
   interface (append-only per the frontend seam rule) plus both `mockClient`
   and `tauriClient` implementations, then a UI surface (likely an inspector
   section or a panel on the placed record) that calls it and shows the
   resulting designation string live. `component.toml`'s `metadata.designation`
   field exists for this but nothing currently formats it from resolved
   params — check whether that formatting belongs in this crate or the
   frontend before writing it.
4. Full P2 WP breakdown (2.1 through 2.7) is in the plan file referenced
   below — don't re-derive the dependency graph.

## Open questions

Unchanged from Session 8 — Q4 (template content, P3), Q5 (`document` source
nested-replay, P3), the embedded-source reopen-without-library test gap.
New from this session: none blocking.

## Pointers

- Tasks → `TODO.md` § COMPONENT-LIBRARY WP-2.3 (gate entry, full detail)
- Snapshot → `CURRENT_STATE.md` § COMPONENT LIBRARY — LIVE DELTA (session 9)
- Plan → `~/.claude/plans/resume-implementation-of-component-twinkling-glade.md`
  (P2 WP breakdown)
- Spec → `TheComponentLibrary/onecad-component-library-spec.md`

---

# Handoff — Component Library (P2 kickoff: WP-2.1 + WP-2.2)

Session 8 · 2026-08-12

> Continues Session 7 directly (below) — same worktree, same uncommitted
> delta, P0/P1 unchanged. P1 is closed; this entry starts P2.

## Goal

P2 (parametric fasteners, spec §10) per plan `~/.claude/plans/resume-
implementation-of-component-twinkling-glade.md`: the first two WPs — the
worker-side table-driven ISO 4762 generator and its Rust-side metadata
mirror + data provenance.

## Done so far (and why)

Full detail in `TODO.md` § COMPONENT-LIBRARY WP-2.1/WP-2.2 (gate entries).
Worth restating:

- **BOLTS data sourcing was a real open question (Q3), now resolved for
  the ISO 4762 family**: fetched via `gh api` from
  `github.com/boltsparts/BOLTS_archive` (`data/hex_socket.blt`, class
  `hexsocketheadcap`), not guessed. The repo's GitHub-reported license
  (GPL-3.0) is a whole-repo default that does NOT apply to this data — the
  file's own header carries LGPL 2.1+, matching spec §6.3's claim once
  checked at the file level. `THIRD_PARTY_NOTICES` (new, repo root) records
  this.
- **The two dimension tables (worker C++, `onecad-library` Rust) are
  DELIBERATE duplicates, not a shared source** — the worker owns geometry
  generation (spec §6: generators are built-in/versioned there), the
  library crate owns authoring/metadata (`component.toml`'s `[parameters]
  role="table"` resolution). A cross-pinning test
  (`tables::tests::geometry_fields_agree_with_the_worker_table_across_the_
  seed_range`) catches future drift between the two copies.
- **Real plan-doc deviation, caught before writing code**: the top-level
  P2 sketch assumed `SetComponentParams` needs new C++ dispatch. It does
  not — WP-1.2 already made it an in-place edit of `PlaceComponentParams`
  via the generic `UpdateOperationParams` path, so this WP's table lookup
  is automatically reachable by ANY edited `PlaceComponent` record once a
  caller can construct that edit (WP-2.3, not yet built).
- **Zero frontend/e2e changes this session** — confirmed via `git status`,
  not assumed. Nothing sends `source.params.thread`/`.length` yet, so
  WP-2.1's new table-driven path is currently reachable only by tests that
  construct the params directly. WP-2.3 is what makes it reachable from a
  live gesture.

## How to resume

1. Run the `handoff` skill with "resume".
2. Worker rebuilt this session (`ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1"
   scripts/build-worker.sh Release`) — staged sidecar reflects WP-2.1's
   table-driven generator. Rebuild again if stale.
3. Next task: **WP-2.3 — `SetComponentParams` command** (`src-tauri/src/
   library.rs`, mirroring `place_component_at`/`detach_component_at`'s
   `*_at`-split shape): resolves the target record's component signature,
   enforces every requested key is `role: free` (structural-only check
   already exists in `onecad-core`; this is the app-crate half of the split
   WP-1.2 flagged), merges into a new resolved param set, and constructs
   `EditCommand::UpdateOperationParams`. Then **WP-2.4** (configurator UI —
   the first WP giving a user any way to actually pick a size before/after
   placing).
4. Full P2 WP breakdown (2.1 through 2.7) is in the plan file referenced
   above — dependency graph and per-WP scope already settled, don't
   re-derive it.

## Open questions

Unchanged from Session 7 — Q4 (template content, P3), Q5 (`document`
source nested-replay, P3) still open, plus the embedded-source
reopen-without-library test gap flagged there. New from this session: none
blocking — WP-2.1/2.2 landed exactly as planned.

## Pointers

- Tasks → `TODO.md` § COMPONENT-LIBRARY WP-2.1/WP-2.2 (gate entries, full detail)
- Snapshot → `CURRENT_STATE.md` § COMPONENT LIBRARY — LIVE DELTA (session 8)
- Plan → `~/.claude/plans/resume-implementation-of-component-twinkling-glade.md`
  (P2 WP breakdown) and `~/.claude/plans/do-thorough-analysis-of-abstract-
  thunder.md` (original P0/P1, P2's coarse starting sketch)
- Spec → `TheComponentLibrary/onecad-component-library-spec.md`
- Data provenance → `THIRD_PARTY_NOTICES`

---

# Handoff — Component Library (WP-1.6 + WP-1.7, P1 closed)

Session 7 · 2026-08-12

> Continues Session 6 directly (below) — same worktree, same uncommitted
> delta, P0/P1.1–1.5 unchanged. This entry covers WP-1.6 + WP-1.7, which
> closes P1 to spec §10's gate language.

## Goal

Resume per Session 6's own "How to resume": WP-1.6 (StatusSection + tasks-
chip real producer) then WP-1.7 (e2e for the WP-1.5 flow), the two remaining
P1 work packages per plan `~/.claude/plans/do-thorough-analysis-of-abstract-
thunder.md`.

## Done so far (and why)

Full detail in `TODO.md` § COMPONENT-LIBRARY WP-1.6 and WP-1.7 (gate
entries) — source of truth, not repeated here. Two things worth carrying:

- **WP-1.7's real finding**: the original plan's WP-1.7 wording ("save,
  close, delete library root, reopen, assert body present") is **not
  provable in the Playwright MOCK lane** — `mockClient.newDocument()`/
  `openDocument()` fabricate a fresh document on every call, nothing
  persists for a `page.reload()` to round-trip through. Rather than write a
  test that looks like it proves reopen-without-library but actually proves
  nothing, the e2e spec (`e2e/library-browse-place-snap.spec.ts`) covers
  what the mock lane genuinely can: the frontend gesture chain (browse → arm
  → hover-classify → snap ghost → commit → tree update → Escape-cancel).
  The reopen invariant IS proven, real-worker-side, for the generator-source
  case: `component_ops.rs::place_component_survives_save_and_a_fresh_
  worker_reopen`. **Residual, flagged not fixed**: the EMBEDDED-source
  variant of that same invariant (the actual spec §12 differentiator — a
  cached blob surviving a deleted library folder) has no automated test
  anywhere. WP-1.3 shipped `place_component` generator-source only; the
  embedded-blob authoring-time copy-in the original plan's WP-1.3 section
  described was never built. Whoever picks up P2/P3 should either build it
  or narrow spec §12's claim.
- **WP-1.6's `StatusSection` is the first NESTED slot contribution** in the
  codebase (it lives inside `StatusBar`'s own `<SlotHost/>`, not one of
  `EDITOR_REGIONS`' top-level regions). `editorMountOrder.golden.test.ts`
  needed its "every panel lands somewhere rendered" completeness check
  taught about this — the frozen `EDITOR_MOUNT_ORDER_CONTRACT` array itself
  is untouched, only the probe changed, per the contracts README's own rule.

## How to resume

1. Run the `handoff` skill with "resume".
2. Worker binary unchanged since Session 6 — no rebuild needed for anything
   in this entry (WP-1.6/1.7 are frontend/e2e-only).
3. **P1 is closed.** Next task is **P2 (parametric fasteners)** per the
   plan's coarser P2 section — needs its own follow-up plan, not started
   here. Open question carried forward: BOLTS ingestion tooling ownership
   (one-time offline script vs. a maintained in-repo tool re-run on BOLTS
   updates) — unresolved, blocks nothing in P1, will matter once P2 starts.
4. Full gate, all green this session:
   ```bash
   bunx tsc --noEmit && bun run test        # 243 files / 4142
   bunx playwright test                     # 396 passed / 0 failed
   cd src-tauri && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
   ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 \
     cargo test --workspace --no-fail-fast   # 2 pre-existing failures unchanged, everything else green
   ```

## Open questions

Unchanged from Session 5/6 — Q3 (BOLTS ingestion, now the gating question
for starting P2), Q4 (template content, P3), Q5 (`document` source nested-
replay, P3) still open. New residual from this session: the embedded-source
"reopen without library" test gap (above) — not blocking, not yet assigned.

## Pointers

- Tasks → `TODO.md` § COMPONENT-LIBRARY WP-1.6 / WP-1.7 (gate entries, full detail)
- Snapshot → `CURRENT_STATE.md` § COMPONENT LIBRARY — LIVE DELTA (session 7)
- Plan → `~/.claude/plans/do-thorough-analysis-of-abstract-thunder.md`
- Spec → `TheComponentLibrary/onecad-component-library-spec.md`
- This session's own plan → `~/.claude/plans/resume-implementation-of-component-twinkling-glade.md`

---

# Handoff — Component Library (WP-1.5, the placement gesture)

Session 6 · 2026-08-12

> Continues Session 5 directly (below) — same worktree, same uncommitted
> delta, P0/P1.1–1.4 unchanged. This entry covers WP-1.5 only.

## Goal

WP-1.5 per the plan and Session 5's own "How to resume": the interactive
snap-placement gesture (spec §5.1-§5.4) — classify hovered geometry, match
against the dragged component's attachments, preview a snapped ghost, commit.

## Original plan

Same as Session 5: `~/.claude/plans/do-thorough-analysis-of-abstract-thunder.md`.

## Done so far (and why)

Full detail, including the architecture fork this WP hit and how it was
resolved, is in `TODO.md` § COMPONENT-LIBRARY WP-1.5 (gate entry) — that is
the source of truth, not repeated here. The one thing worth restating because
it will matter to whoever builds P3's persistent mates: **`mate` is
deliberately NOT populated by this WP.** The worker's `ComponentOp.cpp`
resolver reads `placement.{translate,rotate}` only and has no `mate` handling
at all — checked by reading the file, not assumed. Populating a `mate` field
now would be inert data that LOOKS like a real feature; P3's implementer
should treat this WP's `placement`-only output as the honest baseline, not a
regression to fix.

The other load-bearing decision: **library's placement gesture is a
module-level singleton (`placementController.ts`) that reaches
`ViewportEngine` directly via `engineBridge`, the same pattern
`ModelToolController` already uses** — NOT a `ViewportContribution`. That
contract has no raw-pointer-event hook by design (contributions only draw
into their own scene-graph slot), and `ViewportEngine.configurePicking` (the
canvas's actual pick/hover authority) is a single hardwired seat owned by
`ViewportRoot.tsx`. Presented this fork to the user before writing code
(reuse-existing-primitives vs. new engine-level exclusive-gesture capability
vs. drop live-drag entirely); "reuse existing primitives" was chosen.
`placementController.ts`'s own header comment explains the resulting
mechanics (capture-phase window listeners, `setOrbitSuppressed`,
`probePick`). If a FUTURE module needs the same kind of interactive
canvas-owning gesture, this is the precedent to follow — don't re-litigate
the fork, and don't reach for a new platform capability unless this pattern
genuinely doesn't fit.

## Dead ends ruled out

- Don't route the ghost preview's real commit through `endPreview(commit:
  true)`. It would skip `Library::resolve_source`'s revision
  re-verification. Ghost sessions are ALWAYS cancelled; commit is the
  dedicated `CommandApiService.placeComponent` call.
- Don't add a new `ViewportContext`/platform capability for body-picking.
  `engine.probePick`/`setPreviewBody`/`clearPreviewBody` were already
  public and generic; the "first WP touching the viewport" risk the plan
  flagged turned out to be about INPUT ROUTING, not about the platform
  contract being too narrow for the DRAWING half.
- Don't build a second preview mapper for the ghost (spec §5.1 says so
  explicitly, and `ipc/previewOps.ts`'s own header repeats it). The mock
  lane's local-fallback synthesis in `localSolver.ts` is the sanctioned
  per-client geometry-computation seam, not a second mapper.

## How to resume

1. Run the `handoff` skill with "resume".
2. Rebuild the worker if stale (see Session 5's § How to resume below for
   the exact command) — this WP made zero C++ changes, so the existing
   staged sidecar from Session 5 is still valid if untouched.
3. Next task: **WP-1.6** — StatusSection + tasks-chip real producer. The chip
   already has `begin/setProgress/end` with zero producers (Session 4/
   MODULAR-PLATFORM wave); `reindexLibrary` becomes its first. Then
   **WP-1.7**: e2e (browse→place→snap→save→reopen-without-library), which
   will need the `?mocklibrary=1` fixture this WP added (`mockClient.ts`'s
   `MOCK_LIBRARY_FIXTURE`) wired into a real Playwright spec rather than only
   manual verification.

## Open questions

Unchanged from Session 5 — Q3 (BOLTS ingestion), Q4 (template content), Q5
(`document` source nested-replay) still open, none block WP-1.6/1.7.

## Pointers

- Tasks → `TODO.md` § COMPONENT-LIBRARY WP-1.5 (gate entry, full detail)
- Snapshot → `CURRENT_STATE.md` § COMPONENT LIBRARY — LIVE DELTA (session 6)
- Plan → `~/.claude/plans/do-thorough-analysis-of-abstract-thunder.md`
- Spec → `TheComponentLibrary/onecad-component-library-spec.md`

---

# Handoff — Component Library (P0 + P1.1–1.4)

Session 5 · 2026-08-12

> **THREE LIVE THREADS NOW.** This entry (below) is new work, uncommitted on
> branch `OneCAD-Component-Library` (a worktree). Session 4 and Session 3
> (further down) are prior, committed work on the main line, kept for context —
> read them if this session's code touches something they describe (mount-order
> contracts, checkpoints, kernelbench).

## Goal

Implement the Component Library described in
`TheComponentLibrary/onecad-component-library-spec.md`: placeable mechanical
parts (screws, bearings, motors) with mate-like snapping, parametric
generators, user authoring, and project templates — new capability on top of
the Platform refactor (Session 4) and ongoing modeling-correctness work.

## Original plan

`~/.claude/plans/do-thorough-analysis-of-abstract-thunder.md` — full WP
dependency graph and file-level detail for P0/P1, coarser sketches for
P2–P4, six open questions. Followed the spec's own phasing (§10):
P0 spike → P1 static library → P2 parametric → P3 authoring/templates/mates →
P4 registry-later.

## Done so far (and why)

P0 (both WPs) and P1.1–P1.4 landed; full per-WP gate records with exact test
counts and commands are in `TODO.md` (six dated entries, COMPONENT-LIBRARY
WP-0.1 through WP-1.4) — that's the source of truth for gate detail, not
repeated here. What's worth carrying that isn't just "read the diff":

- **WP-0.1's latency spike came back GO** (p95 = 0.16 ms for the new
  `ClassifyElement` read-only kernel verb, gate was < 16 ms) — WP-1.5 can use
  live hover-to-classify for the snap gesture, no click-to-classify fallback
  needed. This was the one real risk item in the whole plan; it's cleared.
- **`onecad-library` crate is deliberately independent of `onecad-core`**
  (plan's open Q1, resolved by doing it): `onecad-core` owns wire/SCHEMA-facing
  types (`ComponentSourceRef` etc. in `record.rs`), `onecad-library` owns
  package-format types (`component.toml`) separately, and `src-tauri/src/library.rs`
  is the app-crate bridge that translates. Don't collapse these — the crate's
  own `Cargo.toml` header comment states the "MUST NOT depend on tauri, MUST
  NOT do network I/O" invariant this boundary protects.
- **Course-correction on the op family (deviation from the plan's WP-1.2 as
  written):** the plan assumed `SetComponentParams`/`ReplaceComponent` would
  need new `KnownOperation` variants like `PlaceComponent`/`DetachComponent`
  did. They don't — both are in-place param edits via `update_operation_params`,
  same pattern Hole's profile-mode edits already use. Only `DetachComponent`
  needed a new variant (a genuine op-TYPE swap, using the existing
  Fillet⇄Chamfer in-place-swap precedent, one-directional this time). Caught
  mid-implementation, recorded in TODO.md as it happened. If P2 revisits this,
  don't re-derive — the params-only path is confirmed correct.
- **The `AppHandle`/mock-runtime split (WP-1.3):** every mutating Tauri
  command in `library.rs` is a thin public wrapper over a private `*_at(...)`
  twin that takes an explicit path/root instead of `AppHandle`, because
  `tauri::test::mock_app`'s `MockRuntime` cannot satisfy a bare `AppHandle`
  (pinned to `Wry`) but CAN satisfy `tauri::State<'r, T>` (confirmed by
  reading tauri's own source, not assumed). This is now the pattern for any
  future testable Tauri command in this crate, following the precedent
  `recents.rs` already set once.
- **Shared-slot tab pattern for the sidebar (WP-1.4):** `LibraryPanel` and
  `ModelTreePanel` both occupy `Slots.ShellLeft` — the platform's `SlotHost`
  has no built-in exclusivity, so a second panel there would just overlap. Fix
  is `sidebarTabStore` (tiny zustand store) + `SidebarTabHeader` (shared tab
  strip both panels render at their own top); each panel returns `null` when
  not the active tab. No slot/platform contract change. If a THIRD panel ever
  wants `ShellLeft`, extend this store, don't invent a second mechanism.
- **Serde camelCase gotcha, found and fixed, worth remembering for any future
  internally-tagged enum:** `#[serde(tag="kind", rename_all="camelCase")]`
  renames variant NAMES but does not cascade into struct-variant FIELD names —
  each field needs its own `#[serde(rename=...)]`. Bit `ComponentSourceRef::Generator`;
  now pinned by `place_component_source_fields_are_camel_case`.

**Dead ends ruled out:**
- Don't try to make `onecad-core`'s param validators enforce the free/table/
  computed parameter-role rule (spec §3.2) — it needs the resolved component
  signature, which `onecad-core` structurally cannot have (no `onecad-library`
  dependency). That enforcement belongs at the WP-1.3 app-crate authoring
  entry point in P2, not in `session.rs`. The plan flagged this split; it's
  confirmed correct, don't relitigate.
- Don't bother writing an external `tests/library_commands.rs` for the
  `*_at`-split command tests — the `*_at` functions are private to the crate,
  so those tests have to live in an in-file `#[cfg(test)] mod tests` block
  inside `library.rs` itself.

## How to resume

1. Run the `handoff` skill with "resume" (or just start on WP-1.5 directly —
   this entry has the detail).
2. Rebuild the worker if it's stale: `ONECAD_OCCT_ROOT=<pinned-prefix>
   scripts/build-worker.sh Release` (must precede any cargo command that
   compiles the app crate — `bundle.externalBin` requires the staged sidecar
   binary to already exist).
3. Next task: **WP-1.5, snap solver.** Per the plan — placement ghost as a
   `ViewportLayer` contribution modeled on
   `src/modules/modeling/datumViewport.ts::createDatumViewportContribution()`
   (the only existing precedent for an imperative controller reaching
   `ViewportContext` from outside React). New
   `src/modules/library/placementViewport.ts`. Loop: hover → raycast →
   `ModelingServices.GeometryQuery.classifyElement` (built this session, live)
   → match against the dragged component's `accepts` → candidate transform
   per spec §5.3 → `CadClient.beginPreview`/`updatePreview` with a candidate
   `PlaceComponent` (reuses `PreviewOp`) → ghost mesh. This is the first WP
   that touches the viewport engine — read `src/viewport/engine/README.md`
   first (Z-up hard invariant, `refreshColors()`, on-demand rendering).
4. After WP-1.5: WP-1.6 (StatusSection + tasks-chip real producer — the chip
   already has a real `begin/setProgress/end` API from the Session 4/
   MODULAR-PLATFORM wave with zero producers; `contributeLibraryUi`'s
   `reindex` call becomes its first), then WP-1.7 (e2e:
   browse→place→snap→save→reopen-without-library).

## Open questions

From the plan's original six — three resolved by implementation, three still
open (none block WP-1.5):
- Q1 (crate boundary) — RESOLVED: `onecad-library` independent of `onecad-core`, see above.
- Q2 (service-build scope) — MOOT, done: `GeometryQuery`/`CommandApi` both real now.
- Q3 (BOLTS ingestion tooling ownership, P2) — still open: one-time offline
  script vs. a maintained in-repo tool re-run on BOLTS updates?
- Q4 (P3 starter-template content) — still open: can I author the 3-5 starter
  templates directly (dogfooding "Save as Component"), or does the user want
  to supply/review actual content (NEMA footprint accuracy, datum placement)?
- Q5 (`document` source kind's nested-replay mechanism, P3) — still open, no
  existing analog found; candidate is Rust-side load-frozen-doc-into-throwaway-
  session, replay, extract BRep. Worth a targeted search for prior art
  (checkpoints / `checkpointFallbackReplay`) before committing, at that time.
- Q6 (library's own service vs. riding modeling's CommandApi) — RESOLVED:
  library owns list/search (its own small surface), placement routes through
  modeling's `CommandApi` per ADR-0002. This is what WP-1.3/1.4 shipped.

## Pointers

- Tasks → `TODO.md` § COMPONENT-LIBRARY (6 dated gate entries, WP-0.1–WP-1.4)
- Snapshot → `CURRENT_STATE.md` § COMPONENT LIBRARY — LIVE DELTA (2026-08-12)
- Plan → `~/.claude/plans/do-thorough-analysis-of-abstract-thunder.md`
- Spec → `TheComponentLibrary/onecad-component-library-spec.md`

---

# Handoff — Platform refactor (Milestones 1 + 2), and what comes next

Session 4 · 2026-08-08

> **TWO LIVE THREADS.** This file now carries both. Session 4 (below) is the
> Platform/module refactor and is COMMITTED as `4145f3f`. Session 3 (further
> down, unchanged) is the Advanced-Fillet roadmap and is still the live handoff
> for that program — its § "VF-M5 gate regression" is the diagnosis behind P1
> here. Read whichever thread you are picking up; read both before pushing.

## Goal

Reorganize OneCAD so future capabilities (FEM, TechDraw, CAM, third-party
addons) can be added as clean modules, without redesigning the modeling engine
or changing any user-visible behavior.

The principle: **modeling stops being synonymous with OneCAD and becomes the
first privileged built-in module running on the OneCAD Platform.**

## Original plan

`~/.claude/plans/velvety-leaping-adleman.md` — seven waves, W0…W7. Scope was
fixed with the user up front: Milestone 1 (frontend platform, spec §195) **plus**
Milestone 2 (module-namespaced persistence, spec §196); **no file moves**;
branded namespaced ids with the `ModelTool`/`SketchTool` unions retained.

Out of scope by decision: SDK package, test addon, addon manifest/loader/host,
GitHub install, resource-store generalization, dynamic Tauri router, crate
extraction, and everything in spec §193.

## Done so far (and why)

All seven waves landed and are committed as `4145f3f`. Full wave-by-wave detail
is in `TODO.md` § PLATFORM REFACTOR; the reasoning worth carrying:

- **Contracts first, refactor second.** Four frozen behavior contracts went into
  `src/test/contracts/` BEFORE anything moved: toolbar arrangement, the three
  keymap tables + cross-mode opt-out, editor mount order, inspector section
  order. `src/test/contracts/README.md` states the rule that makes them worth
  anything: a probe may change when the mechanism changes, a contract may not be
  edited to make a refactor pass.
- **The probes are non-tautological by construction.** The toolbar assertion runs
  against `toolbarFromRegistry` — the arrangement rebuilt FROM the platform
  registry — not against the descriptor table that fed the registry, or it would
  only prove the table equals itself. The keymap probe compares `resolveBinding`
  to an independently written oracle over every (key, shift, mode) triple.
  `architecture.test.ts` carries a POSITIVE CONTROL, because every other
  assertion in it expects an empty list, which is also what a broken scanner
  returns.
- **Registration happens on EDITOR MOUNT, not at bootstrap.** `App.tsx`
  code-splits the editor deliberately and `StartScreen` idle-prefetches that
  exact specifier; hoisting nineteen feature imports into the startup bundle to
  satisfy an architectural preference would make the start screen pay for the
  editor. This forced two real design changes: `platform.createScope(owner)` for
  independent child scopes, and a teardown fix — a scope disposes what IT
  registered and never sweeps the owner, or the editor's short-lived scope would
  tear down the module's bootstrap registrations.
- **Tool ids are scope-qualified** (`…tool.model.mirror` vs `…tool.sketch.mirror`)
  because `select` and `mirror` exist in both unions meaning different things; a
  flat map would have let one silently shadow the other.
- **Separators are derived from group boundaries.** `group` is consumer metadata,
  `priority` is the sort key — so palette order can never start following module
  load timing.
- **Two slots were added deliberately** (`viewport.chrome`, `shell.notification`).
  Without them the frozen mount order cannot be reproduced with contiguous slot
  regions, and mount order is load-bearing (a past defect had tool chips render
  under the side panels and become unclickable).
- **Module state lives in `document.json`** (ADR-0004), skip-if-empty in both the
  document and the manifest, so existing files serialize byte-identically: no
  container bump, no user migration. `ModuleId` validates on AUTHORING only —
  deserialization is permissive because refusing an id a stricter build dislikes
  would destroy exactly the data preservation exists to protect.
- **Programmatic writes use the user's path.** `EditCommand::SetModuleState` has
  a memento inverse; there is no separate mutation lane for automation.

Deviations from the spec, each with the reason recorded in `TODO.md`:
`onecad.shell.workspace.design` rather than `onecad.workspace.design` (a
contribution id must sit under its owner's namespace, and the workspace composes
several modules); no `platform_invoke` router (three typed commands until the
addon host needs one).

### Dead ends / things already ruled out

- **Do not point `build-worker.sh` at `/opt/homebrew/opt/occt-8.0.1`.** It is a
  plain Homebrew install with no `share/onecad/occt-build.json`, so CMake aborts
  with "OCCT artifact metadata is absent". This session mis-reported that as a
  repo blocker; it is not. Use `~/.onecad-occt/8.0.1` and
  `worker/build-pinned` — see § How to resume in the Session 3 thread below.
- **A single full-suite e2e failure is not evidence.** `filletChamfer.spec:169`
  failed once in a full run and passed 13/13 in isolation. The four `theme.spec`
  failures are the real, reproducible, pre-existing ones.
- **Do not split the commit.** M1 (frontend) and M2 (Rust) were gated as one
  tree; splitting produces at least one commit that does not build, because
  `platform/index.ts` exports `documentState`, which needs the M2 client methods.
- **Do not `git checkout -b` in this tree.** A concurrent session shares the
  working directory; branching silently redirects THEIR commits too.

## How to resume

1. Run the `handoff` skill with "resume".
2. Re-read `CLAUDE.md` — it gained a § "Architecture laws" section that binds new
   code — then `docs/ARCHITECTURE.md` (normative) and `docs/adr/0001`–`0008`
   (why). Those three are the standing rules for this program.
3. Work `TODO.md` § NEXT SESSION, which has the full checklist. The shape:
   - **P0 (do first, ~30 min).** Manual `tauri dev` smoke — the only
     Definition-of-Done item with no evidence. Then rebuild the sidecar against
     the pinned prefix and re-run the worker lane so the numbers describe HEAD.
     `topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean`
     is EXPECTED to fail there; that is P1, not a refactor regression.
   - **P1.** Close the VF-M5 gate regression. Belongs to the other session —
     check `git log` for new `wip:` commits and coordinate before starting. The
     diagnosis is already written in the Session 3 thread below; do not re-derive
     it.
   - **P2.** Finish Milestone 1: toolbar reads the registry live; inspector
     sections, tree nodes and viewport layers become real contributions (the
     contracts exist with zero producers); move `zoomFit`/`home` off modeling.
     Mechanical — the golden tests already pin every answer.
   - **P3.** Milestone 3: `@onecad/sdk` + a bundled test addon that imports
     nothing else. **Do P2 first** — an SDK frozen over a half-converted surface
     freezes the wrong shape.
4. Suites, all expected green except where noted:
   ```bash
   bunx tsc --noEmit && bun run test        # 221 files / 3796
   bunx playwright test                     # 386 pass / 4 fail (theme.spec, pre-existing)
   cd src-tauri && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
   ```

## Open questions

- **Push?** `master` is 5 commits ahead of `origin/master`, nothing pushed. Two
  of those commits (`685efc2`, `069bb48`) are the concurrent session's in-flight
  `wip:` work. Coordinate before `git push`.
- **P1 fix direction** — keep the anchor-exact carve-out enabled until a genuine
  restore signal is plumbed, or plumb one now? Recorded as a choice, not decided.
- **Four untracked paths** (`.opencode/`, `.agents/`, `STEP/`, `skills-lock.json`)
  still want a `.gitignore` entry rather than a commit. Nobody has decided.
- **Should `TitleBar`/`StatusBar`/`NavPill`/`CornerCluster` stay an
  `onecad.shell` module**, or go back to being permanent hard-coded structure?
  Implemented as a module for consistency; the simpler alternative is defensible.

## Pointers

- Tasks → `TODO.md` § NEXT SESSION · Snapshot → `CURRENT_STATE.md`
- Architecture laws → `docs/ARCHITECTURE.md` · decisions → `docs/adr/`
- Frozen behavior contracts → `src/test/contracts/README.md`
- Plan → `~/.claude/plans/velvety-leaping-adleman.md`

---

# Handoff — Advanced-Fillet roadmap (M0 · M1 · M2)

Session 3 · 2026-08-08

Supersedes the Session 1 handoff (TRUST + PREVIEW), which is closed. Sessions 2
and 3 both worked this roadmap; M2 is now complete.

## Goal

Make Advanced Fillet OneCAD's first genuinely differentiated geometric
operation, and build the infrastructure that can **prove, case by case, that
OneCAD is safer and increasingly more capable than raw OCCT**.

Effort split: kernel robustness 70% · direct modeling 20% · UI 10%. Loft/sweep,
assemblies, and release work are all deferred.

The headline KPI is the **raw-OCCT rescue rate** — same input, same intent, raw
OCCT fails, OneCAD succeeds, and both a deep audit and a semantic test pass —
under two hard invariants:

- `raw PASS → OneCAD FAIL` on a supported case is a serious regression.
- Returning invalid or semantically wrong geometry is **always worse** than
  safely refusing.

## Original plan

M0 reproducible-green → M1 QA automation → M2 case-v2 + fillet geometry
generators → M3 metamorphs → M4 failure taxonomy + semantic validators → M5
`--jobs/--shard/--resume` → M6 minimizer + regression promotion → M7 SQLite
ingest + HTML dashboard → M8 large characterization campaign (production fillet
algorithms UNTOUCHED) → M9 baseline KPIs → M10 `FilletBuilder(FilletDefinition)`
→ M11 OCCT capability spike → M12 variable radius → M13 critical-radius
diagnostics → M14 second campaign → M15 first evidence-driven rescue strategy →
M16 measure rescue improvement → M17 chord width / verified G2 / overflow /
corners → M18 Shapr3D goldens → M19 DirectEditPlanner + reblend.

Full roadmap with the 15 agent constraints: `TODO.md` § ADVANCED-FILLET ROADMAP.
**M2–M19 is a multi-month program.** This session delivered M0, M1, and the
first half of M2.

## Done so far (and why)

### M0.1 — OCCT build fingerprint (`scripts/occt-fingerprint.sh`, new)

`build-pinned-occt.sh` required every `-D` to appear literally in
`CMakeCache.txt`. Root-caused against the pinned source **and** a real configure:
OCCT's `OCCT_CHECK_AND_UNSET` does `unset(VAR CACHE)`, and each `CAN_USE_*` comes
from scanning `BUILD_TOOLKITS`. With `BUILD_MODULE_Draw=OFF` nothing requires
`CSF_TclTkLibs`, so `CAN_USE_TK` is OFF and CMakeLists (8.0.1 L558-565; 7.9.3
byte-identical over every `USE_*`/`CAN_USE_*`/`OCCT_CHECK_AND_UNSET` line)
DELETES our `-DUSE_TK=OFF`. Nine further keys can vanish the same way.

The normalizer represents **effective configuration**: an absent DEPENDENT key
is `OFF` *only when the policy requested OFF* (OCCT deleted it precisely because
the feature is not in the build). Requested-ON-but-dropped, a missing REQUIRED
key, a present key disagreeing with policy, and a value outside CMake's truth
constants are all still fatal. Normalization, not relaxation.

Verified end to end by building pinned OCCT 8.0.1 from scratch into a clean
prefix — `USE_TK` really is absent from the produced cache, and the old script
would have aborted there.

### M0.2 — frontend clean build

`bun run build` was red while `bun run test` was green, because **vitest does not
typecheck**. `Rgba` is now `readonly` (a color is a value, never mutated) and the
~20 duplicated raw `[number,number,number,number]` literals collapse onto it;
read-only consumers take `ReadonlyMap`. New `src/test/fixtures/bodyMeshView.ts`
replaces a hand-written 24-field structural mock.

Also fixed a real hex-gate violation found while gating: `InspectorPanel`
hardcoded `#a9aeb6`, which **is** `--color-body-fill`. Now read through
`palette.bodyNeutral()`, so the no-color swatch follows the theme.

### M0.4 — viewport auto-fit regression (a real product bug the gate caught)

`1fe0cef` replaced "auto-fit once on the first body" with a 250 ms debounced
re-fit — right intent (a multi-body assembly must not be framed on whichever body
streamed in first), wrong place: the timer lived in the React bridge, so a
`fitView()` tween could START after everything else already believed the camera
had settled.

Bisected, not guessed: `measure.spec.ts` is 5/5 at `d1c5339`, **1/5 at
`1fe0cef`**, and 5/5 at `1fe0cef` with only that hunk reverted.

Fix keeps the intent and makes the scheduled state real: `ViewportEngine`
`requestAutoFit()` / `autoFitPending` / `cancelAutoFit()`, cancelled by explicit
`fitView`, `fitToBodies`, `enterSketch`, and `dispose`. User-visible too — a
queued fit could snap the camera mid-interaction, and one landing during sketch
entry was saved as the restore pose.

### M0.5 / M0.6 — the remaining 7 e2e failures, root-caused not retried

All seven reproduced on a pristine `1fe0cef`; none came from this work package.

- **`transform-body` ×4** — the spec's `bodyBounds` folded the fat-line EDGE
  layer into the bbox. `Line2` reports `isMesh === true`, and its `position` is
  the *instanced unit-quad template* `(-1,-1,0)…(1,2,0)` (real endpoints live in
  `instanceStart`/`instanceEnd`). Every body's min was clamped to ≤ -1 while
  `max` stayed correct — exactly the observed `[-37, -1, -15]`. Traversal now
  takes `userData.kind === "face"` only.
- **`history-inline-dimension:213`** — `toolApplicability.ts` is NEW in
  `1fe0cef`; the toolbar is selection-gated now, and committing an extrude hides
  its sketch, so Extrude sat `aria-disabled` and the click burned the 45 s
  timeout. Second cause: `getByLabel("Dimension value")` matched both the row
  editor and the tool chip.
- **`hole:226`** — `findFaceOnBody` probed pixels before the camera settled.
- **`sketch-degenerate:35` + `live-dim-mouse-rounding:62`** — `enterSketch` aims
  along the plane normal at `controls.getDistance()`, so the sketch view
  **inherits the model camera distance**, which sets `planePixelWorld`, hence the
  draw tools' screen-constant reject radius (`minSize = 4 × planePixelWorld`)
  AND the zoom-adaptive dimension quantum. Entering mid-fit made both
  non-deterministic; every caller settled AFTER entering, too late.

Same class as the auto-fit bug: **camera state read before it settled.**

`playwright.config.ts` now has `retries: 0` everywhere (user decision). A CI
retry is what let the auto-fit regression read as green.

### M1 — manual-QA debt triaged

All ~112 boxes of the historical checklist classified: **78 retired** against
existing automated tests (each cited `path:line`), **22 named gaps** (GAP-K/H/F,
each with its target lane), **12 genuinely manual**.

- `docs/qa/MANUAL_RELEASE_GATES.md` — the 12 survivors, grouped by *why* a
  machine cannot judge them: visual 4 / native 4 / hardware 4. Carries an
  explicit "do not grow this file" rule.
- `docs/qa/MANUAL_GATES_TRIAGE.md` — item-by-item classification + the gap backlog.
- `docs/qa/archive/MANUAL_GATES_RUN-2026-08-04.md` — historical list, verbatim.

### M2 part 1 — kernelbench case-v2

- `bench/robustness/schemas/case-v2.schema.json` — strict throughout.
- `src-tauri/crates/onecad-kernelbench/src/case_v2.rs` — a separate type set.
- `bench/robustness/examples/fillet-matrix-plane-cylinder-v2.json` — ajv-validated
  example. Deliberately NOT in `regressions/`: regressions are contractual.

Shape decisions:

- **`selector` is top level** (v1 nested it under the operation) — the same
  geometry+selection is reused across radius laws and later across operations, so
  a resolving selector is a *precondition*, not part of the definition.
- **`operation.definition` mirrors the kernel-level `FilletDefinition`** — radius
  law + continuity — so `constant`/`linear`/`controlPoints` are all expressible
  and adding one later is additive on both sides.
- **`geometry.parameters` is a typed per-recipe union**, not the free-form object
  the plan sketched. A free-form bag lets a typo silently generate different
  geometry — the one failure mode a fuzzing corpus cannot survive.
- **`continuity` admits only `g1`; `sizeType` only `radius`.** Per agent rules 11
  and 15: an option is not a capability. G2 and chord width become expressible
  only once a validator can prove the result.

Guards: v1/v2 are provably disjoint (tested both directions in Rust AND ajv); a
test asserts v1 regressions still validate under `Case`; the round-trip
reproduces the committed file, which already caught three missing
`#[serde(rename)]` (`startRadius`, `relativeDelta`, `anchorIndex`).

### M2 part 2 — geometry generators + the v2 execution lane (session 3)

The worker executes a `schemaVersion: 2` case and the supervisor drives one end
to end. No production fillet algorithm touched.

**The C++ `CaseSpec` is normalized, not forked.** `Types.h` carries the v2 shape
and the v1 parse fills the subset, so ONE execution path serves both formats.
Forking the executor per version is exactly where a v2-only bug would hide.
`parse_case` dispatches on `schemaVersion`; `CaseParserV2.cpp` mirrors
`case_v2.rs::validate` field for field (Rust is authoritative for what a case may
CONTAIN, C++ for what the runner will EXECUTE, and a disagreement makes a valid
case fail as an invalid request); shared primitives moved to
`CaseParserShared.h`. A non-constant radius law is REFUSED at parse rather than
flattened to its peak — variable radius is M12, with a validator that can prove
the law.

**`GeometrySupportPair.cpp` is the milestone.** Two constructions:

- **Prism family** (plane|cylinder × plane|cylinder) — one 2D profile extruded
  along Z. Every support is prismatic, so the shared edge is the straight
  vertical line through the ORIGIN and the dihedral is free over `(0,180)`. A
  planar support is a rotated half-space box; a cylindrical one is a cylinder
  tangent to the support line AT the origin with material inside, so near the
  edge it is locally the same half-plane and the dihedral is exactly the profile
  angle. The solid is their `BRepAlgoAPI_Common`.
- **Cone family** (plane × cone) — a frustum's base circle. Here the dihedral is
  NOT free: it is `90 - halfAngle` by construction. The generator rebuilds it
  from the half angle and refuses a case that declares anything else, rather
  than silently generating different geometry than the file describes.

Why the prism form and not the committed example's base-circle form: that locked
plane↔cylinder to a 90° dihedral, and the dihedral is the main conditioning axis
for fillet failure. The prism form also keeps the blend a CYLINDER, which is what
lets `constantRadius`/`cylindricalRadius` gate it at all. The example was updated
to match — examples are explicitly not contractual, `regressions/` is untouched.

**Rust `PreparedCase`** (`prepared.rs`) is the version-agnostic view the
supervisor executes: identity, resource ceilings, metamorph tolerance, canonical
document. `campaign`/`runner`/`child`/`result`/`result_validation` route through
it, so none of them grew a `match` on the schema version. `suite_v2.rs` is the
`fillet/matrix` preset `m1` (24 cases → 60 records); `cli` gained the suite and a
version-dispatched `run-case`.

**Two real defects, both found by RUNNING it rather than by unit tests:**

1. The result envelope echoed the case's own `generator` block, so case-v2's
   extra `family` field failed the supervisor's strict result validation —
   60/60 red with an empty stderr. The RESULT schema is frozen at v1, so
   `Execution.cpp` now BUILDS the `{name, version, seed}` subset.
2. `parse_case_v2` never re-checked `schemaVersion` itself — only the dispatch
   did — so calling it directly on a v1 document read it as v2. Fail-closed on
   both sides now, caught by the new ctest fixture.

**Sizing is measured, not assumed.** A blend needs room; for a prismatic pair the
throat is `R·(1 - cos θ)` where `R` is the tightest curved support (half a planar
support's width when neither is curved). Swept against OCCT 8.0.1: every pair
blends at 0.20 of that throat, the cylinder↔cylinder lens refuses at 0.40.
Supported cases sit at 0.04–0.16, so a red there is a kernel regression and not a
greedy case.

**Finding — `cylindricalRadius` and `g1BoundaryTangency` are box-shaped.** The
first counts EVERY cylindrical face in the output against the requested radius,
so a cylindrical SUPPORT reads as a blend of the wrong radius (17.0 mm error
measured on a 20 mm support); the second only recognises plane↔cylinder tangency
pairs and cannot see a blend meeting a curved support. Both are emitted only for
all-planar pairs, pinned by test. Their recipe-agnostic replacements
(`supportTangency`, `crossSectionProfile`) are expressible in case-v2 but
unimplemented, and an unimplemented validator reports `notApplicable` — which
FAILS a required check rather than passing it — so emitting them now would red
the whole suite. They land with M4.

### Dead ends / things already ruled out

- The e2e failures are **not** flakiness to be retried away — every one had a
  mechanical cause. Don't reintroduce `retries`.
- `sketch-degenerate` and `live-dim-mouse-rounding` pass in isolation and fail
  in-suite. Isolation runs prove nothing here; use a full run or a pristine
  worktree.
- A full-suite e2e number is **not attributable** while a second session edits
  the tree — one run hit a live `[vite] Transform failed` from a mid-edit save.

## How to resume

0. `master` is **4 commits ahead of `origin/master` and unpushed**; the working
   tree is clean apart from four deliberately-untracked paths. Two of those
   commits are a concurrent session's `wip:` work — do not push without
   coordinating (see § Open questions).
1. Run the `handoff` skill with "resume".
2. Re-read `CLAUDE.md` (kernelbench + protocol conventions) and `TODO.md`
   § ADVANCED-FILLET ROADMAP, especially the **15 agent constraints** — they are
   the standing rules for this whole program.
3. Build/verify:
   ```bash
   ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" \
   ONECAD_WORKER_BUILD_DIR="$PWD/worker/build-pinned" scripts/build-worker.sh Release
   ctest --test-dir worker/build-pinned --output-on-failure
   cd src-tauri && ONECAD_WORKER_PATH=$PWD/../worker/build-pinned/onecad-worker \
     ONECAD_REQUIRE_WORKER=1 cargo test --workspace
   ```
   Pinned OCCT 8.0.1 is already installed at `~/.onecad-occt/8.0.1`; the build
   script short-circuits on its fingerprint.
4. Run both suites to confirm the baseline before changing anything:
   ```bash
   export ONECAD_KERNELBENCH_RUNNER=$PWD/worker/build-pinned/onecad-kernelbench-runner
   cd src-tauri
   cargo run --release -p onecad-kernelbench -- run \
     --suite fillet/foundation --preset t0 --backend both --out-dir /tmp/t0
   cargo run --release -p onecad-kernelbench -- run \
     --suite fillet/matrix --preset m1 --backend both --out-dir /tmp/m1
   ```
   T0 must stay 136/136 with `gatingFailures: 0`; M1 is 120 records, 114 pass +
   6 characterization.
5. **Next task is M3** — metamorph EXECUTION. The v2 metamorph set (mirror,
   uniformScale, farOriginTranslation, parameterEpsilon, edgeOrderPermutation,
   contourSeed) is expressible in the schema and validated on both sides, but
   only `translation` and `rotation` are actually executed: `VariantSpec` in
   `worker/src/benchmark/Types.h` still carries only translation + rotation, and
   `suite::Variant` mirrors it. Widening that pair is the work.
   Then M4, whose recipe-agnostic validators are what unblock required
   validators on curved-support pairs.
6. **KBR case-v1 stays frozen.** Never edit `case.rs` or `case-v1.schema.json`.
   `examples/` is NOT contractual; `regressions/` and `presets/` are.

## VF-M5 gate regression (RESOLVED 2026-08-09 — kept for the record)

**Both halves are now closed, and one claim below is FALSE.** The regression was
real and was fixed by `from_zero_replay = false`. But "V1 has no checkpoint
plumbing / there are no restores" is wrong: checkpoints are plumbed end to end and
the regen executor's F12 fallback reaches exactly the hazardous lane. That residual
is now closed by the SCHEMA §7.2 `checkpointFallbackReplay` field, which is the
"plumb one" option this note offered — proven red-first by
`topology_rebind::vfm5_lane_d_checkpoint_fallback_replay_must_not_bind_the_decoy`.
See `TODO.md` § VF-M5 RESIDUAL. The text below is the original note, unedited.

### Original note

**The `from_zero_replay` gate in `069bb48` is WRONG and re-opens the defect-fix
gate.** Real-worker lane (`ONECAD_REQUIRE_WORKER=1 cargo test --workspace`,
worker built from HEAD) fails
`topology_rebind::h6a_flagship_edit_lane_fillet_survives_and_reopens_clean`
(`needsRepair` expected 0, observed 1). Baseline worker passes the same —
regression is the gate. Root cause: V1 has no checkpoint plumbing, so the
setup gate `partition.size() == 0` (worker `PlanExecutor`) is ALWAYS true and
the gate degenerates to `edited_from.is_present()`; the flagship edit lane
(`ToEnd { from: 1 }`, carries `edited_from`) is falsely treated as a
from-zero-replay → anchor-exact carve-out off → model flagged `NeedsRepair`.
The VF-M5 scenario (stale world anchors after a real checkpoint RESTORE) cannot
occur in V1 — there are no restores. Fix: keep the carve-out enabled until a
genuine `baseCheckpoint`/restore signal is plumbed into the plan, or plumb one.
See `CURRENT_STATE.md` for the full writeup. Rebuild the worker before
re-running the real-worker lane — the on-disk sidecar `worker/build/` is
currently a stale pre-gate build (19:37).

## Open questions

- **Commit boundary — RESOLVED, but read this before pushing.** The backlog was
  split into four commits on `master` at this session's close (see
  `CURRENT_STATE.md` for hashes). Two of them, `685efc2` and `069bb48`, are a
  CONCURRENT SESSION'S in-flight work, committed as `wip:` so it was not left
  loose; their message bodies say plainly that this session neither authored nor
  gated them. **Nothing is pushed.** If that session is still running it will
  keep editing these files, so coordinate before `git push` — a force-push or a
  rebase over their in-flight tree is the thing to avoid.
- **The `wip:` commits carry a known-red build.** `bun run build` fails on
  `src/ipc/mockClient.import.test.ts:106` (`'snap' is possibly 'null'`), which is
  now committed inside `685efc2`. Same class as the M0.2 bug: vitest is green
  (209 files / 3698 tests) because it does not typecheck. Not a M2 problem —
  M2 is Rust + C++ only — but `master` does not currently build the frontend.
- **Two e2e failures belong to that session, not to this work:**
  `theme.spec:121,145` (`getByRole('button', {name: /^Appearance:/})` not found,
  from their `TitleBar.tsx`) and webkit-only `boolean-preview`.
- **Four paths are deliberately untracked:** `.opencode/` (61 MB), `.agents/`,
  `STEP/`, `skills-lock.json`. They want a `.gitignore` entry, not a commit;
  nobody has decided which.

## Pointers

- Tasks → `TODO.md` · Snapshot → `CURRENT_STATE.md`
- Roadmap + 15 agent constraints → `TODO.md` § ADVANCED-FILLET ROADMAP
- QA triage → `docs/qa/MANUAL_GATES_TRIAGE.md` · release gates →
  `docs/qa/MANUAL_RELEASE_GATES.md`
- Benchmark contract → `bench/robustness/README.md`
