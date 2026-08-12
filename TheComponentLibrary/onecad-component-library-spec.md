# OneCAD Component Library — Implementation Specification

> **Read this first.** You are implementing a major new feature in OneCAD: a **Component Library** of reusable mechanical components (screws, bearings, motors, and user-authored parts) that users place into documents with mate-like snapping, plus **project templates**. This spec is the contract. It names the real seams you must integrate with; where it gives exact type signatures (protocol, ops, storage) you must match them; where it describes components at the module level you have latitude on internals. Every phase is independently testable and gated.
>
> **This is a four-layer repo.** TypeScript/React frontend (`src/`), Tauri + Rust workspace (`src-tauri/`), C++20 OCCT worker sidecar (`worker/`), Playwright e2e (`e2e/`). The invariants below are load-bearing — violating one is a systemic defect, not a style issue.

---

## 0. Ground rules — invariants you must not violate

These come from the repo's own architecture (docs/ARCHITECTURE.md, docs/adr/, protocol/SCHEMA.md). They are enforced by tests and code review. **Do not relitigate them; build within them.**

1. **No JavaScript on the kernel path.** Only Rust (`serde_json`) and C++ (`nlohmann::json`) parse wire envelopes. The frontend only ever receives projection DTOs. `u64` values must stay safe — never route a raw 64-bit int through JS.
2. **ADR-0002 — the modeling kernel is closed.** The component library is a **built-in module**, not an addon. All modeling access goes through the modeling module's published services (`onecad.modeling.command-api`, `onecad.modeling.geometry-query`) or the transaction layer. Never reach into `history.rs`, `regen.rs`, `WorkerSession`, or the OCW1 protocol from library UI code. New operations are added to the `KnownOperation` enum as built-ins — this is allowed and expected; injecting them from an addon would not be.
3. **ADR-0003 — UI goes in controlled slots.** The library panel, configurator, placement ghost, and progress indicator are slot contributions, never raw DOM. The host owns panel frame/resizing/focus/theme; the contribution owns content only. No `document.querySelector`, no absolute pixel placement.
4. **Deterministic `NeedsRepair` beats a silent wrong bind — this is the product's founding invariant and the library's core value.** A placed component whose definition is missing or changed surfaces `NeedsRepair`. **Never** silently substitute geometry (this is the SolidWorks Toolbox failure mode: a fastener reverting to its largest size on another machine). Geometry is always cached locally so a document opens with zero library dependency; the cache is authoritative for display, the definition for identity.
5. **`protocol/SCHEMA.md` is normative.** Any wire change needs a linked Rust + C++ update, a §14 changelog entry, a fixture bump, and cross-track sign-off. Op payload schemas go in §7.3.
6. **Worker stdout carries OCW1 frames only.** All logs to stderr.
7. **Design tokens only.** `src/styles/tokens.css` is the sole source of colors. No raw hex in TS/TSX; theming works by redeclaring tokens under `:root[data-theme="dark"]`.
8. **Rust is the sole hash authority** for ElementIds; NewBody BodyIds are worker-minted deterministic `body_<opId>` (or `body_<opId>:<k>` for N bodies), adopted by Rust at `AcceptPrepared` with format + uniqueness validation.
9. **Platform must not depend on modeling.** `src/platform/**` may not import `@/features`, `@/tools`, `@/modules`, or a modeling store. The library module respects the same wall in the other direction.
10. **Validation at authoring, not deserialize.** Params `validate()` runs at authoring entry points (`crate::edit::session`), so a document written by a future build still loads. Match this pattern.

**Build reality for the agent:** package manager is **bun**. The worker must be staged before any cargo command that compiles the app crate — run `scripts/build-worker.sh Release` first. Set `ONECAD_REQUIRE_WORKER=1` for worker-backed gates. OCCT 8.0.1 is built via `scripts/build-pinned-occt.sh`, passed as `ONECAD_OCCT_ROOT`. Playwright `retries: 0` everywhere. Every gate command must actually be run and pass; never claim a gate passed without running it.

---

## 1. What you are building

