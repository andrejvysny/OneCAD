/*
 * Datum planes as a `ViewportContribution` — the first real producer of that
 * contract, and the reason the contract has the shape it does.
 *
 * These used to be six methods on `ViewportEngine` (`syncDatums`,
 * `datumHitTest`, `setDatumHover`, `setDatumSelected`, `setDatumGhost`,
 * `isDatumGhostVisible`) plus a lazily-constructed layer field. That put a
 * MODELING concept (a datum is a modeling document record) inside the host
 * renderer, which is exactly the coupling the platform refactor exists to
 * remove: a viewport that knows what a datum is cannot host a module that has
 * none.
 *
 * The layer itself did not need rewriting — it needed a host contract wide
 * enough to build it: a scene root, a per-frame tick (the quads ARE the hit
 * geometry and hold a constant screen size), a theme hook, a raycaster, and a
 * projected label.
 *
 * WHY A MODULE SINGLETON. `SketchController` and `ModelToolController` are
 * imperative objects created outside React, and `datumSync` is a store
 * subscription; none of them can reach a platform service through a hook. The
 * contribution publishes itself here on attach, following the `engineBridge`
 * precedent (its own module doc explains the same reasoning). The platform
 * SERVICE is registered too, so a future non-modeling consumer has the seam —
 * modeling's own internals just take the shorter path.
 */
import type { ViewportContext, ViewportContribution } from "@/platform";
import { DatumLayer, datumGhostPlane, type DatumVisual } from "@/viewport/engine/DatumLayer";
import type { PickablePlane } from "@/viewport/engine/PlanePicker";
import { ModelingViewport } from "./panelIds";

/** What the rest of modeling needs from the datum layer, without owning it. */
export interface DatumVisuals {
  sync(metas: readonly DatumVisual[]): void;
  hitTest(clientX: number, clientY: number): string | null;
  setHover(id: string | null): void;
  setSelected(ids: readonly string[]): void;
  setGhost(baseKind: PickablePlane | null, offset: number, label?: string): void;
  readonly ghostVisible: boolean;
}

let current: DatumVisuals | null = null;

/** The live datum visuals, or null before the viewport attaches them. */
export function getDatumVisuals(): DatumVisuals | null {
  return current;
}

/**
 * White-box surface for the e2e specs, DEV builds only. The datum probes used
 * to reach `window.__vpEngine.datumHitTest` / `.isDatumGhostVisible`; those
 * methods are gone from the host, so the probe follows the mechanism to where
 * the capability actually lives.
 */
function publishDebug(visuals: DatumVisuals | null): void {
  if (!import.meta.env.DEV) return;
  (window as unknown as { __datumVisuals?: DatumVisuals | null }).__datumVisuals = visuals;
}

export function createDatumViewportContribution(): ViewportContribution {
  return {
    id: ModelingViewport.Datums,
    priority: 100,
    attach(ctx: ViewportContext) {
      const layer = new DatumLayer({
        root: ctx.root,
        createLabel: ctx.createLabel,
        invalidate: ctx.invalidate,
      });

      // Screen-constant sizing, at the host's ladder position. The quads are the
      // raycast geometry, so this is correctness, not polish.
      let camera: Parameters<Parameters<ViewportContext["onFrame"]>[0]>[0] | null = null;
      let height = 0;
      const frame = ctx.onFrame((cam, h) => {
        camera = cam;
        height = h;
        layer.update(cam, h);
      });

      // MANDATORY for any layer that reads palette colors: the host cannot tell
      // that a contribution forgot to subscribe, so a missed hook is a silent
      // staleness bug (viewport/engine/README.md § Theming).
      const theme = ctx.onThemeChange(() => layer.refreshColors());

      const hover = ctx.registerSecondaryHover((x, y) => {
        const id = api.hitTest(x, y);
        return id ? `datum:${id}` : null;
      });

      /** Re-size immediately rather than next frame: a hit-test can raycast
       *  before the render loop runs, and the quads ARE the hit geometry. */
      const sizeNow = () => {
        if (camera) layer.update(camera, height);
      };

      const api: DatumVisuals = {
        sync(metas) {
          layer.syncDatums(metas);
          sizeNow();
        },
        hitTest(x, y) {
          const raycaster = ctx.raycastFromClient(x, y);
          return raycaster ? layer.hitTest(raycaster) : null;
        },
        setHover: (id) => layer.setHover(id),
        setSelected: (ids) => layer.setSelected(ids),
        setGhost(baseKind, offset, label) {
          if (baseKind === null) {
            layer.setGhost(null);
            return;
          }
          layer.setGhost(datumGhostPlane(baseKind, offset), label);
          sizeNow();
        },
        get ghostVisible() {
          return layer.ghostVisible;
        },
      };
      current = api;
      publishDebug(api);

      return {
        dispose() {
          if (current === api) {
            current = null;
            publishDebug(null);
          }
          hover.dispose();
          theme.dispose();
          frame.dispose();
          layer.dispose();
        },
      };
    },
  };
}
