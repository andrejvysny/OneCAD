# 0014 — Render is its own module, targets the existing Visualization placeholder, and adopts OpenPBR as its material schema baseline

- Status: Accepted
- Date: 2026-08-13

## Context

The product wants a Fusion 360-style Render workspace: material/appearance
authoring, environment/lighting setup, render settings, and a render output
distinct from the modeling viewport's real-time shading. Nothing is built yet —
this ADR only fixes the decisions a later implementer would otherwise have to
re-derive, per ADR-0001's Module/Workspace/Addon split and this repo's stated
reason for ADRs existing at all: a coding agent infers architecture from folder
names unless the law is written down.

Two structural questions had to be settled before even a stub module could be
registered: what module owns render/material state, and what workspace it
appears in. The shell already ships a `Visualization` workspace placeholder
(`src/modules/shell/workspaces.ts`, `ShellWorkspaces.Visualization`) that shows
a "no tools installed" banner — `workspaceIds.ts` already documents the intended
transition: "when a real module lands it registers its own workspace under its
own namespace and this one goes away." The product owner confirmed this
placeholder *is* the future Render workspace, just currently labeled
"Visualization" in code, and asked that the stub's docs target it as-is rather
than mint a second placeholder or rename anything now.

A material model also has to come from somewhere. Inventing a bespoke shading
parameter set would mean re-deriving physically-based rendering semantics
(layering, energy conservation, IOR handling) that a maintained open
specification already solves. OpenPBR
(https://academysoftwarefoundation.github.io/OpenPBR/) is the Academy Software
Foundation's cross-industry PBR material standard, is what the product asked
for by name, and gives a stable target to schema-version against instead of an
implementation detail borrowed from one renderer.

## Decision

1. **Render is a separate module, `onecad.render`, not folded into Modeling.**
   Material/render state is a distinct concern from geometry/topology — keeping
   it separate lets it be built, versioned, and (per ADR-0001) disabled
   independently of the modeling kernel. It is a normal, non-privileged module:
   nothing in this decision touches ADR-0002's kernel closure.

2. **The module's future UI lives in the existing `onecad.shell` `Visualization`
   workspace placeholder.** No new workspace id is registered by this ADR or by
   the stub it authorizes. When `onecad.render` gains real capability, a
   follow-up decision governs whether it registers its own
   `onecad.render.workspace.*` and the shell placeholder retires (the
   transition `workspaceIds.ts` already describes for Simulation/Drawing), or
   whether the shell hands the existing workspace off some other way.
   Concretely: `docs/RENDER_MODULE.md` documents this as an open question, not
   as settled.

3. **OpenPBR is the schema baseline for the module's future material data
   model**, not a bespoke shading model. `docs/RENDER_MODULE.md` sketches the
   mapping; it is explicitly draft and will need re-verification against
   whatever OpenPBR revision is current when implementation actually starts.

4. **This ADR authorizes only an inert registration stub for now**
   (`src/modules/render/`, wired into `src/app/bootstrap.ts`): the module
   reaches `"ready"` with zero contributions. No panel, tool, command,
   workspace, or document-state read/write exists yet.

## Cost

OpenPBR is a large, still-evolving specification (some layers are pre-1.0) —
tying the schema to it means the module's own `RENDER_SCHEMA_VERSION`
(`src/modules/render/manifest.ts`) has to be tracked deliberately against spec
revisions rather than frozen once and forgotten, the same discipline ADR-0004
already requires of module state in general.

Targeting the shell's placeholder instead of minting `onecad.render`'s own
workspace now means the eventual migration (retarget vs. hand off) is deferred
work, not solved work — a later ADR still has to pick one, and until then the
Visualization workspace stays owned by `onecad.shell`, not `onecad.render`,
even though the product treats them as the same thing.