A **Component** is a library entity usable as a placeable part. The central design rule: **a static STEP solid, a parametric generator definition, and a user-authored document are three kinds of the same Component concept.** They share identity, metadata, a parameter signature, and attachments; they differ only in how geometry is produced.

**Capabilities, in scope for this spec:**
- Browse/search a local component library in a docked panel; drag-to-place with **mate-like snapping** (concentric on cylinders/holes, flush on planar faces, auto-size on hole rims).
- **Static and parametric components from day one.** Parametric standard fasteners (ISO 4762, 7380, 4014/4017, 4032, 7089/7093), table-driven bearings (ISO 15 series), and NEMA 17/23 motor frames, generated by built-in, versioned generators running through the existing worker op path. Fastener dimension tables are seeded from the open-licensed **BOLTS** dataset.
- **Placed components are first-class instances** with identity, editable parameters, replace-in-place, and detach — implemented as a new `KnownOperation` family with locally cached geometry.
- **Persistent mates**: a placed component records its attachment as a constraint; when the target face moves, the component re-seats on regen (and surfaces `NeedsRepair` if the target vanishes).
- **User authoring**: "Save as Component" captures a body/selection as a reusable parametric component with declared parameters and author-placed attachments.
- **Project templates**: frozen `.onecad` documents offered on the start screen (which already has a stubbed `templates` nav key).

---

## 2. The Component package format

The on-disk library is a directory tree. A **package = one directory per component**, self-describing. The same format is used locally now and served by a future registry — the format is the registry commitment.

```
<library-root>/
├── library.json                      # the index — IS the future registry manifest
├── onecad.std.iso4762/
│   ├── component.toml                # the manifest (below)
│   ├── tables/iso4762.json           # dimension data (generator kind)
│   └── preview.webp                  # thumbnail
├── onecad.std.bearing.608/
│   └── …
└── <author>.<part-name>/             # user-authored
    ├── component.toml
    ├── source.onecad                 # frozen document (document kind)
    └── preview.webp
```

### 2.1 `component.toml` — normative schema

```toml
[identity]
id        = "onecad.std.iso4762"     # stable, namespaced, never reused. <ns>.<...>
version   = "1.0.0"                  # semver; drives opt-in upgrade
revision  = "sha256:9f2c…"           # content hash over the whole package

[metadata]
name        = "Socket Head Cap Screw"
standard    = "ISO 4762"             # optional
designation = "ISO 4762 M{thread}x{length}"   # BOM string template over param keys
category    = ["fasteners", "socket-head"]
tags        = ["metric", "shcs"]
unit        = "mm"

[source]
kind = "generator"                   # exactly one of: embedded | generator | document
# embedded:  blob = "sha256:…"      (a STEP/BREP payload in the blob store)
# generator: generator = "iso4762", generator_version = 1
# document:  file = "source.onecad"

[parameters]
# role: free (user-editable per instance) | table (derived from dim table, locked)
#       | computed (pure formula, locked)
thread        = { role = "free",  key = "M6", domain = ["M3","M4","M5","M6","M8"] }
length        = { role = "free",  value = 20, snap = "preferred", min = 4 }
head_d        = { role = "table", from = "iso4762.dk" }
thread_detail = { role = "free",  value = "cosmetic", domain = ["cosmetic","simplified","modeled"] }

[attachments]
# named mate points. frame = a local basis on named geometry; accepts = geometry kinds it mates with.
head_seat  = { on = "face:head_underside", accepts = ["plane"] }
shank_axis = { on = "cylinder:shank",      accepts = ["cylinder", "hole", "circularEdge"] }
```

### 2.2 The index — `library.json`

Maps `id → version → package path`, and caches search metadata (name, category, tags, source kind, parameter keys). Written atomically. When a remote registry arrives later, this same schema gains remote entries plus a download-on-first-use blob cache; **the local format does not change.** Keep the schema versioned (`"indexVersion": 1`).

### 2.3 Blob storage

Reuse the existing **content-addressed** pattern from `src-tauri/crates/onecad-core/src/io/imports.rs` (`ImportBlob`, `import_blob_path(sha256, codec)`). Geometry and STEP payloads are stored once under `blobs/<sha256>` and referenced by hash, deduplicated across components, integrity-checked on read. Do not invent a parallel store — generalize the import-blob machinery.

