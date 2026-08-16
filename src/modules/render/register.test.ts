/*
 * What `onecad.render` actually contributes, and the ordering it depends on.
 *
 * The module registers ONE service and no UI (its workspace home is still the
 * shell's `Visualization` placeholder), so the interesting assertions are about
 * the seams: modeling activates first because render reads its GeometryQuery at
 * activation, the store hydrates from the document-state lane, and a backend
 * document-changed re-hydrates it — which is how an undone material write gets
 * back on screen.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createPlatform, type Platform } from "@/platform";
import {
  emitMockDocumentChanged,
  getMockLatency,
  mockClient,
  setMockLatency,
} from "@/ipc/mockClient";
import { registerModelingModule } from "@/modules/modeling/register";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";
import { settleUntil } from "@/test/settle";
import { registerRenderModule } from "./module";
import { RENDER_MODULE_ID, RenderServices, type MaterialQueryService } from "./manifest";
import { createMaterial } from "./model/material";
import { CURRENT_RENDER_SCHEMA_VERSION } from "./model/state";
import { renderStore } from "./store/renderStore";

vi.mock("@/tools/activateTool", () => ({ activateTool: vi.fn(async () => {}) }));
vi.mock("@/shortcuts/useShortcuts", () => ({ runAction: vi.fn() }));

const GOLD = createMaterial("Gold");

/** Both modules, in the order `app/bootstrap.ts` registers them. */
function bootPlatform(): Platform {
  const platform = createPlatform();
  registerModelingModule(platform);
  registerRenderModule(platform);
  platform.initializeSync();
  return platform;
}

const realLatency = getMockLatency();

beforeEach(async () => {
  // The simulated backend latency is here to make loading states real, which
  // this file does not test; turn-counted settling should not have to outrun it.
  setMockLatency(0);
  renderStore.getState().reset();
  await mockClient.closeDocument();
});

afterEach(() => {
  setMockLatency(realLatency);
  renderStore.getState().reset();
});

describe("render module registration", () => {
  it("reaches ready and publishes MaterialQuery", () => {
    const platform = bootPlatform();

    expect(platform.moduleState(RENDER_MODULE_ID)).toBe("ready");
    const svc = platform.services.require<MaterialQueryService>(RenderServices.MaterialQuery);
    expect(typeof svc.listMaterials).toBe("function");
    expect(typeof svc.poolKeyForBody).toBe("function");
    expect(platform.services.ownerOf(RenderServices.MaterialQuery)).toBe(RENDER_MODULE_ID);
  });

  it("contributes no UI — its workspace home is still the shell placeholder", () => {
    const platform = bootPlatform();
    // Owner-scoped: modeling is booted alongside (render depends on it), so a
    // bare `size` would only be asserting that modeling registered nothing.
    const ownedBy = (registry: { registrations(): readonly { owner: string }[] }) =>
      registry.registrations().filter((r) => r.owner === RENDER_MODULE_ID);
    expect(ownedBy(platform.commands)).toEqual([]);
    expect(ownedBy(platform.tools)).toEqual([]);
    expect(ownedBy(platform.panels)).toEqual([]);
    expect(ownedBy(platform.workspaces)).toEqual([]);
    expect(ownedBy(platform.inspector)).toEqual([]);
  });

  it("activates AFTER modeling, whose GeometryQuery it reads at activation", () => {
    const platform = createPlatform();
    // Registered in the WRONG order on purpose: `dependsOn` has to be what
    // decides this, not the order `bootstrap.ts` happens to use.
    registerRenderModule(platform);
    registerModelingModule(platform);
    platform.initializeSync();

    expect(platform.moduleState(MODELING_MODULE_ID)).toBe("ready");
    expect(platform.moduleState(RENDER_MODULE_ID)).toBe("ready");
  });

  it("refuses to boot without modeling rather than activating half-wired", () => {
    const platform = createPlatform();
    registerRenderModule(platform);
    expect(() => platform.initializeSync()).toThrow(/depends on "onecad.modeling"/);
  });

  it("disposing the module unregisters its service", () => {
    const platform = bootPlatform();
    platform.scopeFor(RENDER_MODULE_ID).dispose();
    expect(platform.services.has(RenderServices.MaterialQuery)).toBe(false);
  });
});

