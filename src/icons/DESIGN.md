> **Mirrored from `OneCAD-CPP/resources/icons/DESIGN.md`.** The masters live there and
> are the source of truth for geometry; this copy is here so the spec is available when
> authoring the handful of glyphs CPP has no counterpart for (`src/icons/authored.ts`).
> The rules below hold verbatim — canvas, stroke weights, projection, metaphor grammar
> and constraint forms are all unchanged. Only the *plumbing* differs on this side:
>
> | §  | Qt / CPP                                          | React / Tauri                                                    |
> |----|---------------------------------------------------|------------------------------------------------------------------|
> | 3  | `IconLoader` string-replaces the `#2D7FF9` literal | `tone: "accent"` renders `var(--color-icon-accent)`; no literal reaches `src/` (hex gate) |
> | 3  | Monochrome / two-tone user preset (`icons/twoTone`) | Two-tone only. Containers that already paint the glyph accent-colored (an active `ToolButton`) collapse to mono locally by pointing `--color-icon-accent` at `currentColor` — so §3's "must still read when collapsed" is a live requirement, not a spare preset |
> | 2  | Absolute `stroke-width` per element               | Stored as a RATIO of the icon's base width, scaled by each call site's `strokeWidth` |
> | 7  | `.svg` files registered in `resources.qrc`        | `scripts/gen-icons.mjs` compiles them into `cppIcons.generated.ts`; nothing ships as an asset |
>
> Render sizes differ too: the Qt app drew at 28px (`SidebarToolButton`) and 18px, the
> web app at 11-20px. §1's "must stay legible at both" is therefore *tighter* here —
> check any new glyph at 11px, not just in the `?gallery` grid at 20px.

---

# OneCAD Icon Design System

Purpose-built icon family for a parametric CAD app. Replaces the ad-hoc Feather/Lucide
glyphs that lacked a shared visual language. Every icon in `resources/icons/*.svg` follows
this spec so the set reads as one system.

Research basis: KiCad icon guidelines (grid / stroke / flat / monochrome), Fusion·Onshape·
SolidWorks feature grouping (create / modify / duplicate), standard geometric-constraint
symbol set.

---

## 1. Canvas & geometry

- **viewBox** `0 0 24 24`, `width/height="24"`. (Rendered at 28px by `SidebarToolButton`,
  18px by the tint helpers — must stay legible at both.)
- **Safe padding** ~2px on all sides → **20×20 live area** (keep meaningful marks in `2..22`).
- **Optical weight** matters more than exact bounds — center mass, don't just center bbox.

## 2. Stroke & fill

- **Primary structural lines: `stroke-width="2"`** (was 1.5 — too thin at 28px).
- **Secondary / detail lines: `stroke-width="1.5"`**.
- `stroke-linecap="round"`, `stroke-linejoin="round"` everywhere.
- `fill="none"` for line icons; solid `fill` only for the *result solid* face or filled nodes.
- Do NOT rely on `stroke-width < 1.5` for anything load-bearing.

## 3. Color tokens (drives the two presets)

Two selectable presets share ONE master SVG. The loader (`IconLoader`) maps tokens per preset.

| Token in SVG | Role | Monochrome preset | Two-tone preset |
|---|---|---|---|
| `currentColor` | PRIMARY — profile, geometry, structure | → `ButtonText` | → `ButtonText` |
| `#2D7FF9` | ACCENT — the *operation verb* (arrow / axis / motion / cut) | → `ButtonText` | → theme `iconAccent` |
| `#FFFFFF` | legacy white fill | → `ButtonText` | → `ButtonText` |

Rules:
- Author every icon as a **two-tone master**: primary = `currentColor`, accent element(s) =
  literal `#2D7FF9`. The monochrome preset is derived by collapsing accent → primary.
- **Exactly one idea gets the accent** — the thing the action *does* (extrude arrow, revolve
  axis, cut region, mirror axis, constraint relation). Never accent the whole icon.
- Accent must still make sense when collapsed to one color (i.e. shape carries meaning; color
  only reinforces it).

## 4. 3D projection convention (shared across all solid icons)

One fixed dimetric so extrude / revolve / shell / boolean / fillet-solid / patterns look like
the same object family.

- **Depth vector `d = (+4, −3)`** grid units (back copy = front shifted up-right by d).
- Draw only visible edges: front face, top face, right face. Omit hidden back edges.
- Reference cube (front-bottom-left at `(6,20)`, w=8, h=9):
  - front face `6,11 · 14,11 · 14,20 · 6,20`
  - top face `6,11 · 14,11 · 18,8 · 10,8`
  - right face `14,11 · 18,8 · 18,17 · 14,20`
- 2D sketch tools + constraints are **flat / orthographic** (no dimetric).

## 5. Metaphor grammar (reused verbs)

`thin profile  →  accent operation verb  →  emphasized result`

- **profile** = thin `currentColor` outline (the sketch input)
- **verb** = the `#2D7FF9` accent (arrow, axis, path, cut, reflection)
- **result** = the solid (filled or heavier `currentColor`)

Keep the same verb form across the family: a *pull arrow* means extrude/pushpull; a *dashed
axis + curved arrow* means revolve/rotate/circular-pattern; *ghost copies* mean duplicate.

## 6. Groups (Fusion/Onshape convention)

create · modify · duplicate · sketch-geometry · sketch-edit · constraint · view/nav ·
tree/entity · generic-UI.

## 7. Files & naming

- Existing icons keep their filename + qrc alias (`ic_extrude.svg`, …) — no call-site churn.
- New constraint icons: **`ic_constraint_<name>.svg`** (e.g. `ic_constraint_horizontal.svg`),
  registered in `resources/resources.qrc` under prefix `/icons`.
- SVG must be minimal: no `<?xml?>` prolog needed, no editor cruft, no inline `style=`
  blocks — plain elements with `stroke`/`fill` attributes (the loader does string replace).

## 8. Constraint symbol forms (flat, 20×20 live)

horizontal ⊤-bar · vertical ⊢-bar · parallel `∥` · perpendicular `⊥` · tangent (line kissing
circle) · concentric (two rings) · coincident (two dots merging) · equal `=` · midpoint (tick
at segment center) · symmetric (mirror axis + 2 marks) · point-on-curve (dot on arc) ·
distance (⟷ + ext lines) · h-distance · v-distance · angle `∠` + arc · radius (R + arrow to
arc) · diameter `⌀` · lock/fix (padlock). Accent = the *relation* mark (the axis, the arrow,
the equality ticks); geometry it acts on stays `currentColor`.
