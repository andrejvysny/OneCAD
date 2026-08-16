/*
 * `MaterialQuery`'s contract, driven through the store's real write path — the
 * service is a VIEW, so a test that seeded the store directly would prove less
 * than one that assigns the way the UI will.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { getMockLatency, mockClient, setMockLatency } from "@/ipc/mockClient";
import { createMaterial, resolveMaterial, resolvedMaterialHash } from "./model/material";
import { createMaterialQueryService } from "./materialQuery";
import type { MaterialQueryService } from "./manifest";
import { renderStore, setRenderBodyLister, setRenderDocumentStateService } from "./store/renderStore";

const STEEL = createMaterial("Steel", { base: { base_metalness: 1, base_color: [0.5, 0.5, 0.5] } });
const BRASS = createMaterial("Brass", { base: { base_color: [0.8, 0.6, 0.2] } });

const realLatency = getMockLatency();
let query: MaterialQueryService;

beforeEach(async () => {
  setMockLatency(0);
  setRenderDocumentStateService(null);
  setRenderBodyLister(null);
  renderStore.getState().reset();
  await mockClient.closeDocument();
  await renderStore.getState().hydrate();
  query = createMaterialQueryService();
});

afterEach(() => {
  setMockLatency(realLatency);
  renderStore.getState().reset();
});

/** Steel on `body1`, Brass overriding `body1`'s face `el_2`. */
async function seedAssignments(): Promise<void> {
  await renderStore.getState().upsertMaterial(STEEL);
  await renderStore.getState().upsertMaterial(BRASS);
  await renderStore.getState().assignBody("body1", STEEL.id);
  await renderStore.getState().assignFace("el_2", BRASS.id);
}

describe("library reads", () => {
  it("lists the library and resolves ids", async () => {
    await renderStore.getState().upsertMaterial(STEEL);
    await renderStore.getState().upsertMaterial(BRASS);

    expect(query.listMaterials()).toEqual([STEEL, BRASS]);
    expect(query.getMaterial(STEEL.id)).toEqual(STEEL);
    expect(query.getMaterial("mat_00000000-0000-4000-8000-000000000000")).toBeNull();
  });
});

describe("assignment reads", () => {
  it("materialForFace prefers the face override and falls back to the body", async () => {
    await seedAssignments();

    expect(query.materialForFace("body1", "el_2")).toEqual(BRASS);
    expect(query.materialForFace("body1", "el_1")).toEqual(STEEL);
    expect(query.materialForFace("body2", "el_1")).toBeNull();
  });

  it("materialIdForBody ignores face overrides entirely", async () => {
    await seedAssignments();
    expect(query.materialIdForBody("body1")).toBe(STEEL.id);
    expect(query.materialIdForBody("body2")).toBeNull();
  });

  it("faceOverridesForBody reports only the ids that are overridden", async () => {
    await seedAssignments();
    expect(query.faceOverridesForBody("body1", ["el_1", "el_2", "el_3"])).toEqual({
      el_2: BRASS.id,
    });
    expect(query.faceOverridesForBody("body1", [])).toEqual({});
  });

  it("resolves an id that names nothing to null rather than to a default", async () => {
    await renderStore.getState().assignBody("body1", STEEL.id); // never upserted
    expect(query.materialForFace("body1", "el_1")).toBeNull();
    expect(query.resolvedForBody("body1")).toBeNull();
    expect(query.poolKeyForBody("body1")).toBeNull();
  });
});

describe("resolution + pool key", () => {
  it("resolvedForBody fills every OpenPBR parameter", async () => {
    await seedAssignments();
    expect(query.resolvedForBody("body1")).toEqual(resolveMaterial(STEEL));
    expect(query.resolvedForBody("body2")).toBeNull();
  });

  it("poolKeyForBody is stable across reads and moves with the material", async () => {
    await seedAssignments();
    const key = query.poolKeyForBody("body1");

    expect(key).toBe(`${STEEL.id}:${resolvedMaterialHash(STEEL)}`);
    expect(query.poolKeyForBody("body1")).toBe(key);

    // Editing the material must move the key — that is what makes it usable as a
    // pool cache key at all.
    await renderStore.getState().upsertMaterial({ ...STEEL, base: { base_metalness: 0 } });
    expect(query.poolKeyForBody("body1")).not.toBe(key);
  });
});

describe("usageCounts", () => {
  it("counts body and face assignments together", async () => {
    await seedAssignments();
    await renderStore.getState().assignBody("body2", STEEL.id);
    await renderStore.getState().assignFace("el_9", STEEL.id);

    expect(query.usageCounts()).toEqual({ [STEEL.id]: 3, [BRASS.id]: 1 });
  });

  it("omits a material nothing wears", async () => {
    await renderStore.getState().upsertMaterial(STEEL);
    expect(query.usageCounts()).toEqual({});
  });
});

describe("subscribe", () => {
  it("fires on a material or assignment change and stops on unsubscribe", async () => {
    let fired = 0;
    const stop = query.subscribe(() => {
      fired += 1;
    });

    await renderStore.getState().upsertMaterial(STEEL);
    expect(fired).toBe(1);
    await renderStore.getState().assignBody("body1", STEEL.id);
    expect(fired).toBe(2);

    // Session-only state is not a material change — a subscriber that repainted
    // for it would repaint on every viewport bind report.
    renderStore.getState().reportUnboundFaceOverrides("body1", ["el_4"]);
    expect(fired).toBe(2);

    stop();
    await renderStore.getState().assignBody("body1", null);
    expect(fired).toBe(2);
  });
});
