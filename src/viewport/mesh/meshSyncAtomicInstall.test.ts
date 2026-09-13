/*
 * VP-HARDENING WP04 — the install is atomic (numerics §11.2).
 *
 * `MeshIngest` prepares BOTH consumers of a new mesh — the registry entry and
 * its scene handle — before anything is published. A failure while preparing
 * must leave the previous display snapshot whole: same entry, same scene
 * object, same budget, and no orphaned GPU geometry. This file forces that
 * failure, which no reachable payload can (the only thing that throws in the
 * prepare phase is scene-handle construction), by wrapping the real
 * `buildBodyObject` in a switchable fault.
 *
 * It lives apart from `meshSync.test.ts` because `vi.mock` is file-wide and the
 * other 55 cases there need the genuine article.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as THREE from "three";

const fault = vi.hoisted(() => ({ failHandleBuild: false }));

vi.mock("../engine/BodyObject", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../engine/BodyObject")>();
  return {
    ...actual,
    buildBodyObject: (
      ...args: Parameters<typeof actual.buildBodyObject>
    ): ReturnType<typeof actual.buildBodyObject> => {
      if (fault.failHandleBuild) throw new Error("scene handle build failed");
      return actual.buildBodyObject(...args);
    },
  };
});

import { MeshIngest } from "./meshSync";
import * as reg from "./meshRegistry";
import { makeBoxMesh, makeCylinderMesh } from "@/ipc/mockMeshes";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { __resetLogForTests } from "@/debug/log";
import type { CadClient } from "@/ipc/client";
import type { DocumentChange } from "@/ipc/types";
import type { ViewportEngine } from "../engine/ViewportEngine";

const tick = () => new Promise((r) => setTimeout(r, 0));

function fakeEngine() {
  const bodiesRoot = new THREE.Group();
  return {
    bodiesRoot,
    invalidate: vi.fn(),
    refreshHighlights: vi.fn(),
    setHighlightState: vi.fn(),
    onAfterRender: () => () => {},
  } as unknown as ViewportEngine & { bodiesRoot: THREE.Group };
}

function fakeClient(getMesh: ReturnType<typeof vi.fn>) {
  const listeners = new Set<(c: DocumentChange) => void>();
  const client = {
    getBodyMesh: getMesh,
    getCurrentMeshPublication: () => null,
    onDocumentChanged: (cb: (c: DocumentChange) => void) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  } as unknown as CadClient;
  const emit = (c: DocumentChange) => {
    const d = documentStore.getState();
    listeners.forEach((l) => l({ ...c, documentId: d.documentId, runtimeSession: d.runtimeSession }));
  };
  return { client, emit };
}

let ingest: MeshIngest | null = null;

beforeEach(() => {
  fault.failHandleBuild = false;
  reg.disposeAll();
  reg.__resetRegistryForTests();
  documentStore.setState({
    documentId: "doc-1",
    runtimeSession: "runtime-1",
    bodies: { body1: { id: "body1", name: "body1", visible: true } },
  });
});
afterEach(() => {
  fault.failHandleBuild = false;
  ingest?.detach();
  ingest = null;
  viewportStore.getState().setStatusHint(null);
  __resetLogForTests({ enabled: false });
});

describe("MeshIngest atomic install", () => {
  it("TEST-MESH-05 a scene-handle build failure leaves the previous body installed, drawn, and unleaked", async () => {
    const getMesh = vi
      .fn<() => Promise<ArrayBuffer>>()
      .mockResolvedValueOnce(makeBoxMesh())
      .mockResolvedValueOnce(makeCylinderMesh());
    const engine = fakeEngine();
    const { client, emit } = fakeClient(getMesh);
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    const original = reg.getEntry("body1");
    const originalGroup = engine.bodiesRoot.children[0];
    expect(original).toBeDefined();
    expect(originalGroup).toBeDefined();
    const heldForBox = ingest.admission.snapshot();

    // The replacement parses, validates, and is admitted — then its scene
    // consumer fails to build.
    fault.failHandleBuild = true;
    emit({
      revision: 2,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:2" }],
      removedBodies: [],
    });
    await tick();

    // Nothing was published: same entry, same scene object, one scene object.
    expect(reg.getEntry("body1")).toBe(original);
    expect(engine.bodiesRoot.children).toHaveLength(1);
    expect(engine.bodiesRoot.children[0]).toBe(originalGroup);
    // The replacement failed, so the body IS now showing history.
    expect(ingest.getDisplayState("body1")).toBe("stale-inspection-only");
    expect(reg.isEntryPromotable(reg.getEntry("body1"))).toBe(false);
    expect(viewportStore.getState().statusHint?.message).toContain("scene handle build failed");
    // The prepared budget went back; only the installed box is still charged.
    expect(ingest.admission.snapshot()).toEqual(heldForBox);

    // The geometry built for the abandoned replacement was disposed rather than
    // orphaned — `disposeAll`'s tripwire is what would say otherwise.
    fault.failHandleBuild = false;
    ingest.detach();
    ingest = null;
    expect(reg.leakTripwireCount).toBe(0);
    expect(reg.registrySize()).toBe(0);
  });

  it("TEST-MESH-05 recovers on the next publication after a scene-handle failure", async () => {
    const getMesh = vi
      .fn<() => Promise<ArrayBuffer>>()
      .mockResolvedValueOnce(makeBoxMesh())
      .mockResolvedValueOnce(makeCylinderMesh())
      .mockResolvedValueOnce(makeCylinderMesh());
    const engine = fakeEngine();
    const { client, emit } = fakeClient(getMesh);
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    fault.failHandleBuild = true;
    emit({ revision: 2, changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:2" }], removedBodies: [] });
    await tick();
    expect(ingest.getDisplayState("body1")).toBe("stale-inspection-only");

    fault.failHandleBuild = false;
    emit({ revision: 3, changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:3" }], removedBodies: [] });
    await tick();

    expect(ingest.getDisplayState("body1")).toBe("current");
    expect(reg.getEntry("body1")!.view.faceCount).toBe(3);
    expect(engine.bodiesRoot.children).toHaveLength(1);
    expect(ingest.admission.snapshot().holdings).toBe(1);
  });
});
