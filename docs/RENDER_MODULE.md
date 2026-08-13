# Render module (draft — not implemented)

Everything below is a design draft for a module that does not exist yet beyond
an inert registration stub (`src/modules/render/`, see
`docs/adr/0014-render-module-openpbr.md` for the decisions that authorize it).
No section here is a commitment to an API shape, a schema, or a timeline —
treat every "draft" marker as "will be re-verified before anyone writes code
against it."

## 1. Goal

Parity target is Fusion 360's Render workspace: a place to assign
physically-based materials/appearances to bodies and faces, set up an
environment (HDRI/lighting), configure render settings, and produce a render
output that is visually distinct from — and higher quality than — the modeling
viewport's real-time shading.

Material authoring is built on OpenPBR
(https://academysoftwarefoundation.github.io/OpenPBR/), the Academy Software
Foundation's cross-renderer physically-based material specification, rather
than a bespoke shading model invented for this app.

## 2. Non-goals (v1 and unscheduled)

- No bespoke shader language or node-graph shading system.
- No addon-defined materials or render backends — whether OpenPBR authoring
  stays closed like the geometry kernel (ADR-0002) or opens to addons later is
  an **open question** (§6), but it is closed by default until decided.
- No implementation timeline. This document exists so a later implementer does
  not have to re-derive scope; it does not schedule that implementer's work.

## 3. OpenPBR mapping (draft, unverified against implementation)

OpenPBR describes a material as layered, energy-conserving stack rather than a
flat parameter bag. The layers relevant to a first material schema:

- **Base substrate** — diffuse/metallic base color, roughness, an
  IOR-driven specular response, and a metalness blend between dielectric and
  conductor behavior.
- **Specular coat** — a secondary, independently rough specular lobe over the
  base (car paint, coated plastics).
- **Coat** — a clear dielectric layer with its own roughness/thickness/tint,
  distinct from specular coat.
- **Thin-film** — iridescence over the base or coat.
- **Subsurface** — subsurface scattering parameters (radius, color) for
  translucent materials (wax, skin, marble).
- **Transmission** — glass/liquid-style transparency with roughness and IOR.
- **Emission** — self-illuminating materials.
- **Geometry inputs** — normal/bump/displacement and thin-walled flags.

Draft direction: the module's material record mirrors this layering directly
(one optional sub-object per layer) rather than flattening every parameter
into one struct, so a material that only uses the base layer stays small on
disk and a future OpenPBR spec revision that adds a layer is additive, not a
schema break. Exact field names/defaults are **not pinned here** — they need to
be taken from whatever OpenPBR spec revision is current when this is actually
implemented, not from this draft's summary of it.

## 4. Document state shape (draft, unimplemented)

Sketch of what would live under `onecad.render`'s module-state slice (the
`src/platform/documentState.ts` seam, `ModuleId`-scoped, opaque-payload,
verbatim-preserved per ADR-0005):

- **Material library** — named material definitions (the OpenPBR-layered
  record from §3), keyed by a render-module-owned material id.
- **Assignments** — a map from a Modeling-owned `ElementId` (body or face) to
  a material id. This is a cross-module reference: the render module reads
  Modeling's element identity through Modeling's public service surface only
  (`onecad.modeling.geometry-query` or an equivalent read-only service),
  never by reaching into Modeling's internals — the architecture law in
  `docs/ARCHITECTURE.md` §3/§6 applies here same as anywhere else.
- **Environment** — HDRI reference (resource-store id, not inline binary data
  — ADR-0004 keeps large binaries out of `document.json`), plus ground/sky
  and exposure settings.
- **Render settings** — resolution, sample count, output format — whatever a
  chosen render backend (§5) needs, kept backend-agnostic where possible so a
  future backend swap does not force a document migration.

`RENDER_SCHEMA_VERSION` (`src/modules/render/manifest.ts`) is reserved at `1`
for this future shape; nothing reads or writes it yet.

## 5. Render backend — open question, not decided

The single biggest unresolved architectural question. Options, not a choice:

- **(a) Real-time PBR preview only** — reuse the existing Three.js viewport
  materials/pipeline for an interactive approximation, no offline pass. Cheapest
  to build, weakest output quality, no new backend process.
- **(b) Offline path-traced render** — either a new responsibility added to the
  C++ worker sidecar, or an entirely separate render process/service. OCCT does
  not provide a path tracer; this would be new integration work, not a kernel
  extension.
- **(c) Something else** — e.g. shelling out to an existing open-source
  renderer. Not evaluated.

This document does not recommend one. Whoever picks this up should treat it as
a real design decision worth its own ADR, not an implementation detail to
default into.

## 6. Phased roadmap (draft, no dates)

- **Phase 0 (this work).** `onecad.render` module scaffolded and registered,
  reaches `"ready"` with zero contributions. This document and ADR-0014 record
  the scope and the decisions already made.
- **Phase 1.** Material data model (§3/§4) + assignment UI. Viewport-only PBR
  preview (§5a) for interactive feedback — no offline render yet.
- **Phase 2.** Environment/lighting (HDRI, exposure).
- **Phase 3.** Render backend decision (§5) + implementation, render settings
  UI, render output.

## 7. Open questions

- Render backend approach (§5) — real-time preview vs. offline path tracer vs.
  something else.
- When/how the shell's `Visualization` workspace placeholder gets formally
  retargeted to `onecad.render`'s own namespace, or handed off some other way
  (ADR-0014 §2 defers this).
- Which OpenPBR spec revision to pin the schema against, and how schema
  migrations track future spec revisions.
- Whether OpenPBR material authoring should ever be addon-extensible, or stays
  closed like the geometry kernel (ADR-0002's precedent) — closed by default
  until decided (§2).
