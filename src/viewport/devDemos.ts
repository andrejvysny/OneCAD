/*
 * Demo-only URL-flag handlers (`?vpdemo` / `?sketchdemo` / `?toolsdemo`),
 * split out of ViewportRoot.tsx so its mock-kernel import doesn't ride along
 * into production. ViewportRoot only reaches this module through
 * `void import("./devDemos")` gated on `import.meta.env.DEV` — that check is
 * statically false in a production build, so Rollup dead-code-eliminates the
 * dynamic import call entirely and this module (and the mock kernel it pulls
 * in transitively) never enters the prod bundle. dev and e2e (both DEV
 * builds) load it exactly as ViewportRoot used to inline it.
 */
import {
  emitMockDocumentChanged,
  mockMeshKey,
  resetMockSketches,
  resetMockDocument,
  seedMockDemoCylinder,
  setMockLatency,
} from "@/ipc/mockClient";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity } from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { selectionStore, sketchRegionRef, type EntityRef } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import type { ModelToolController } from "@/tools/modelTools/ModelToolController";
import type { ViewportEngine } from "./engine/ViewportEngine";

export interface DemoFlagsDeps {
  engine: ViewportEngine;
  client: CadClient;
  container: HTMLElement;
  modelToolController: ModelToolController;
  vpdemo: boolean;
  /** `?vpdemo=cyl` — publish the bored bushing beside the box (see below). */
  vpdemoCylinder: boolean;
  sketchdemo: boolean;
  toolsdemo: boolean;
}

/**
 * Run whichever demo blocks their URL flag requested. Bodies moved verbatim
 * out of ViewportRoot.tsx's engine-init effect — only the flag checks changed,
 * from `hasFlag(...)` calls to booleans threaded in by the caller (`hasFlag`
 * itself stays private to ViewportRoot).
 */
export function runDemoFlags(deps: DemoFlagsDeps): void {
  const {
    engine,
    client,
    container,
    modelToolController,
    vpdemo,
    vpdemoCylinder,
    sketchdemo,
    toolsdemo,
  } = deps;

  // ?vpdemo — drive the mock box through the FULL onDocumentChanged path.
  // `?vpdemo=cyl` adds a bushing with a real Ø8.5 bore beside it, so the mock
  // lane has a CYLINDRICAL face to hover (the box has none) — the input the
  // placement gesture's concentric snap and auto-size actually run on.
  if (vpdemo) {
    const changedBodies = [{ bodyId: "body1", meshKey: mockMeshKey("body1", "coarse") }];
    if (vpdemoCylinder) changedBodies.push(seedMockDemoCylinder());
    emitMockDocumentChanged({ revision: 1, changedBodies, removedBodies: [] });
  }

  // ?sketchdemo — enter sketch mode on the seeded XY sketch (no backend;
  // the mock enterSketch does it). Pass an explicit id (skip the plane
  // picker) so the demo is deterministic. Proves the sketch UX end to end.
  if (sketchdemo) {
    resetMockSketches();
    toolStore.getState().setMode("sketch", "sketch2");
  }

  // ?toolsdemo — seed a finished rectangle sketch + a window harness so the
  // 60fps gate can arm/drag/commit extrude and read frame timing.
  if (toolsdemo) {
    resetMockSketches();
    resetMockDocument();
    const sid = "toolsketch";
    let regionRef: EntityRef | null = null;
    const rect: SketchEntity[] = [
      { id: "e1", type: "Line", p0: [-30, -20], p1: [30, -20] },
      { id: "e2", type: "Line", p0: [30, -20], p1: [30, 20] },
      { id: "e3", type: "Line", p0: [30, 20], p1: [-30, 20] },
      { id: "e4", type: "Line", p0: [-30, 20], p1: [-30, -20] },
    ];
    void (async () => {
      await client.enterSketch({ newOnPlane: "XY", sketchId: sid });
      await client.sketchUpsert(sid, rect, []);
      const finish = await client.finishSketch(sid);
      if (finish.regions[0]) {
        regionRef = sketchRegionRef(sid, finish.regions[0].regionId);
      }
      documentStore.getState().addSketch({
        id: sid,
        name: "Sketch T",
        visible: true,
        dof: 0,
        status: "ok",
        geometryToken: `mock:${sid}:v1`,
      });
    })();
    (window as unknown as { __toolsGate?: unknown }).__toolsGate = {
      setLatency: (ms: number) => setMockLatency(ms),
      engine,
      controller: modelToolController,
      container,
      client,
      stores: { selectionStore, toolStore, documentStore },
      arm: () => {
        if (!regionRef) return;
        selectionStore.getState().set([regionRef]);
        toolStore.getState().setTool("extrude");
      },
    };
  }
}
