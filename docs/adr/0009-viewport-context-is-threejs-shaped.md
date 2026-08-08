# ADR-0009 — The viewport contract is Three.js-shaped in v1

Status: accepted (2026-08-08)

## Context

`ViewportContribution` shipped with a `ViewportContext` carrying one method,
`invalidate()`. That was enough to declare the contract and nothing else: it had
zero producers, and the first real producer showed why. Porting `DatumLayer` onto
it needs a scene group to attach to, a per-frame tick to hold screen-constant
size, a theme hook, a raycaster, and a projected HTML label. None of those can be
expressed without naming what a scene object IS.

Three options were on the table:

1. Keep the context abstract (`root: unknown`) and cast on the module side. The
   coupling still exists, it is just unchecked — and every contribution
   re-invents the same cast.
2. Invent a renderer-neutral scene abstraction the host implements over Three.js.
   Real work, and speculative: there is no second renderer, and the abstraction
   would be designed against a single implementation.
3. Admit the coupling and type it.

## Decision

`src/platform/contributions.ts` imports `Camera`, `Object3D` and `Raycaster`
from `three` as **types only**, and `ViewportContext` exposes:

- `root: Object3D` — a host-owned group
- `createLabel(el): ViewportLabelHandle` — projected HTML, host-positioned
- `onFrame(cb)` / `onThemeChange(cb)` — host-driven, never a contribution's own loop
- `raycastFromClient(x, y)` — the host owns the projection maths
- `registerSecondaryHover(fn)` — a hover token for the host's pick pass

The architecture rule this is measured against forbids `@/platform` importing
`@/features`, `@/tools`, `@/modules`, `@/stores`, `@/viewport`, `@/app`
(`architecture.test.ts`). `three` is none of those: it is a library the whole
frontend already depends on, and `slots.ts` has described `viewport.layer` as
"in-scene contributions (Three.js objects)" since the slot list was written.

`createLabel` is a façade rather than a handle to `HtmlOverlayDriver`. Handing
out the driver would put a renderer internal into the public surface, and would
let a contribution reposition labels it does not own.

## Consequences

- A second renderer would be a breaking change to this contract. That is the
  honest position: v1 has one, the host owns it (ARCHITECTURE §7), and pretending
  otherwise would cost design effort now to protect a decision nobody has made.
- Addons see Three.js types. They already would have, through the objects they
  add to `root`.
- The silent-failure invariant in `viewport/engine/README.md` now extends to
  contributions: a layer that reads palette colors and does not subscribe to
  `onThemeChange` goes stale with no error. `onThemeChange` is documented as
  mandatory for such layers, and the datum port is covered by a negative-checked
  theme test.