describe("document-state wiring", () => {
  it("hydrates the store from the document's slice at activation", async () => {
    await mockClient.setModuleState(RENDER_MODULE_ID, {
      schemaVersion: CURRENT_RENDER_SCHEMA_VERSION,
      payload: {
        schemaVersion: 1,
        openPbrRevision: "1.1.1",
        library: { [GOLD.id]: GOLD },
        assignments: { bodies: { body1: GOLD.id }, faces: {} },
      },
    });

    const platform = bootPlatform();
    const svc = platform.services.require<MaterialQueryService>(RenderServices.MaterialQuery);

    await settleUntil(() => expect(svc.listMaterials()).toEqual([GOLD]));
    expect(svc.materialIdForBody("body1")).toBe(GOLD.id);
  });

  it("re-hydrates on document-changed, so an undone write comes back on screen", async () => {
    const platform = bootPlatform();
    await settleUntil(() => expect(renderStore.getState().hydrated).toBe(true));
    expect(renderStore.getState().state.library).toEqual({});

    // Someone else changed the document (the undo path is the real case — see
    // "backend undo" in `store/renderStore.test.ts`).
    await mockClient.setModuleState(RENDER_MODULE_ID, {
      schemaVersion: CURRENT_RENDER_SCHEMA_VERSION,
      payload: {
        schemaVersion: 1,
        openPbrRevision: "1.1.1",
        library: { [GOLD.id]: GOLD },
        assignments: { bodies: {}, faces: {} },
      },
    });
    emitMockDocumentChanged({ revision: 99, changedBodies: [], removedBodies: [] });

    await settleUntil(() =>
      expect(renderStore.getState().state.library).toEqual({ [GOLD.id]: GOLD }),
    );

    // …and the subscription belongs to the module, so disposal drops it.
    platform.scopeFor(RENDER_MODULE_ID).dispose();
    renderStore.getState().reset();
    emitMockDocumentChanged({ revision: 100, changedBodies: [], removedBodies: [] });
    await settleUntil(() => expect(true).toBe(true));
    expect(renderStore.getState().hydrated).toBe(false);
  });

  it("recomputes dangling bodies from modeling's live body list", async () => {
    const { documentStore } = await import("@/stores/documentStore");
    documentStore
      .getState()
      .applyChange({ bodies: { body1: { id: "body1", name: "Body 1", visible: true } } });
    bootPlatform();
    await settleUntil(() => expect(renderStore.getState().hydrated).toBe(true));

    await renderStore.getState().upsertMaterial(GOLD);
    await renderStore.getState().assignBody("body1", GOLD.id);
    expect(renderStore.getState().danglingBodies).toEqual([]);

    // Modeling's projection loses the body; render must CLASSIFY the assignment
    // as dangling (never delete it) off the published service, with no import of
    // the store that owns it.
    documentStore.getState().applyChange({ bodies: {} });
    await settleUntil(() => expect(renderStore.getState().danglingBodies).toEqual(["body1"]));
  });
});

describe("bootstrap with the render module present", () => {
  it("initializes shell, modeling and render together", async () => {
    const { bootstrapOneCAD } = await import("@/app/bootstrap");
    const platform = bootstrapOneCAD();
    expect(platform.moduleState(RENDER_MODULE_ID)).toBe("ready");
    expect(platform.moduleIds()).toContain(RENDER_MODULE_ID);
    expect(platform.services.has(RenderServices.MaterialQuery)).toBe(true);
    await settleUntil(() => expect(renderStore.getState().hydrated).toBe(true));
  });
});
