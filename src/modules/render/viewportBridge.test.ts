/*
 * The render → viewport adapter, driven through the store's real write path
 * (same discipline as `materialQuery.test.ts`: the bridge is a VIEW, so seeding
 * the store directly would prove less than assigning the way the UI will).
 *
 * This file also owns the STRUCTURAL type check between the two vocabularies —
 * see the `satisfies` below. It has to live on this side: `src/viewport/**` may
 * not import `src/modules/**`, so the viewport's own tests cannot assert it.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { getMockLatency, mockClient, setMockLatency } from "@/ipc/mockClient";
import type { BodyMaterialSource, PbrMaterialParams } from "@/viewport/engine/pbrParams";
import { getBodyMaterialSource } from "@/viewport/materialSourceBridge";
import type { ModuleScope } from "@/platform";
import {
  createMaterial,
  resolveMaterial,
  resolvedMaterialHash,
  type ResolvedMaterial,
} from "./model/material";
import { createMaterialQueryService } from "./materialQuery";
import { attachRenderViewportBridge, createRenderMaterialSource } from "./viewportBridge";
import { renderStore, setRenderBodyLister, setRenderDocumentStateService } from "./store/renderStore";

const STEEL = createMaterial("Steel", {
  base: { base_metalness: 1, base_color: [0.5, 0.5, 0.5] },
});
const BRASS = createMaterial("Brass", { base: { base_color: [0.8, 0.6, 0.2] } });

const realLatency = getMockLatency();
let source: BodyMaterialSource;

beforeEach(async () => {
  setMockLatency(0);
  setRenderDocumentStateService(null);
  setRenderBodyLister(null);
  renderStore.getState().reset();
  await mockClient.closeDocument();
  await renderStore.getState().hydrate();
  source = createRenderMaterialSource(createMaterialQueryService());
});

afterEach(() => {
  setMockLatency(realLatency);
  renderStore.getState().reset();
});

/** Steel on `body1`, Brass overriding face `el_2`. */
async function seed(): Promise<void> {
  await renderStore.getState().upsertMaterial(STEEL);
  await renderStore.getState().upsertMaterial(BRASS);
  await renderStore.getState().assignBody("body1", STEEL.id);
  await renderStore.getState().assignFace("el_2", BRASS.id);
}

/*
 * THE SEAM. `ResolvedMaterial` must satisfy `PbrMaterialParams` structurally —
 * that is the entire mechanism by which the viewport renders OpenPBR without
 * ever importing this module. If either field set drifts, this stops compiling.
 */
describe("the structural contract", () => {
  it("ResolvedMaterial satisfies PbrMaterialParams", () => {
    const resolved = resolveMaterial(STEEL) satisfies PbrMaterialParams;
    const asParams: PbrMaterialParams = resolved;
    // …and in the other direction the viewport's type is NOT secretly wider.
    const back: ResolvedMaterial = resolved;
    expect(asParams.base_metalness).toBe(back.base_metalness);
  });
});

describe("createRenderMaterialSource", () => {
  it("passes the pool key and resolved params straight through", async () => {
    await seed();
    expect(source.poolKeyForBody("body1")).toBe(`${STEEL.id}:${resolvedMaterialHash(STEEL)}`);
    expect(source.paramsForBody("body1")).toEqual(resolveMaterial(STEEL));
  });

  it("answers null for a body with no assignment", () => {
    expect(source.poolKeyForBody("body9")).toBeNull();
    expect(source.paramsForBody("body9")).toBeNull();
  });

  it("returns the overriding material's base_color for overridden faces only", async () => {
    await seed();
    expect(source.faceOverrideBaseColors("body1", ["el_1", "el_2", "el_3"])).toEqual({
      el_2: resolveMaterial(BRASS).base_color,
    });
  });

  /*
   * A dangling override yields NO entry rather than a fabricated default: the
   * face then draws with the body's material, which is the honest rendering of
   * "this override names a material that isn't in the library".
   */
  it("skips an override naming a material the library does not have", async () => {
    await renderStore.getState().upsertMaterial(BRASS);
    await renderStore.getState().assignFace("el_2", BRASS.id);
    await renderStore.getState().removeMaterial(BRASS.id);

    expect(source.faceOverrideBaseColors("body1", ["el_2"])).toEqual({});
  });

  it("routes the binding report into the store's session-only maps", () => {
    source.reportFaceOverrideBindings("body1", ["el_2"], ["el_7"]);

    expect(renderStore.getState().boundFaceOverrides).toEqual({ body1: ["el_2"] });
    expect(renderStore.getState().unboundFaceOverrides).toEqual({ body1: ["el_7"] });

    // Empty clears the body's entry rather than leaving a stale claim.
    source.reportFaceOverrideBindings("body1", [], []);
    expect(renderStore.getState().boundFaceOverrides).toEqual({});
    expect(renderStore.getState().unboundFaceOverrides).toEqual({});
  });

  it("fires subscribers on a material change, not on a binding report", async () => {
    const cb = vi.fn();
    const unsubscribe = source.subscribe(cb);

    await renderStore.getState().upsertMaterial(STEEL);
    expect(cb).toHaveBeenCalledTimes(1);

    // A report is session evidence, not appearance — re-notifying on it would
    // put the viewport in a resync loop with itself.
    source.reportFaceOverrideBindings("body1", ["el_2"], []);
    expect(cb).toHaveBeenCalledTimes(1);

    unsubscribe();
    await renderStore.getState().upsertMaterial(BRASS);
    expect(cb).toHaveBeenCalledTimes(1);
  });
});

describe("attachRenderViewportBridge", () => {
  it("publishes on the viewport bridge and withdraws on module dispose", () => {
    const disposers: Array<() => void> = [];
    const scope = {
      own: (d: { dispose: () => void }) => {
        disposers.push(() => d.dispose());
        return d;
      },
    } as unknown as ModuleScope;

    attachRenderViewportBridge(scope);
    expect(getBodyMaterialSource()).not.toBeNull();

    for (const d of disposers) d();
    expect(getBodyMaterialSource()).toBeNull();
  });
});