---

## 3. The operation family (new `KnownOperation` variants)

Placed components are **first-class instances**, not copied bodies. This is the decision that aligns the library with the founding invariant. Add four operations. Each is a single atomic, undoable op; each publishes as a uniquely-named, editable history node (per the modeling-UX contract — no inert insertions).

Follow the existing `*Params` idiom exactly (see `TransformBodyParams` / `ImportStepParams` in `src-tauri/crates/onecad-core/src/document/record.rs`): `#[serde(rename_all = "camelCase")]`, `#[serde(flatten)] extra: Extra` for forward-compat, a `validate()` checked at authoring, rich doc comments referencing SCHEMA §7.3.

### 3.1 `PlaceComponent`

Instantiate a library component. Records identity + parameters + an optional target attachment + a frozen placement; publishes the resolved geometry as its result bodies.

```rust
pub struct PlaceComponentParams {
    /// Library identity the instance was placed from.
    pub component_id: String,          // namespaced id, e.g. "onecad.std.iso4762"
    pub component_version: String,     // semver
    pub component_revision: String,    // "sha256:…" content hash at place time
    /// Free-parameter overrides for this instance (thread, length, …). Keys must
    /// exist in the component signature and be role=free; validated at authoring.
    #[serde(default)]
    pub params: BTreeMap<String, ComponentParamValue>,
    /// Source resolution, so regen can re-derive geometry without the library:
    ///   embedded  → blob sha256 + codec (reuse ImportSourceCodec)
    ///   generator → generator id + version + resolved param set
    ///   document  → frozen document blob sha256 + param overrides
    pub source: ComponentSourceRef,
    /// Optional placement mate recorded at insert. ElementRef pair + the snap kind.
    /// Absent ⇒ dropped in free space, positioned by `placement` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate: Option<ComponentMate>,
    /// Frozen world placement (pivot + rotation), TransformBody-style. Frozen at
    /// authoring; re-edits recompose against the stored pivot so repeated edits
    /// are exact (no drift). When `mate` is present this is the RESOLVED transform
    /// the mate produced, kept as the fallback if the mate later fails to resolve.
    pub placement: FrozenPlacement,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

pub struct ComponentMate {
    /// Attachment on the placed component (names a key in the component's
    /// [attachments] table) — the component's own local frame.
    pub self_attachment: String,
    /// The target element in the document, as a full ElementRef (primary +
    /// intent descriptor + anchor) so the ladder can re-resolve it after upstream
    /// edits. THIS is what makes the mate persistent.
    pub target: ElementRef,
    /// The snap classification that produced the mate: concentric | coincident |
    /// concentricAndCoincident. Drives how regen re-seats the component.
    pub kind: MateKind,
    /// Orientation flip at insert (the A-key). Applied on top of the solved frame.
    #[serde(default)]
    pub flipped: bool,
}
```

**Lineage:** `PlaceComponent` mints new bodies (one solid ⇒ `body_<opId>`), `modified` on nothing. The placed solid must pass the existing single-solid publication policy (`single_solid_policy` in `worker/src/kernel/validation/ShapeAudit.cpp`) — a component resolves to exactly one solid in v1 (see §9, open constraint on compounds).

**Geometry resolution on regen:** the worker resolves `source` to geometry (reads the blob / runs the generator / replays the document with overrides), then applies the transform: if `mate` is present, re-resolve `mate.target` through the ladder, re-seat the component via the mate kind + flip; on resolution failure, emit `NeedsRepair` (never substitute, never fall back silently to `placement` — the stale `placement` is used for *display* only, with the instance flagged).

### 3.2 `SetComponentParams`

Edit a placed instance's free parameters (M6×20 → M8×30). Re-resolves `source` with the merged param set, re-publishes geometry, keeps identity and any mates whose target still resolves. Params not declared `free` in the component signature are rejected at authoring.

```rust
pub struct SetComponentParamsParams {
    pub instance: ElementRef,                 // the placed component body/op
    pub set: BTreeMap<String, ComponentParamValue>,   // free keys only
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}
```

### 3.3 `ReplaceComponent`

Swap an instance to a different component (all M6 SHCS → flanged button head). Carries the new `component_id`/`version`/`revision`/params; mates re-resolve through the ladder by attachment name where the new component declares a matching attachment; failures surface `NeedsRepair`. This is an in-place edit of the existing instance op, preserving its `RecordId` and downstream refs — the same trick `Hole` uses to keep identity across profile edits.

### 3.4 `DetachComponent`

Drop identity, keep the cached geometry as ordinary bodies. The honest "break link." The result is exactly what a static copy-in would have produced, on demand. After detach, no `component_*` fields remain; the op becomes inert provenance.

### 3.5 Wire + schema obligations

For each op: add the `KnownOperation` variant and params struct in `record.rs`; lower it in `src-tauri/src/worker/wire.rs` (`wire_op`, and `wire_op_inputs` — the `inputs[]` semantic-ref slot table must agree with `KnownOperation::inputs`; `mate.target` is a semantic ref and belongs in `inputs[]`); add a dispatch arm in `worker/src/session/PlanExecutor.cpp::run_single_op`; implement `worker/src/ops/ComponentOp.cpp/.h`; document the payload in SCHEMA §7.3 with a §14 changelog entry and a fixture bump. **Cross-track sign-off required.**

---

## 4. Storage layer — new Rust crate

Create `src-tauri/crates/onecad-library`. Pure domain crate, no Tauri (mirroring `onecad-core`'s discipline). Responsibilities:

- **Package read/write**: parse/validate `component.toml`; enforce identity rules (namespaced id, semver, revision hash recomputed on write).
- **Index management**: load/rebuild `library.json`; atomic writes; reconcile packages on disk against the index.
- **Blob store**: generalize `onecad-core`'s import-blob content-addressing into a shared store the library root uses; dedupe by sha256; integrity-check on read.
- **Resolution**: given `{id, version, revision, source}`, produce the worker-consumable geometry source (blob handle / generator invocation / frozen document + overrides). Verify the package revision still matches the recorded hash → on mismatch or absence, return a typed `LibraryError` that the op layer converts to `NeedsRepair` (never an `Err` that breaks document load).
- **Parametric tables**: load and validate generator dimension tables (§6). Spot-check assertions per size against the standard.
- **Registry-ready**: all network access behind a trait (`RegistrySource`) with a `LocalRegistry` impl now; a `RemoteRegistry` later. No network in v1.

**Suggested surface (component-level — you own internals):**

```rust
pub struct Library { root: PathBuf, /* … */ }
impl Library {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, LibraryError>;
    pub fn index(&self) -> &LibraryIndex;
    pub fn reindex(&mut self) -> Result<(), LibraryError>;           // feeds the tasks chip
    pub fn get(&self, id: &str, version: Option<&str>) -> Option<ComponentPackage>;
    pub fn resolve_source(&self, inst: &ComponentSourceRef)
        -> Result<ResolvedSource, LibraryError>;                     // hash-verified
    pub fn save_component(&mut self, pkg: NewComponent) -> Result<ComponentId, LibraryError>;
    pub fn save_template(&mut self, t: NewTemplate) -> Result<TemplateId, LibraryError>;
}
```

---

## 5. The placement solver (mate-like snapping)

Target: full mate-like placement. The encouraging fact from reconnaissance: **OneCAD already owns most of the machinery.** `ElementRef`/`AnchorIntent` (`src-tauri/crates/onecad-core/src/document/refs.rs`) already carries a `localFrame` (origin + x/y/z basis) and `surface_uv` captured at selection, and the ladder already re-finds faces after upstream edits. A mate is structurally a pair of `ElementRef`s plus a transform constraint — build on the naming spine, not a parallel picking system.

### 5.1 Live preview

The drag ghost maps directly onto the existing **`PreviewOp`** verb (SCHEMA §7.6): runs one candidate op against a throwaway head copy, returns MESH1, fencing-free, `expectedSnapshotId` stale-guard. The library's drag-place issues `PreviewOp` with a candidate `PlaceComponent` per frame; commit issues the real op through the transaction layer. Do not build a second preview mapper — SCHEMA forbids it.

### 5.2 Face classification on hover

The snap solver classifies hovered geometry by type and matches it against the dragged component's `accepts` list. This needs a fast, read-only surface classification (plane / cylinder / circular edge) on the pick result. The ladder computes descriptors offline; the interactive path needs a cheap variant. **This is the main technical risk — it is the P0 spike (§10).** Expose it as a geometry-query capability on the modeling service (`onecad.modeling.geometry-query`), so library UI never touches the kernel: pick → classify → return `{kind, frame}`.

### 5.3 Attachment inference rules

| Hovered target | Satisfies attachment | Snap |
|---|---|---|
| Cylindrical face / hole bore | `accepts:["cylinder","hole"]` | **Concentric** — axis aligned to cylinder axis, seated at near end |
| Planar face | `accepts:["plane"]` | **Flush / coincident** — mating face coplanar, normal-aligned, at pick point |
| Circular edge (hole rim) | axis + seat | **Concentric + coincident** in one click (the preferred fastener gesture) |
| Hole feature (from Hole op) | + auto-size | Snap concentric **and** set the free size param to the nearest smaller standard size |

### 5.4 The gesture

1. Drag from panel (or double-click) → live `PreviewOp` ghost follows the cursor (the modeling contract's "live result" state).
2. Hover a target → classify, match attachment, preview the snap (axis line for concentric, seating-plane flash for flush, both for a hole rim).
3. **Auto-size on holes**: offer nearest smaller standard size; update the size chip live.
4. **Flip** with `A`; **cycle** alternative valid attachments with `Tab`.
5. Click to commit → one atomic `PlaceComponent` (identity + params + frozen placement + cached geometry + recorded mate pair).
6. Or drop in free space → position later with the Move tool (fallback, never the required path).

### 5.5 Persistent mates (in scope)

Because `PlaceComponent.mate.target` is a full `ElementRef`, regen re-resolves it through the ladder. On regen, for each instance with a mate: resolve target → if `autoBind`, recompute the component's transform from the mate kind + the resolved frame + flip, and re-publish if it moved; if `needsRepair`, flag the instance (display it at its stale cached `placement`) — never drop it, never silently move it. This is the differentiator: a plate that moves re-seats its screws; a plate whose hole was deleted produces a truthful `NeedsRepair`, not a floating part.

Mate solving must respect the project's tolerance discipline: re-seat only when the resolved frame moved beyond a pinned epsilon; a within-plane wiggle that doesn't change the mate is a no-op (mirrors `Hole`'s 1e-3 mm re-projection gate). Keep mates one-directional (component follows target; the target never follows the component) to avoid cycles in v1.

---

## 6. The parametric engine

Proven pattern (BOLTS, FreeCAD): **separate dimension data (tables) from geometry generators (code).** Generators are **built-in and versioned**, run through the existing worker op path — no new execution path, no JS on the kernel path.

### 6.1 Parameter roles

`free` (user-editable per instance) · `table` (derived from the dimension table, keyed by the free size — locked) · `computed` (pure formula). The table key is the **thread designation, not a raw diameter** (an "M6 washer" has a 6.4 mm hole). Length is free but constrained: snap to the ISO 888 preferred series, validate per-size minimums.

### 6.2 Seed catalog

- **Fasteners** (table-driven, BOLTS-seeded): ISO 4762/DIN 912 socket cap, ISO 7380 button head, ISO 4014/4017 hex bolts, ISO 4032 nuts, ISO 7089/7093 washers — M2–M12.
- **Bearings** (pure lookup, ISO 15): only bore/OD/width matter for placement; fully determined by bearing code (608 = 8×22×7). Stepped-ring geometry suffices.
- **Motors** (NEMA 17/23): faceplate square, hole pattern, pilot Ø/height, shaft Ø/length fixed; body length is a free per-instance value.

### 6.3 BOLTS seeding + licensing (hard constraint)

The only mature, legally redistributable dataset is **BOLTS** (LGPL 2.1+, per-part license tracking). McMaster-Carr / GrabCAD / Misumi / SKF / 3D ContentCentral all **prohibit bundling** — never scrape them. Seed ISO fastener dimension tables from BOLTS data with attribution and a per-part license audit; author bearing/motor tables yourself (they're trivial). Ship a `THIRD_PARTY_NOTICES` file. A future TraceParts partnership (the one channel built for third-party CAD apps) or user-credentialed McMaster API are **later** online options — out of scope for this build, but don't close the door.

### 6.4 Threads

Three detail levels via the `thread_detail` param: **cosmetic** (default — cylinder at pitch Ø + designation metadata; assembly-safe, fast), **simplified** (annular grooves), **modeled** (true helical ISO 68-1 profile, opt-in for 3D-print output). Mainstream CAD defaults to cosmetic for measured perf reasons; match that. Modeled threads go through the worker and are a natural kernelbench robustness case.

### 6.5 Table verification

Treat each table as a reviewed data file with a spot-check harness asserting key dimensions per size against a reference source. Generate robustness cases from the table extremes (smallest/largest size per family) through the existing `onecad-kernelbench` harness — that is exactly its job, and it extends the harness past fillet, which the project already wants.

---

## 7. The `onecad.library` UI module

Create a new built-in module mirroring `src/modules/modeling/`'s shape: `manifest.ts` (`moduleId("onecad.library")`, schema version, service ids), `register.ts` (`registerLibraryModule(platform)`), `ids.ts`, plus a feature folder `src/features/library/`. Per ADR-0002 the library consumes modeling only through `onecad.modeling.command-api` / `geometry-query`; per ADR-0003 everything on screen is a slot contribution. Follow the modeling contract from the UX audit: Awaiting input → Live result → Valid/invalid → Commit → Discard; numeric entry routes type-to-primary, every keystroke previews, Enter applies, Esc discards, blur never required.

**Contributions to register:**
- **Library panel** → `Slots.ShellLeft`. Search field + filter chips (category, standard, "mine"); category tree; card grid of thumbnails; each card shows name, live designation preview, and a source badge (parametric / static / mine). Selecting a card opens the configurator.
- **Configurator** → `Slots.ShellOverlay` (or an inspector section). The component's `free` params as a compact dropdown set, a live 3D preview, and the designation string updating in real time.
- **Placement ghost + snap affordances** → `Slots.ViewportLayer` / `Slots.ViewportOverlay` (axis lines, seating flash, size chip). Screen-space, depth-independent, collision-aware with chips — per the shared GizmoOverlay direction in the UX audit.
- **Progress** → `Slots.StatusSection` / the background-tasks chip. Library indexing, package import, and modeled-thread preview generation are the chip's **first real producer** (`begin/setProgress/end`). This pays down a known platform debt.
- **History node** → placed components appear in history as uniquely-named editable nodes ("ISO 4762 M6×20") with parameters and output lineage, via the module's tree provider.
- **Templates on the start screen** → wire the **already-stubbed** `templates` nav key in `src/features/start/StartScreen.tsx` (`StartNavKey` includes `"templates"`, `NAV_EMPTY_HINT.templates` exists). Reuse the `RecentGrid`/`ProjectCard` card anatomy; choosing a template instantiates a copy as a new untitled project (the template itself is immutable).

**Authoring ("Save as Component"):** right-click a body/history subtree → declare parameter roles (SolidWorks Locating/Internal/Size split applied to whole parts) → click faces/edges to place attachments (the 60 seconds of work that makes it snap forever) → name/categorize/thumbnail from a viewport snapshot → writes a `document`-kind package. Honor the single-solid constraint at save (offer union / pick primary / split). Re-saving bumps semver; existing instances keep their recorded revision and offer opt-in upgrade.

---

## 8. Templates

A template is a frozen `.onecad` package stored under `templates/` in the library root — same storage, integrity, and future-registry mechanics as components. Ship 3–5 honest starters: Blank, 3D-printer part (mm, XY-on-bed datum), NEMA motor mount plate, enclosure base. "Save as Template" sits beside "Save as Component." Templates and components compose (a template may contain placed instances that resolve through the normal machinery).

---

## 9. Open constraint — single-solid publication

The publication policy is solid-like at the top level (`single_solid_policy`). A placed component is one solid, so v1 is compatible. A multi-body "subassembly component" (motor + connector) is a compound and is **out of scope for v1** — author multi-body content as separate components or union to one solid. Revisit only when persistent mates give a compound meaning. State this honestly in authoring UI rather than letting users discover it at placement time.

---

## 10. Phased build plan — each phase independently gated

**P0 — Foundation spike (de-risk first, no shipped feature).**
Prove interactive surface classification (plane/cylinder/circular edge) on a hovered pick, read-only, fast enough for live preview (<16 ms per hover classification). Prove one generator end-to-end (hard-coded ISO 4762, M6 only) through the worker op path. **Go/no-go on the placement UX.** If hover classification is too slow, fall back to click-to-classify-then-preview (still far ahead of drop-and-joint). Gate: a measured latency number + one M6 screw placed concentrically on a hole in a dev build.

**P1 — Static library + place-at-target (first shippable value).**
Library root + `component.toml` + `library.json`; `embedded` source kind only; `PlaceComponent` + `DetachComponent`; cached geometry; revision verification → `NeedsRepair` on mismatch; `shell.left` panel with search + drag-to-place; concentric/flush snap + flip; auto-size on holes; tasks chip wired to indexing; seed self-authored static bearings + NEMA motors. Gate: kernel CTest for the ops, Rust tests for storage/resolution, a Playwright spec for browse→place→snap→save→reopen.

**P2 — Parametric fasteners (the differentiator).**
`generator` source kind, general; table/generator split; BOLTS-seeded ISO tables; `SetComponentParams`; parameter roles enforced in the configurator; three-level thread detail (cosmetic default); auto-size sets the free size param; kernelbench cases from table extremes. Gate: table spot-check harness green; a placed screw's size editable in place; kernelbench boolean/fastener cases run.

**P3 — Authoring + templates + persistent mates (the market gap).**
`document` source kind; "Save as Component" (param roles + attachment placement); "Save as Template" + start-screen row; `ReplaceComponent`; opt-in version upgrade; **persistent mate re-seating on regen** with the NeedsRepair-on-vanished-target rule. Gate: e2e for author→place→edit-upstream→mate-reseats, and for template→new-project.

**P4 — (later, separable)** community registry as a remote `library.json` + blob fetcher; TraceParts partnership exploration. Not in this build.

---

## 11. Test matrix (per phase)

- **Kernel (worker CTest):** each new op's geometry (exact volumes where applicable), mate re-seat transforms, generator output across table extremes, modeled-thread robustness. Follow the existing `test_m6a_ops.cpp` pattern.
- **Rust:** package parse/validate, index rebuild, blob dedupe/integrity, revision-mismatch → `NeedsRepair` (never `Err`), param-role enforcement, wire lowering slot agreement (`wire_op_inputs` vs `KnownOperation::inputs` — pinned by the existing slot-order test), PlaceComponent/SetComponentParams/ReplaceComponent/DetachComponent authoring + `validate()`.
- **Playwright e2e:** browse/search, drag-place with snap, auto-size, flip, edit-in-place, detach, author-a-component, template→new-project, and the reopen-with-no-library-dependency case. `retries: 0`.
- **Cross-cutting:** SCHEMA §7.3 docs + §14 changelog + fixture bump for every wire change; the invariants in §0 stay green (run the invariants audit before and after).

---

## 12. Definition of done

A user can: open the library panel, search "M8 socket head," drag it onto a hole, have it snap concentric and auto-size to M8, commit it as one undoable history node, edit its length in place, save, close, reopen with the library folder deleted and still see the screw (flagged `NeedsRepair`, geometry intact), move the plate and watch the screw re-seat, author their own bracket as a snapping component, and start a new project from a template. All gates pass, all invariants hold, and the wire contract is documented and signed off.
