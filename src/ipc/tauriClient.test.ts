/*
 * tauriClient — real-backend CadClient over the Tauri command/event surface.
 *
 * Uses `@tauri-apps/api/mocks` (`mockIPC`) to intercept `invoke` + (opt-in) the
 * event plugin, so command marshalling, the ArrayBuffer mesh path, event
 * subscription, runtime selection and the solver-lane seam are all exercised
 * without a real Tauri runtime. The 278 mock-driven tests default to the mock
 * because they never install the IPC bridge.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { createTauriClient, __setRegenTimeoutForTests, __lastSketchSolvedForTests } from "./tauriClient";
import * as localSolverModule from "./localSolver";
import { createClient } from "./client";
import { mockClient } from "./mockClient";
import { operationToEditCommand } from "./tauriCommandMap";
import { makeBoxMesh } from "./mockMeshes";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { documentStore, emptyDocument, seedMockDocument } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { __resetLogForTests, logSnapshot } from "@/debug/log";
import type { DocumentChange, OperationOp } from "./types";

const tick = (ms = 0) => new Promise((r) => setTimeout(r, ms));

function readyProjection(revision: number, features: unknown[] = []): unknown {
  return {
    status: "ready",
    revision,
    title: "Doc",
    dirty: true,
    bodies: {},
    sketches: {},
    features,
  };
}

afterEach(() => {
  clearMocks();
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  // Reset the shared projection store (the hydration test writes it).
  documentStore.getState().applySnapshot(seedMockDocument());
  // Reset the shared status-hint store (the H7a mid-history insert hint writes it).
  viewportStore.getState().setStatusHint(null);
});

/** A full SketchPlane payload (XZ) an `enter_sketch` mock returns. */
const XZ_PLANE = { kind: "XZ", origin: [0, 0, 0], xAxis: [0, 1, 0], yAxis: [0, 0, 1], normal: [1, 0, 0] };
const PREVIEW_REGION = {
  regionId: "r",
  outerLoop: [],
  holes: [],
  previewTriangles: {
    positions: [0, 0, 10, 0, 10, 10, 0, 10],
    indices: [0, 1, 2, 0, 2, 3],
    holesSubtracted: 0,
  },
};

// ── Runtime selection ─────────────────────────────────────────────────────────

describe("createClient runtime selection", () => {
  it("returns the mock client when no Tauri bridge is present", () => {
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    expect(createClient()).toBe(mockClient);
  });

  it("returns the real tauri client when __TAURI_INTERNALS__ is injected", () => {
    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {};
    const client = createClient();
    expect(client).not.toBe(mockClient);
    expect(typeof client.getBodyMesh).toBe("function");
    expect(typeof client.applyOperation).toBe("function");
  });
});

// ── Command marshalling (camelCase args) ──────────────────────────────────────

describe("tauriClient command marshalling", () => {
  it("newDocument invokes new_document and returns the snapshot DTO", async () => {
    const seen: string[] = [];
    mockIPC((cmd) => {
      seen.push(cmd);
      if (cmd === "new_document") return { documentId: "doc-1", title: "Untitled" };
    });
    const snap = await createTauriClient().newDocument();
    expect(seen).toContain("new_document");
    expect(snap).toEqual({ documentId: "doc-1", title: "Untitled" });
  });

  it("openDocument / importStep pass the path as a camelCase arg", async () => {
    const args: Record<string, unknown> = {};
    mockIPC((cmd, payload) => {
      args[cmd] = payload;
      return { documentId: "d", title: "P" };
    });
    const client = createTauriClient();
    await client.openDocument("/tmp/a.onecad");
    await client.importStep("/tmp/b.step");
    expect(args["open_document"]).toEqual({ path: "/tmp/a.onecad" });
    expect(args["import_step"]).toEqual({ path: "/tmp/b.step" });
  });

  it("getBodyMesh marshals bodyId + lod + generation", async () => {
    let payload: unknown;
    mockIPC((cmd, p) => {
      if (cmd === "get_mesh") {
        payload = p;
        return makeBoxMesh();
      }
    });
    await createTauriClient().getBodyMesh("uuid-1", "coarse");
    expect(payload).toEqual({ bodyId: "uuid-1", lod: "coarse", generation: null });
  });

  it("listRecents + openFileDialog pass through", async () => {
    mockIPC((cmd) => {
      if (cmd === "list_recents")
        return [{ id: "a", name: "A", path: "/a.onecad", modifiedAt: "2026-01-01T00:00:00Z" }];
      if (cmd === "import_file_dialog") return "/chosen.step";
      if (cmd === "open_file_dialog") return "/chosen.onecad";
    });
    const client = createTauriClient();
    expect(await client.listRecents()).toHaveLength(1);
    expect(await client.importFileDialog()).toBe("/chosen.step");
    expect(await client.openFileDialog()).toBe("/chosen.onecad");
  });

  it("importFileDialog invokes import_file_dialog with no args", async () => {
    const payloads: Record<string, unknown> = {};
    mockIPC((cmd, payload) => {
      payloads[cmd] = payload;
      if (cmd === "import_file_dialog") return "/parts/Imported.onecad";
    });
    expect(await createTauriClient().importFileDialog()).toBe("/parts/Imported.onecad");
    expect(payloads["import_file_dialog"]).toEqual({});
  });

  it("openFileDialog resolves null on cancel", async () => {
    mockIPC((cmd) => (cmd === "open_file_dialog" ? null : undefined));
    expect(await createTauriClient().openFileDialog()).toBeNull();
  });

  /** The STEP dialog is its OWN command: `open_file_dialog` filters `.onecad`, so
   *  routing the start-screen import through it made a STEP file unpickable. */
  it("stepFileDialog invokes step_file_dialog with no args (null on cancel)", async () => {
    const payloads: Record<string, unknown> = {};
    mockIPC((cmd, payload) => {
      payloads[cmd] = payload;
      if (cmd === "step_file_dialog") return "/parts/Widget.step";
    });
    expect(await createTauriClient().stepFileDialog()).toBe("/parts/Widget.step");
    expect(payloads["step_file_dialog"]).toEqual({});
    expect(payloads["open_file_dialog"]).toBeUndefined();

    clearMocks();
    mockIPC((cmd) => (cmd === "step_file_dialog" ? null : undefined));
    expect(await createTauriClient().stepFileDialog()).toBeNull();
  });
});

// ── Crash recovery (check_recovery / recover_document) ─────────────────────────

describe("tauriClient crash recovery", () => {
  it("checkRecovery invokes check_recovery and returns the info DTO", async () => {
    const seen: string[] = [];
    mockIPC((cmd) => {
      seen.push(cmd);
      if (cmd === "check_recovery")
        return { autosavePath: "/x/a.onecad", originalPath: "/docs/Bracket.onecad", modifiedMs: 1_700_000_000_000 };
    });
    const info = await createTauriClient().checkRecovery();
    expect(seen).toContain("check_recovery");
    expect(info).toEqual({
      autosavePath: "/x/a.onecad",
      originalPath: "/docs/Bracket.onecad",
      modifiedMs: 1_700_000_000_000,
    });
  });

  it("checkRecovery resolves null when there is nothing to recover", async () => {
    mockIPC((cmd) => (cmd === "check_recovery" ? null : undefined));
    expect(await createTauriClient().checkRecovery()).toBeNull();
  });

  it("recoverDocument invokes recover_document with { documentId, accept } and returns the snapshot / null", async () => {
    const args: Record<string, unknown> = {};
    mockIPC((cmd, payload) => {
      args[cmd] = payload;
      if (cmd === "recover_document") {
        const accept = (payload as { accept: boolean }).accept;
        return accept ? { documentId: "doc-r", title: "Bracket" } : null;
      }
    });
    const client = createTauriClient();
    const snap = await client.recoverDocument("offer-1", true);
    expect(args["recover_document"]).toEqual({ documentId: "offer-1", accept: true });
    expect(snap).toEqual({ documentId: "doc-r", title: "Bracket" });
    expect(await client.recoverDocument("offer-1", false)).toBeNull();
  });

  it("openDocument forwards onRecovery so Rust can enforce the guard", async () => {
    const args: Record<string, unknown> = {};
    mockIPC((cmd, payload) => {
      args[cmd] = payload;
      if (cmd === "open_document") return { documentId: "doc-o", title: "Bracket" };
    });
    const client = createTauriClient();
    await client.openDocument("/docs/Bracket.onecad");
    expect(args["open_document"]).toEqual({
      path: "/docs/Bracket.onecad",
      onRecovery: undefined,
    });
    await client.openDocument("/docs/Bracket.onecad", "openSaved");
    expect(args["open_document"]).toEqual({
      path: "/docs/Bracket.onecad",
      onRecovery: "openSaved",
    });
  });
});

// ── Save / Save As / Export STEP (Rust-owned dialogs + fs) ─────────────────────

describe("tauriClient file commands", () => {
  it("saveDocument marshals the path (null when reusing the last path)", async () => {
    const args: Record<string, unknown> = {};
    mockIPC((cmd, payload) => {
      if (cmd === "save_document") args[cmd] = payload;
    });
    const client = createTauriClient();
    await client.saveDocument();
    expect(args["save_document"]).toEqual({ path: null, previewPng: null });
    await client.saveDocument("/tmp/a.onecad");
    expect(args["save_document"]).toEqual({ path: "/tmp/a.onecad", previewPng: null });
  });

  /*
   * The arg NAME is the contract, not decoration: tauri v2 maps the camelCase
   * key onto the Rust parameter `preview_png`, so a rename here silently drops
   * every thumbnail (the command still succeeds — a missing preview is not an
   * error server-side), which is exactly the kind of failure no other test sees.
   */
  it("saveDocument forwards previewPng verbatim under the name Rust expects", async () => {
    let payload: unknown;
    mockIPC((cmd, p) => {
      if (cmd === "save_document") payload = p;
    });
    const png = "data:image/png;base64,AAAA";
    await createTauriClient().saveDocument("/tmp/a.onecad", png);
    expect(payload).toEqual({ path: "/tmp/a.onecad", previewPng: png });
  });

  it("saveDocumentAs shows the save dialog, saves, and returns its outcome", async () => {
    const seen: string[] = [];
    let saved: unknown;
    mockIPC((cmd, payload) => {
      seen.push(cmd);
      if (cmd === "save_file_dialog") return "/chosen.onecad";
      if (cmd === "save_document") {
        saved = payload;
        return {
          documentId: "doc-1",
          savedRevision: 3,
          currentRevision: 3,
          clean: true,
          path: "/chosen.onecad",
          title: "chosen",
        };
      }
    });
    const outcome = await createTauriClient().saveDocumentAs();
    expect(outcome).toMatchObject({ path: "/chosen.onecad", clean: true, title: "chosen" });
    expect(saved).toEqual({ path: "/chosen.onecad", previewPng: null });
    expect(seen).toContain("save_file_dialog");
    expect(seen).toContain("save_document");
  });

  it("saveDocumentAs carries a previewPng through the dialog to the save", async () => {
    // The first save of a NEW document is always a Save As, so a thumbnail that
    // only rode on `saveDocument` would never reach a brand-new project.
    let saved: unknown;
    mockIPC((cmd, payload) => {
      if (cmd === "save_file_dialog") return "/chosen.onecad";
      if (cmd === "save_document") saved = payload;
    });
    const png = "data:image/png;base64,BBBB";
    await createTauriClient().saveDocumentAs(png);
    expect(saved).toEqual({ path: "/chosen.onecad", previewPng: png });
  });

  it("saveDocumentAs returns null and does NOT save when the dialog is cancelled", async () => {
    const seen: string[] = [];
    mockIPC((cmd) => {
      seen.push(cmd);
      if (cmd === "save_file_dialog") return null;
    });
    expect(await createTauriClient().saveDocumentAs()).toBeNull();
    expect(seen).not.toContain("save_document");
  });

  it("exportStep invokes export_step_file and returns the written path", async () => {
    let payload: unknown;
    mockIPC((cmd, p) => {
      if (cmd === "export_step_file") {
        payload = p;
        return "/out.step";
      }
    });
    expect(await createTauriClient().exportStep()).toBe("/out.step");
    expect(payload).toEqual({ path: null });
  });

  it("exportStep returns null on a cancelled export dialog", async () => {
    mockIPC((cmd) => (cmd === "export_step_file" ? null : undefined));
    expect(await createTauriClient().exportStep()).toBeNull();
  });

  it("exportStl invokes export_stl_file and returns the written path", async () => {
    let payload: unknown;
    mockIPC((cmd, p) => {
      if (cmd === "export_stl_file") {
        payload = p;
        return "/out.stl";
      }
    });
    expect(await createTauriClient().exportStl()).toBe("/out.stl");
    expect(payload).toEqual({ path: null });
  });

  it("exportStl returns null on a cancelled export dialog", async () => {
    mockIPC((cmd) => (cmd === "export_stl_file" ? null : undefined));
    expect(await createTauriClient().exportStl()).toBeNull();
  });

  it("exportObj invokes export_obj_file and returns the written path", async () => {
    let payload: unknown;
    mockIPC((cmd, p) => {
      if (cmd === "export_obj_file") {
        payload = p;
        return "/out.obj";
      }
    });
    expect(await createTauriClient().exportObj()).toBe("/out.obj");
    expect(payload).toEqual({ path: null });
  });

  it("exportObj returns null on a cancelled export dialog", async () => {
    mockIPC((cmd) => (cmd === "export_obj_file" ? null : undefined));
    expect(await createTauriClient().exportObj()).toBeNull();
  });
});

// ── worker-status event ───────────────────────────────────────────────────────

describe("tauriClient worker-status", () => {
  it("delivers worker-status to subscribers and stops after unsubscribe", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });
    const client = createTauriClient();
    const seen: { state: string; epoch: number }[] = [];
    const unsub = client.onWorkerStatus((s) => seen.push(s));
    await tick(); // let the lazy listen() register

    await emit("worker-status", { state: "restarting", epoch: 3 });
    expect(seen).toEqual([{ state: "restarting", epoch: 3 }]);

    unsub();
    await emit("worker-status", { state: "ready", epoch: 4 });
    expect(seen).toHaveLength(1); // no delivery after unsubscribe
  });
});

// ── enter_sketch constraint reverse-map (re-entry hydration) ──────────────────

describe("tauriClient enter_sketch constraints", () => {
  it("reverse-maps the worker-wire constraints into frontend constraints", async () => {
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return {
            sketchId: (payload as { sketchId: string }).sketchId,
            plane: XZ_PLANE,
            entities: [],
            constraints: [
              { id: "cc", type: "Coincident", entities: ["p1", "p2"] },
              { id: "cd", type: "Distance", entities: ["p1", "p2"], value: 90 },
              { id: "bad", type: "NotAThing", entities: ["x"] },
            ],
            dof: 2,
            status: "UnderConstrained",
          };
      },
      { shouldMockEvents: true },
    );
    const session = await createTauriClient().enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    // Known kinds map through; the unknown kind is dropped.
    expect(session.constraints).toEqual([
      { id: "cc", type: "Coincident", entities: ["p1", "p2"] },
      { id: "cd", type: "Distance", entities: ["p1", "p2"], value: 90 },
    ]);
  });
});

// ── get_sketch: pure read for the always-visible sketch layer ─────────────────

describe("tauriClient getSketch (pure read)", () => {
  it("invokes get_sketch with { sketchId }, maps the DTO, and opens NO session", async () => {
    const seen: string[] = [];
    let args: unknown;
    mockIPC((cmd, payload) => {
      seen.push(cmd);
      if (cmd === "get_sketch") {
        args = payload;
        return {
          sketchId: (payload as { sketchId: string }).sketchId,
          plane: XZ_PLANE,
          entities: [
            { id: "p1", type: "Point", at: [0, 0] },
            { id: "p2", type: "Point", at: [40, 0] },
            { id: "l1", type: "Line", p0Ref: "p1", p1Ref: "p2" },
          ],
          constraints: [{ id: "co", type: "Coincident", entities: ["p1", "p2"] }],
          dof: 2,
          status: "UnderConstrained",
        };
      }
    });
    const session = await createTauriClient().getSketch("sketch2");

    expect(args).toEqual({ sketchId: "sketch2" });
    expect(session.plane.kind).toBe("XZ");
    // BUG-5: p1/p2 are l1's endpoints (owned) — no stray Point entities on re-entry.
    expect(session.entities.map((e) => e.id).sort()).toEqual(["l1"]);
    // The Coincident's point uuids remap to their owner line + position.
    expect(session.constraints).toEqual([
      { id: "co", type: "Coincident", entities: ["l1", "l1"], positions: ["Start", "End"] },
    ]);
    // Pure read: no AddSketch (apply_edit_command) and no enter_sketch fired.
    expect(seen).toContain("get_sketch");
    expect(seen).not.toContain("apply_edit_command");
    expect(seen).not.toContain("enter_sketch");
  });

  it("surfaces a rejected get_sketch as an Error with the ApiError kind", async () => {
    mockIPC((cmd) => (cmd === "get_sketch" ? Promise.reject({ kind: "notFound", message: "no sketch" }) : undefined));
    await expect(createTauriClient().getSketch("ghost")).rejects.toThrow(/notFound: no sketch/);
  });

  it("resolves an ENTERED sketch's frontend id to its backend UUID (like its siblings)", async () => {
    // The explicit-`sketchId` enter lane keeps minting a SEPARATE backend UUID, so
    // the frontend id is NOT a valid backend SketchId; sending it raw makes the
    // Rust side reject (or, worse, read a different sketch).
    let enteredBackendId: string | undefined;
    let getArgs: unknown;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch") {
          enteredBackendId = (payload as { sketchId: string }).sketchId;
          return {
            sketchId: enteredBackendId,
            plane: XZ_PLANE,
            entities: [],
            constraints: [],
            dof: 0,
            status: "UnderConstrained",
          };
        }
        if (cmd === "get_sketch") {
          getArgs = payload;
          return {
            sketchId: (payload as { sketchId: string }).sketchId,
            plane: XZ_PLANE,
            entities: [],
            constraints: [],
            dof: 0,
            status: "UnderConstrained",
          };
        }
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk-front" });
    expect(enteredBackendId).toBeDefined();
    expect(enteredBackendId).not.toBe("sk-front"); // the two id spaces really differ

    await client.getSketch("sk-front");
    expect(getArgs).toEqual({ sketchId: enteredBackendId });
  });
});

// ── Re-entry after reopen: enter a PERSISTED sketch without duplicating it ────

describe("tauriClient enter persisted sketch (re-entry hydration)", () => {
  it("enters a persisted document sketch WITHOUT AddSketch and seeds the id-map", async () => {
    const seen: string[] = [];
    let enterSketchId: string | undefined;
    let upsertOps: unknown[] | undefined;
    mockIPC(
      (cmd, payload) => {
        seen.push(cmd);
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch") {
          enterSketchId = (payload as { sketchId: string }).sketchId;
          return {
            sketchId: enterSketchId,
            plane: XZ_PLANE,
            // Real stored geometry (a line + its two backing points) + a constraint.
            entities: [
              { id: "p1", type: "Point", at: [0, 0] },
              { id: "p2", type: "Point", at: [40, 0] },
              { id: "l1", type: "Line", p0Ref: "p1", p1Ref: "p2" },
            ],
            constraints: [{ id: "co", type: "Coincident", entities: ["p1", "p2"] }],
            dof: 2,
            status: "UnderConstrained",
          };
        }
        if (cmd === "sketch_upsert") {
          upsertOps = (payload as { ops: unknown[] }).ops;
          return {
            sketchId: (payload as { sketchId: string }).sketchId,
            sketchRevision: 2,
            dof: 2,
            status: "UnderConstrained",
            solvedPositions: {},
          };
        }
      },
      { shouldMockEvents: true },
    );
    // "sketch2" is a persisted document sketch (seedMockDocument) with no in-memory map.
    const client = createTauriClient();
    const session = await client.enterSketch("sketch2");

    // The persisted id IS the backend SketchId — enter_sketch targets it directly…
    expect(enterSketchId).toBe("sketch2");
    // …and NO AddSketch fired (no duplicate empty sketch created).
    expect(seen).not.toContain("apply_edit_command");
    expect(seen).toContain("enter_sketch");
    // Real geometry hydrated (not an empty duplicate). BUG-5: p1/p2 are l1's
    // endpoints (owned) — no stray Points; the Coincident remaps to owner + position.
    expect([...session.entities.map((e) => e.id)].sort()).toEqual(["l1"]);
    expect(session.constraints).toEqual([
      { id: "co", type: "Coincident", entities: ["l1", "l1"], positions: ["Start", "End"] },
    ]);

    // A re-upsert of the SAME hydrated entities marshals to ZERO ops (id-map seeded).
    await client.sketchUpsert("sketch2", session.entities, session.constraints);
    expect(upsertOps).toEqual([]);
  });
});

// ── enter_sketch({newOnPlane}) w/o sketchId: minted id IS the backend SketchId ─

describe("tauriClient enter_sketch minted id (no explicit sketchId)", () => {
  it("adopts the minted frontend id as the backend SketchId, so AddSketch, enter_sketch and the returned session.sketchId all agree", async () => {
    let addSketchId: string | undefined;
    let enterSketchId: string | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          const command = (payload as { command: { sketch: { id: string } } }).command;
          addSketchId = command.sketch.id;
          return readyProjection(1);
        }
        if (cmd === "enter_sketch") {
          enterSketchId = (payload as { sketchId: string }).sketchId;
          return { sketchId: enterSketchId, plane: XZ_PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
        }
      },
      { shouldMockEvents: true },
    );
    const session = await createTauriClient().enterSketch({ newOnPlane: "XZ" });

    // Both wire calls carried the SAME minted id — a real UUID, not "sk-N".
    expect(addSketchId).toBeDefined();
    expect(addSketchId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i);
    expect(enterSketchId).toBe(addSketchId);
    // …and the session id returned to the caller matches too (projection-store key).
    expect(session.sketchId).toBe(addSketchId);
  });

  it("explicit sketchId (newOnPlane w/ sketchId) still mints a SEPARATE backend UUID (unchanged)", async () => {
    let addSketchId: string | undefined;
    let enterSketchId: string | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          const command = (payload as { command: { sketch: { id: string } } }).command;
          addSketchId = command.sketch.id;
          return readyProjection(1);
        }
        if (cmd === "enter_sketch") {
          enterSketchId = (payload as { sketchId: string }).sketchId;
          return { sketchId: enterSketchId, plane: XZ_PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
        }
      },
      { shouldMockEvents: true },
    );
    const session = await createTauriClient().enterSketch({ newOnPlane: "XZ", sketchId: "sk" });

    expect(addSketchId).toBeDefined();
    expect(addSketchId).not.toBe("sk"); // backend id is a minted UUID, distinct from the frontend id
    expect(enterSketchId).toBe(addSketchId);
    expect(session.sketchId).toBe("sk"); // frontend id kept as the session id, as before
  });
});

// ── enter_sketch({newOnDatum}) — the datum host branch (DATUM W1) ────────────

describe("tauriClient enter_sketch on a DATUM plane", () => {
  it("fires AddSketch with the datum attachment + the datum's resolved frame verbatim", async () => {
    const DATUM_PLANE = {
      origin: [0, 0, 10] as [number, number, number],
      xAxis: [0, 1, 0] as [number, number, number],
      yAxis: [-1, 0, 0] as [number, number, number],
      normal: [0, 0, 1] as [number, number, number],
    };
    let command: { cmd: string; sketch: Record<string, unknown> } | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          command = (payload as { command: { cmd: string; sketch: Record<string, unknown> } }).command;
          return readyProjection(1);
        }
        if (cmd === "enter_sketch") {
          const sketchId = (payload as { sketchId: string }).sketchId;
          return {
            sketchId,
            plane: { kind: "custom", ...DATUM_PLANE },
            entities: [],
            constraints: [],
            dof: 0,
            status: "FullyConstrained",
          };
        }
      },
      { shouldMockEvents: true },
    );

    await createTauriClient().enterSketch({
      newOnDatum: { datumId: "datum-uuid" },
      plane: { kind: "custom", ...DATUM_PLANE },
    });

    expect(command?.cmd).toBe("addSketch");
    // The typed attachment the core serde expects, plus the BACKEND-resolved
    // basis carried through untouched (a `custom` plane on the wire).
    expect(command?.sketch.attachment).toEqual({ kind: "datum", datum: "datum-uuid" });
    expect(command?.sketch.plane).toEqual(DATUM_PLANE);
  });

  it("a failed enter on a datum still runs the orphan DeleteSketch compensation", async () => {
    const commands: { cmd: string }[] = [];
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          commands.push((payload as { command: { cmd: string } }).command);
          return readyProjection(1);
        }
        if (cmd === "enter_sketch") throw new Error("workerDown: no worker");
      },
      { shouldMockEvents: true },
    );

    await expect(
      createTauriClient().enterSketch({
        newOnDatum: { datumId: "datum-uuid" },
        plane: { kind: "custom", origin: [0, 0, 10], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
      }),
    ).rejects.toThrow(/workerDown/);

    expect(commands.map((c) => c.cmd)).toEqual(["addSketch", "deleteSketch"]);
  });
});

// ── enter_sketch on a model FACE → add_sketch_on_face (SKETCH-ON-FACE W2) ─────

describe("tauriClient enter_sketch on a model FACE", () => {
  const FACE_PLANE = {
    kind: "custom" as const,
    origin: [0, 0, 15] as [number, number, number],
    xAxis: [0, 1, 0] as [number, number, number],
    yAxis: [-1, 0, 0] as [number, number, number],
    normal: [0, 0, 1] as [number, number, number],
  };

  /** A `SketchOnFaceDto` for a face whose boundary projected to a rectangle. */
  const onFaceDto = (sketchId: string) => ({
    sketchId,
    plane: { origin: FACE_PLANE.origin, xAxis: FACE_PLANE.xAxis, yAxis: FACE_PLANE.yAxis, normal: FACE_PLANE.normal },
    entityCount: 8,
    constraintCount: 4,
    faceCount: 1,
    hasClosedBoundary: true,
    projectedBoundaryVersion: 1,
  });

  it("routes through add_sketch_on_face — NOT apply_edit_command — with the FE-minted id adopted verbatim", async () => {
    const seen: string[] = [];
    let onFaceArgs: Record<string, unknown> | undefined;
    let enterSketchId: string | undefined;
    mockIPC(
      (cmd, payload) => {
        seen.push(cmd);
        if (cmd === "add_sketch_on_face") {
          onFaceArgs = payload as Record<string, unknown>;
          return onFaceDto(String((payload as { sketchId: string }).sketchId));
        }
        if (cmd === "enter_sketch") {
          enterSketchId = (payload as { sketchId: string }).sketchId;
          return { sketchId: enterSketchId, plane: FACE_PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
        }
        return readyProjection(1);
      },
      { shouldMockEvents: true },
    );

    const session = await createTauriClient().enterSketch({
      newOnFace: { bodyId: "body1", elementId: "el_1", topoKey: "f:4", worldPoint: [1, 2, 15] },
      plane: FACE_PLANE,
    });

    // The face lane must NOT assemble an AddSketch of its own: that could only
    // produce an EMPTY host-face sketch with projectedBoundaryVersion 0.
    expect(seen).not.toContain("apply_edit_command");
    expect(seen).toContain("add_sketch_on_face");

    // The id the frontend minted IS the backend SketchId, the enter target, and
    // the session id — one string, or the projection store's entry is stranded.
    expect(onFaceArgs?.sketchId).toBe(session.sketchId);
    expect(enterSketchId).toBe(session.sketchId);
    expect(String(onFaceArgs?.sketchId)).toMatch(/^[0-9a-f]{8}-/);

    // camelCase args; `body_<uuid>` worker wire form; topoKey forwarded (the rung
    // that resolves a just-promoted, never-consumed ElementId).
    expect(onFaceArgs).toMatchObject({
      snapshotId: 0,
      bodyId: "body_body1", // the frontend's bare id, prefixed for the wire
      elementId: "el_1",
      topoKey: "f:4",
    });
    expect(typeof onFaceArgs?.name).toBe("string");
  });

  it("forwards a null topoKey when the caller has none, and keeps the body_ prefix idempotent", async () => {
    let onFaceArgs: Record<string, unknown> | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "add_sketch_on_face") {
          onFaceArgs = payload as Record<string, unknown>;
          return onFaceDto(String((payload as { sketchId: string }).sketchId));
        }
        if (cmd === "enter_sketch") {
          const sketchId = (payload as { sketchId: string }).sketchId;
          return { sketchId, plane: FACE_PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
        }
        return readyProjection(1);
      },
      { shouldMockEvents: true },
    );

    await createTauriClient().enterSketch({
      newOnFace: { bodyId: "body_abc", elementId: "el_2" },
      plane: FACE_PLANE,
    });
    expect(onFaceArgs?.topoKey).toBeNull();
    expect(onFaceArgs?.bodyId).toBe("body_abc");
  });

  it("hydrates the projected LOCKED entities the enter_sketch DTO carries", async () => {
    // What `add_sketch_on_face` committed comes back through the ordinary
    // enter_sketch path: 4 boundary points + 4 locked lines + 4 Fixed pins.
    const pts = ["p0", "p1", "p2", "p3"];
    const at: Record<string, [number, number]> = {
      p0: [-30, 40], p1: [-30, -40], p2: [30, -40], p3: [30, 40],
    };
    mockIPC(
      (cmd, payload) => {
        if (cmd === "add_sketch_on_face") return onFaceDto(String((payload as { sketchId: string }).sketchId));
        if (cmd === "enter_sketch") {
          return {
            sketchId: (payload as { sketchId: string }).sketchId,
            plane: FACE_PLANE,
            entities: [
              ...pts.map((id) => ({ id, type: "Point", at: at[id], referenceLocked: true })),
              ...pts.map((id, i) => ({
                id: `ln${i}`,
                type: "Line",
                p0Ref: id,
                p1Ref: pts[(i + 1) % pts.length],
                referenceLocked: true,
              })),
            ],
            constraints: pts.map((id, i) => ({ id: `fx${i}`, type: "Fixed", entities: [id] })),
            dof: 0,
            status: "FullyConstrained",
          };
        }
        return readyProjection(1);
      },
      { shouldMockEvents: true },
    );

    const session = await createTauriClient().enterSketch({
      newOnFace: { bodyId: "body1", elementId: "el_1", topoKey: "f:4" },
      plane: FACE_PLANE,
    });

    // Owned child points ride on their lines — no stray dots.
    expect(session.entities.map((e) => e.type)).toEqual(["Line", "Line", "Line", "Line"]);
    expect(session.entities.every((e) => e.referenceLocked)).toBe(true);
    expect(session.constraints).toHaveLength(4);
    expect(session.constraints.every((c) => c.type === "Fixed")).toBe(true);
  });

  it("still runs the orphan DeleteSketch compensation when enter_sketch rejects", async () => {
    const commands: { cmd: string }[] = [];
    let created: string | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "add_sketch_on_face") {
          created = String((payload as { sketchId: string }).sketchId);
          return onFaceDto(created);
        }
        if (cmd === "apply_edit_command") {
          commands.push((payload as { command: { cmd: string } }).command);
          return readyProjection(1);
        }
        if (cmd === "enter_sketch") throw new Error("workerDown: no worker");
      },
      { shouldMockEvents: true },
    );

    await expect(
      createTauriClient().enterSketch({
        newOnFace: { bodyId: "body1", elementId: "el_1", topoKey: "f:4" },
        plane: FACE_PLANE,
      }),
    ).rejects.toThrow(/workerDown/);

    // The sketch add_sketch_on_face committed would otherwise orphan in the tree
    // (with its projected geometry), and every retry would mint another.
    expect(commands.map((c) => c.cmd)).toEqual(["deleteSketch"]);
    expect((commands[0] as { cmd: string; sketch?: unknown }).sketch).toBe(created);
  });
});

// ── enter_sketch failure → AWAITED DeleteSketch compensation (orphan cleanup) ─

interface WireCmd {
  cmd: string;
  sketch?: string | { id: string; name: string };
}

describe("tauriClient enterSketch orphan cleanup (DeleteSketch compensation)", () => {
  it("deletes the just-created sketch when enter_sketch rejects, cleans the map, and rethrows the ORIGINAL error", async () => {
    const commands: WireCmd[] = [];
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          commands.push((payload as { command: WireCmd }).command);
          return readyProjection(1);
        }
        if (cmd === "enter_sketch") return Promise.reject({ kind: "workerDown", message: "no worker" });
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();

    // First attempt: AddSketch commits, enter_sketch rejects → DeleteSketch fires.
    await expect(client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" })).rejects.toThrow(/workerDown: no worker/);
    expect(commands.map((c) => c.cmd)).toEqual(["addSketch", "deleteSketch"]);
    const add1 = commands[0].sketch as { id: string };
    // The DeleteSketch targets the SAME backend SketchId AddSketch minted.
    expect(commands[1].sketch).toBe(add1.id);

    // Retry: the map was cleaned, so a fresh AddSketch fires with a DISTINCT id
    // (had the map leaked, ensureBackendSketch would reuse it and skip AddSketch).
    await expect(client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" })).rejects.toThrow(/workerDown/);
    expect(commands.map((c) => c.cmd)).toEqual(["addSketch", "deleteSketch", "addSketch", "deleteSketch"]);
    const add2 = commands[2].sketch as { id: string };
    expect(add2.id).not.toBe(add1.id);
  });

  it("suffixes the error when the DeleteSketch compensation ALSO rejects", async () => {
    // The orphan-cleanup failure is on the structured log lane now (Wave F).
    __resetLogForTests();
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          const command = (payload as { command: WireCmd }).command;
          if (command.cmd === "deleteSketch") return Promise.reject({ kind: "opFailed", message: "delete boom" });
          return readyProjection(1);
        }
        if (cmd === "enter_sketch") return Promise.reject({ kind: "workerDown", message: "no worker" });
      },
      { shouldMockEvents: true },
    );
    await expect(createTauriClient().enterSketch({ newOnPlane: "XZ" })).rejects.toThrow(
      /workerDown: no worker \(cleanup failed — empty sketch left in tree\)$/,
    );
    expect(
      logSnapshot().filter((e) => e.level === "error" && e.msg.includes("orphan cleanup")),
    ).toHaveLength(1);
    __resetLogForTests({ enabled: false });
  });
});

// ── Unique sketch names via nextSketchName (single source: the projection store) ─

describe("tauriClient fresh-sketch naming", () => {
  it("names a new plane-picked sketch via nextSketchName off the live store (Sketch 1 → Sketch 2)", async () => {
    documentStore.getState().applySnapshot({
      status: "ready",
      revision: 1,
      title: "D",
      dirty: false,
      bodies: {},
      sketches: {
        s1: {
          id: "s1",
          name: "Sketch 1",
          visible: true,
          dof: 0,
          status: "ok",
          geometryToken: "geometry-s1",
        },
      },
      datums: {},
      features: [],
      appliedOps: 0,
      geometrySource: "live",
    });
    let addName: string | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          addName = (payload as { command: { sketch: { name: string } } }).command.sketch.name;
          return readyProjection(1);
        }
        if (cmd === "enter_sketch")
          return {
            sketchId: (payload as { sketchId: string }).sketchId,
            plane: XZ_PLANE,
            entities: [],
            constraints: [],
            dof: 0,
            status: "FullyConstrained",
          };
      },
      { shouldMockEvents: true },
    );
    await createTauriClient().enterSketch({ newOnPlane: "XZ" });
    expect(addName).toBe("Sketch 2");
  });
});

// ── Mesh ArrayBuffer path through the MESH1 parser ────────────────────────────

describe("tauriClient mesh path", () => {
  it("returns the get_mesh ArrayBuffer verbatim and the MESH1 parser accepts it", async () => {
    const bytes = makeBoxMesh();
    mockIPC((cmd) => (cmd === "get_mesh" ? bytes : undefined));
    const buf = await createTauriClient().getBodyMesh("uuid", "coarse");
    expect(buf.byteLength).toBeGreaterThan(64);
    const view = parseMeshPayload(buf);
    expect(view.positions.length).toBeGreaterThan(0);
    expect(view.indices.length % 3).toBe(0);
  });
});

// ── document-changed event subscription ───────────────────────────────────────

describe("tauriClient events", () => {
  it("delivers document-changed to subscribers and stops after unsubscribe", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });
    const client = createTauriClient();
    const seen: DocumentChange[] = [];
    const unsub = client.onDocumentChanged((c) => seen.push(c));
    await tick(); // let the lazy listen() register

    await emit("document-changed", {
      revision: 7,
      changedBodies: [{ bodyId: "b1", meshKey: "b1:coarse:7" }],
      removedBodies: [],
    });
    expect(seen).toHaveLength(1);
    expect(seen[0].revision).toBe(7);
    expect(seen[0].changedBodies[0].bodyId).toBe("b1");

    unsub();
    await emit("document-changed", { revision: 8, changedBodies: [], removedBodies: [] });
    expect(seen).toHaveLength(1); // no delivery after unsubscribe
  });

  it("openDocument attaches the event bridge BEFORE invoking — an event fired during " +
     "the command round-trip reaches the store (SAVE/OPEN hardening)", async () => {
    // The reopen bug class: the pre-regen projection-updated (and a fast
    // open-regen's document-changed) fires DURING the open_document round-trip.
    // With the old fire-and-forget ensureEvents, listeners could still be
    // attaching — those events were silently lost.
    documentStore.getState().applySnapshot(emptyDocument());
    mockIPC(async (cmd) => {
      if (cmd === "open_document") {
        await emit("projection-updated", {
          status: "ready",
          documentId: "doc-reopened",
          revision: 1,
          title: "Reopened",
          dirty: false,
          bodies: { b1: { id: "b1", name: "Body 1", visible: true } },
          sketches: {},
          datums: {},
          features: [],
        });
        return { documentId: "doc-reopened", title: "Reopened" };
      }
      return undefined;
    }, { shouldMockEvents: true });
    const client = createTauriClient();

    await client.openDocument("/tmp/reopened.onecad");
    await tick();

    const s = documentStore.getState();
    expect(s.documentId).toBe("doc-reopened");
    expect(s.bodies.b1).toBeDefined();
  });
});

// ── OperationOp → EditCommand mapping (pure) ──────────────────────────────────

describe("operationToEditCommand", () => {
  it("maps Extrude to addOperation with camelCase Scalar params + profile ref", () => {
    const cmd = operationToEditCommand({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r1",
      params: { distance: 25, extrudeMode: "Blind", booleanMode: "NewBody" },
    });
    expect(cmd.cmd).toBe("addOperation");
    if (cmd.cmd !== "addOperation") throw new Error("unreachable");
    // H7a: author AT the cursor, unconditionally — a frontier commit is a no-op
    // change (cursor === len), a rolled-back one lands mid-history instead of
    // appending past the bar as a permanently-inert draft.
    expect(cmd.atCursor).toBe(true);
    expect(cmd.record.recordId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
    expect(cmd.record.opType).toBe("Extrude");
    const p = cmd.record.params as unknown as Record<string, unknown>;
    expect(p.distance).toEqual({ value: 25 });
    expect(p.draftAngleDeg).toEqual({ value: 0 });
    expect(p.extrudeMode).toBe("Blind");
    expect(p.booleanMode).toBe("NewBody");
    expect(p.twoDirections).toBe(false);
    expect(p.profile).toEqual({ sketchId: "sk", regionId: "r1" });
  });

  it("maps Revolve to addOperation with angleDeg (DEGREES), sketchLine axis + profile", () => {
    const cmd = operationToEditCommand({
      opType: "Revolve",
      sketchId: "sk",
      regionId: "r1",
      params: {
        angleDeg: 270,
        axis: { kind: "sketchLine", sketchId: "sk", lineId: "line-7" },
        booleanMode: "NewBody",
      },
    });
    expect(cmd.cmd).toBe("addOperation");
    if (cmd.cmd !== "addOperation") throw new Error("unreachable");
    expect(cmd.record.opType).toBe("Revolve");
    const p = cmd.record.params as unknown as Record<string, unknown>;
    // Unit pinned: angle passes through as a Scalar in DEGREES (no radians).
    expect(p.angleDeg).toEqual({ value: 270 });
    expect(p.axis).toEqual({ kind: "sketchLine", sketchId: "sk", lineId: "line-7" });
    expect(p.booleanMode).toBe("NewBody");
    expect(p.profile).toEqual({ sketchId: "sk", regionId: "r1" });
  });

  it("defaults Revolve booleanMode + omits an absent axis", () => {
    const cmd = operationToEditCommand({
      opType: "Revolve",
      sketchId: "sk",
      regionId: "r1",
      params: { angleDeg: 360 },
    });
    if (cmd.cmd !== "addOperation") throw new Error("unreachable");
    const p = cmd.record.params as unknown as Record<string, unknown>;
    expect(p.angleDeg).toEqual({ value: 360 });
    expect(p.booleanMode).toBe("NewBody");
    expect("axis" in p).toBe(false);
    expect("targetBodyId" in p).toBe(false);
  });

  it("maps a Revolve featureId edit to updateOperationParams (param-only re-edit)", () => {
    const cmd = operationToEditCommand({
      opType: "Revolve",
      featureId: "rev-record-uuid",
      sketchId: "sk",
      regionId: "r1",
      params: { angleDeg: 90 },
    });
    if (cmd.cmd !== "updateOperationParams") throw new Error("unreachable");
    expect(cmd.record).toBe("rev-record-uuid");
    expect(cmd.op.opType).toBe("Revolve");
    expect((cmd.op.params as unknown as Record<string, unknown>).angleDeg).toEqual({ value: 90 });
  });

  it("maps Boolean to addOperation with real body refs + operation", () => {
    const cmd = operationToEditCommand({
      opType: "Boolean",
      inputs: [],
      params: { operation: "Cut", targetBodyId: "body-a", toolBodyId: "body-b" },
    });
    if (cmd.cmd !== "addOperation") throw new Error("unreachable");
    expect(cmd.record.opType).toBe("Boolean");
    expect(cmd.record.params).toEqual({
      operation: "Cut",
      targetBodyId: "body-a",
      toolBodyId: "body-b",
    });
  });

  it("maps a featureId edit to updateOperationParams targeting the record id", () => {
    const cmd = operationToEditCommand({
      opType: "Extrude",
      featureId: "record-uuid",
      sketchId: "s",
      regionId: "r",
      params: { distance: 5 },
    });
    if (cmd.cmd !== "updateOperationParams") throw new Error("unreachable");
    expect(cmd.record).toBe("record-uuid");
    expect(cmd.op.opType).toBe("Extrude");
  });
});

// ── applyOperation / undo / redo — command + regen correlation ────────────────

describe("tauriClient edit + correlation", () => {
  it("applyOperation invokes apply_edit_command and correlates the regen", async () => {
    let command: { cmd?: string } | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          command = (payload as { command: { cmd?: string } }).command;
          setTimeout(() => {
            // Real backend order: document-changed (bodies), then the regen-finished
            // sibling. A fresh AddOperation (recordId) settles on the latter.
            void emit("document-changed", {
              revision: 6,
              changedBodies: [{ bodyId: "nb", meshKey: "nb:coarse:6" }],
              removedBodies: [],
            });
            void emit("regen-finished", { revision: 6, sourceRevision: 5, outcome: "published" });
          }, 0);
          return readyProjection(5, [
            { id: "f", kind: "extrude", label: "Extrude", valueText: "25.0 mm", status: "dirty" },
          ]);
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300);
    const op: OperationOp = {
      opType: "Boolean",
      inputs: [],
      params: { operation: "Union", targetBodyId: "t", toolBodyId: "u" },
    };
    const res = await createTauriClient().applyOperation(op);
    expect(command?.cmd).toBe("addOperation");
    expect(res.revision).toBe(6);
    expect(res.changedBodies).toEqual([{ bodyId: "nb", meshKey: "nb:coarse:6" }]);
    expect(res.removedBodies).toEqual([]);
    expect(res.features).toHaveLength(1);
    expect(res.opLabel).toBe("Union");
    expect(res.terminal).toBe("published");
  });

  it("applyOperation falls back to the pre-regen projection when no regen fires", async () => {
    mockIPC((cmd) => (cmd === "apply_edit_command" ? readyProjection(5) : undefined), {
      shouldMockEvents: true,
    });
    __setRegenTimeoutForTests(20);
    const res = await createTauriClient().applyOperation({
      opType: "Boolean",
      params: { operation: "Union", targetBodyId: "t", toolBodyId: "u" },
    } as OperationOp);
    expect(res.revision).toBe(5);
    expect(res.changedBodies).toEqual([]);
    expect(res.terminal).toBe("timeout");
  });

  it("undo invokes undo and reports the removed bodies from the regen", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "undo") {
          setTimeout(() => {
            void emit("document-changed", { revision: 4, changedBodies: [], removedBodies: ["gone"] });
          }, 0);
          return readyProjection(4);
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300);
    const res = await createTauriClient().undo();
    expect(res.removedBodies).toEqual(["gone"]);
    expect(res.revision).toBe(4);
  });

  // ── insert_step (STEP-IMPORT WP-A) ──────────────────────────────────────────
  // Pins the shape the Rust wave must land: NO args (Rust owns the dialog), a
  // projection back, regen correlated like any other edit, null == cancelled.

  it("insertStep invokes insert_step with no args and correlates the regen", async () => {
    const payloads: Record<string, unknown> = {};
    mockIPC(
      (cmd, payload) => {
        if (cmd === "insert_step") {
          payloads[cmd] = payload;
          setTimeout(() => {
            void emit("document-changed", {
              revision: 7,
              changedBodies: [
                { bodyId: "imp1", meshKey: "imp1:coarse:7" },
                { bodyId: "imp2", meshKey: "imp2:coarse:7" },
              ],
              removedBodies: [],
            });
            void emit("regen-finished", { revision: 7, sourceRevision: 6, outcome: "published" });
          }, 0);
          return readyProjection(6, [
            { id: "f-imp", kind: "boolean", opType: "ImportStep", label: "Import", valueText: "", status: "dirty" },
          ]);
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300);
    const res = await createTauriClient().insertStep();
    expect(payloads["insert_step"]).toEqual({});
    expect(res?.revision).toBe(7);
    expect(res?.changedBodies.map((b) => b.bodyId)).toEqual(["imp1", "imp2"]);
    expect(res?.opLabel).toBe("Import");
    expect(res?.features[0].opType).toBe("ImportStep");
  });

  it("insertStep resolves null when the Rust dialog was cancelled", async () => {
    mockIPC((cmd) => (cmd === "insert_step" ? null : undefined), { shouldMockEvents: true });
    // A short timeout would still pass if the awaiter leaked; the point is that it
    // returns PROMPTLY (no correlation wait at all) on a cancel.
    __setRegenTimeoutForTests(5000);
    await expect(createTauriClient().insertStep()).resolves.toBeNull();
    __setRegenTimeoutForTests(300);
  });

  it("surfaces a rejected command as an Error carrying the ApiError kind", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "apply_edit_command") return Promise.reject({ kind: "opFailed", message: "boom" });
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(50);
    await expect(
      createTauriClient().applyOperation({
        opType: "Boolean",
        params: { operation: "Union", targetBodyId: "t", toolBodyId: "u" },
      } as OperationOp),
    ).rejects.toThrow(/opFailed: boom/);
  });
});

// ── Solver-lane seam (local) + preview-commit delegation ──────────────────────

describe("tauriClient sketch solver lane (real commands)", () => {
  const xzPlane = { kind: "XZ", origin: [0, 0, 0], xAxis: [0, 1, 0], yAxis: [0, 0, 1], normal: [1, 0, 0] };

  it("enters via AddSketch + enter_sketch, marshals upserts to SketchEditOp[], finishes", async () => {
    const seen: string[] = [];
    let upsertArgs: { sketchId?: string; ops?: { op: string; entity?: { kind: string } }[] } | undefined;
    mockIPC(
      (cmd, payload) => {
        seen.push(cmd);
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return {
            sketchId: (payload as { sketchId: string }).sketchId,
            plane: xzPlane,
            entities: [],
            constraints: [],
            dof: 4,
            status: "UnderConstrained",
          };
        if (cmd === "sketch_upsert") {
          upsertArgs = payload as typeof upsertArgs;
          return { sketchId: (payload as { sketchId: string }).sketchId, sketchRevision: 1, dof: 3, status: "UnderConstrained", solvedPositions: {} };
        }
        if (cmd === "finish_sketch") return { regions: [] };
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    const s = await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    expect(s.plane.kind).toBe("XZ");
    expect(s.sketchId).toBe("sk"); // keeps the FRONTEND id (map holds the UUID)
    expect(seen).toContain("apply_edit_command"); // AddSketch created the sketch
    expect(seen).toContain("enter_sketch");

    const up = await client.sketchUpsert(
      "sk",
      [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
      [{ id: "c1", type: "Horizontal", entities: ["e1"] }],
    );
    expect(up.dof).toBe(3);
    expect(up.sketchId).toBe("sk");
    // A Line marshals to two synthesized Points + the Line; H → an AddConstraint.
    const ops = upsertArgs?.ops ?? [];
    expect(ops.filter((o) => o.op === "addEntity" && o.entity?.kind === "point")).toHaveLength(2);
    expect(ops.some((o) => o.op === "addEntity" && o.entity?.kind === "line")).toBe(true);
    expect(ops.some((o) => o.op === "addConstraint")).toBe(true);

    const fin = await client.finishSketch("sk");
    expect(Array.isArray(fin.regions)).toBe(true);
    expect(seen).toContain("sketch_upsert");
    expect(seen).toContain("finish_sketch");
  });

  /*
   * H3b — the inspector's sketch-dimension quick path. Two things are load-bearing:
   * it works on a sketch that was NEVER entered (no id-map — the only state the
   * history panel can be in), and the Angle deg→rad conversion goes through
   * `angleUnits.toWireDimensionValue`, the seam all three marshalling sites share.
   */
  it("setSketchDimension sends ONE setDimension op, converting an Angle deg→rad", async () => {
    let upsertArgs: { sketchId?: string; ops?: { op: string; constraint?: string; value?: { value: number } }[] } | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "sketch_upsert") {
          upsertArgs = payload as typeof upsertArgs;
          return {
            sketchId: (payload as { sketchId: string }).sketchId,
            sketchRevision: 4,
            dof: 0,
            status: "FullyConstrained",
            solvedPositions: {},
          };
        }
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();

    // A LENGTH passes through untouched, addressed by the ids the caller holds
    // (which for a never-entered sketch ARE the backend ids).
    const res = await client.setSketchDimension("sk-uuid", "c-uuid", "Distance", 60);
    expect(res.sketchRevision).toBe(4);
    expect(upsertArgs?.sketchId).toBe("sk-uuid");
    expect(upsertArgs?.ops).toEqual([
      { op: "setDimension", constraint: "c-uuid", value: { value: 60 } },
    ]);

    // An ANGLE is DEGREES in the UI and RADIANS on the wire (SCHEMA §7.4).
    await client.setSketchDimension("sk-uuid", "c-ang", "Angle", 45);
    expect(upsertArgs?.ops?.[0].value?.value).toBeCloseTo(Math.PI / 4, 12);
  });

  it("cancelSketch invokes cancel_sketch on the backend sketch id", async () => {
    const seen: string[] = [];
    mockIPC(
      (cmd, payload) => {
        seen.push(cmd);
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return { sketchId: (payload as { sketchId: string }).sketchId, plane: xzPlane, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
        if (cmd === "cancel_sketch") return null;
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    await client.cancelSketch("sk");
    expect(seen).toContain("cancel_sketch");
  });
});

describe("tauriClient preview seam (local) + commit delegation", () => {
  it("propagates PreviewOp NeedsRepair evidence as a structural preview failure", async () => {
    mockIPC((cmd) => {
      if (cmd === "get_sketch") {
        return {
          sketchId: "sk",
          plane: XZ_PLANE,
          entities: [],
          constraints: [],
          dof: 0,
          status: "FullyConstrained",
        };
      }
      if (cmd === "get_sketch_regions") return { regions: [PREVIEW_REGION] };
      if (cmd === "preview_op") {
        return {
          snapshotId: 7,
          bodies: [],
          bodyEvents: [],
          changedBodies: [],
          deletedBodies: [],
          needsRepair: [{ refId: "op.input0", reason: "ambiguous" }],
        };
      }
    });
    const client = createTauriClient();
    await client.getSketch("sk");
    await client.getSketchRegions("sk");
    const seen: Array<{ error?: { kind: string; structural: boolean; evidence?: unknown[] } }> = [];
    const unsub = client.onPreviewResult((result) => seen.push(result));
    const session = await client.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 10 },
    });
    client.updatePreview(session.sessionId, { distance: 20 }, 1);
    await tick();

    expect(seen[0]?.error).toMatchObject({
      kind: "needsRepair",
      structural: true,
      evidence: [{ refId: "op.input0", reason: "ambiguous" }],
    });
    unsub();
  });

  it("keeps kernel diagnostics separate on a rejected exact preview", async () => {
    const diagnostics = [
      {
        severity: "error",
        code: "FILLET_WALKING_FAILED",
        message: "failed while advancing contour",
        stage: "build",
        evidence: { contour: { index: 1 } },
      },
    ];
    mockIPC((cmd) => {
      if (cmd === "get_sketch") {
        return {
          sketchId: "sk",
          plane: XZ_PLANE,
          entities: [],
          constraints: [],
          dof: 0,
          status: "FullyConstrained",
        };
      }
      if (cmd === "get_sketch_regions") return { regions: [PREVIEW_REGION] };
      if (cmd === "preview_op") {
        return Promise.reject({ kind: "opFailed", message: "fillet failed", diagnostics });
      }
    });
    const client = createTauriClient();
    await client.getSketch("sk");
    await client.getSketchRegions("sk");
    const seen: Array<{ error?: { diagnostics?: unknown[]; evidence?: unknown[] } }> = [];
    const unsub = client.onPreviewResult((result) => seen.push(result));
    const session = await client.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 10 },
    });
    client.updatePreview(session.sessionId, { distance: 20 }, 1);
    await tick();

    expect(seen[0]?.error?.diagnostics).toEqual(diagnostics);
    expect(seen[0]?.error?.evidence).toBeUndefined();
    unsub();
  });

  it("endPreview(commit) materializes the op through apply_edit_command", async () => {
    const cmds: string[] = [];
    mockIPC(
      (cmd) => {
        cmds.push(cmd);
        if (cmd === "get_sketch") {
          return {
            sketchId: "sk",
            plane: XZ_PLANE,
            entities: [],
            constraints: [],
            dof: 0,
            status: "FullyConstrained",
          };
        }
        if (cmd === "get_sketch_regions") return { regions: [PREVIEW_REGION] };
        if (cmd === "apply_edit_command") {
          setTimeout(() => {
            void emit("document-changed", {
              revision: 9,
              changedBodies: [{ bodyId: "b9", meshKey: "b9:coarse:9" }],
              removedBodies: [],
            });
            void emit("regen-finished", { revision: 9, sourceRevision: 8, outcome: "published" });
          }, 0);
          return readyProjection(8);
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300);
    const client = createTauriClient();
    await client.getSketch("sk");
    await client.getSketchRegions("sk");
    const session = await client.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 10 },
    });
    client.updatePreview(session.sessionId, { distance: 20 }, 1);
    const res = await client.endPreview(session.sessionId, true);
    expect(res).not.toBeNull();
    expect(cmds).toContain("apply_edit_command");
    expect(res?.changedBodies[0].bodyId).toBe("b9");
  });

  it("endPreview(cancel) resolves null without a command", async () => {
    const cmds: string[] = [];
    mockIPC((cmd) => {
      cmds.push(cmd);
      if (cmd === "get_sketch") {
        return {
          sketchId: "sk",
          plane: XZ_PLANE,
          entities: [],
          constraints: [],
          dof: 0,
          status: "FullyConstrained",
        };
      }
      if (cmd === "get_sketch_regions") return { regions: [PREVIEW_REGION] };
      return undefined;
    });
    const client = createTauriClient();
    await client.getSketch("sk");
    await client.getSketchRegions("sk");
    const session = await client.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 10 },
    });
    expect(await client.endPreview(session.sessionId, false)).toBeNull();
    expect(cmds).not.toContain("apply_edit_command");
  });
});

// ── Sketch drag gesture — latest-wins reconciliation ──────────────────────────

describe("tauriClient drag gesture (latest-wins)", () => {
  async function enteredClient(onDrag: () => unknown) {
    let beginArgs: { dragPoint?: string } | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return { sketchId: (payload as { sketchId: string }).sketchId, plane: XZ_PLANE, entities: [], constraints: [], dof: 4, status: "UnderConstrained" };
        if (cmd === "sketch_upsert")
          return { sketchId: (payload as { sketchId: string }).sketchId, sketchRevision: 1, dof: 3, status: "UnderConstrained", solvedPositions: {} };
        if (cmd === "begin_gesture") {
          beginArgs = payload as { dragPoint: string };
          return { gestureId: 7, ready: true };
        }
        if (cmd === "solve_drag") return onDrag();
        if (cmd === "end_gesture")
          // The worker keys solvedPositions by backend POINT UUID (the id begin
          // translated "e1.Start" to); the client reverse-maps it to "e1.Start".
          return {
            sketchId: "u",
            sketchRevision: 9,
            dof: 0,
            status: "FullyConstrained",
            solvedPositions: beginArgs?.dragPoint ? { [beginArgs.dragPoint]: [8, 8] } : {},
          };
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    // Create a Line so the point map has "e1.Start" for the drag-point translation.
    await client.sketchUpsert("sk", [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }], []);
    return { client, getBeginArgs: () => beginArgs };
  }

  const drag = (seq: number, superseded = false) => ({
    gestureId: 7,
    seq,
    status: superseded ? "superseded" : "success",
    dof: 1,
    conflicting: [] as string[],
    positions: superseded ? {} : { p: [seq, seq] },
    solveMicros: 5,
    superseded,
  });

  it("begins on a translated point id, drops stale + superseded seq, commits on end", async () => {
    let resp: unknown;
    const { client, getBeginArgs } = await enteredClient(() => resp);

    const begin = await client.beginGesture("sk", "e1.Start");
    expect(begin.ready).toBe(true);
    // The frontend "e1.Start" was translated to a minted point UUID (not passed raw).
    expect(getBeginArgs()?.dragPoint).not.toBe("e1.Start");
    expect(getBeginArgs()?.dragPoint).toMatch(/^[0-9a-f-]{36}$/);

    resp = drag(5);
    expect((await client.solveDrag([5, 5]))?.seq).toBe(5); // applied

    resp = drag(3);
    expect(await client.solveDrag([3, 3])).toBeNull(); // stale seq → dropped

    resp = drag(9, true);
    expect(await client.solveDrag([9, 9])).toBeNull(); // superseded flag → dropped

    resp = drag(8);
    expect((await client.solveDrag([8, 8]))?.seq).toBe(8); // newer seq → applied

    const end = await client.endGesture([8, 8]);
    expect(end.status).toBe("FullyConstrained");
    // F-WP9: the backend point UUID is reverse-mapped to the frontend point key.
    expect(end.solvedPositions?.["e1.Start"]).toEqual([8, 8]);
  });
});

// ── Conflict-id mapping (SCHEMA §7.4 `conflicting[]`) ─────────────────────────

describe("tauriClient conflicting-id mapping (backend uuid → frontend id)", () => {
  it("sketchUpsert maps the solve's conflicting uuids back to frontend constraint ids", async () => {
    let constraintUuid: string | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return { sketchId: (payload as { sketchId: string }).sketchId, plane: XZ_PLANE, entities: [], constraints: [], dof: 4, status: "UnderConstrained", conflicting: [] };
        if (cmd === "sketch_upsert") {
          const ops = (payload as { ops: { op: string; constraint?: { id: string } }[] }).ops ?? [];
          const addC = ops.find((o) => o.op === "addConstraint");
          if (addC?.constraint) constraintUuid = addC.constraint.id;
          // The worker reports the just-minted constraint uuid as conflicting.
          return { sketchId: (payload as { sketchId: string }).sketchId, sketchRevision: 1, dof: 0, status: "Conflicting", conflicting: constraintUuid ? [constraintUuid] : [], solvedPositions: {} };
        }
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    const up = await client.sketchUpsert(
      "sk",
      [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
      [{ id: "c1", type: "Horizontal", entities: ["e1"] }],
    );
    // The raw backend uuid was reverse-mapped to the FRONTEND constraint id "c1".
    expect(up.conflicting).toEqual(["c1"]);
    expect(up.conflicting).not.toContain(constraintUuid);
  });

  it("solveDrag maps conflicting uuids to frontend ids (no longer raw uuids)", async () => {
    let constraintUuid: string | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return { sketchId: (payload as { sketchId: string }).sketchId, plane: XZ_PLANE, entities: [], constraints: [], dof: 4, status: "UnderConstrained", conflicting: [] };
        if (cmd === "sketch_upsert") {
          const ops = (payload as { ops: { op: string; constraint?: { id: string } }[] }).ops ?? [];
          const addC = ops.find((o) => o.op === "addConstraint");
          if (addC?.constraint) constraintUuid = addC.constraint.id;
          return { sketchId: (payload as { sketchId: string }).sketchId, sketchRevision: 1, dof: 3, status: "UnderConstrained", conflicting: [], solvedPositions: {} };
        }
        if (cmd === "begin_gesture") return { gestureId: 7, ready: true };
        if (cmd === "solve_drag")
          return { gestureId: 7, seq: 1, status: "conflicting", dof: 1, conflicting: constraintUuid ? [constraintUuid] : [], positions: {}, solveMicros: 0, superseded: false };
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    await client.sketchUpsert(
      "sk",
      [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
      [{ id: "c1", type: "Horizontal", entities: ["e1"] }],
    );
    await client.beginGesture("sk", "e1.Start");
    const res = await client.solveDrag([5, 5]);
    expect(res?.conflicting).toEqual(["c1"]);
    expect(res?.conflicting).not.toContain(constraintUuid);
  });
});

// ── Promotion (pick → ElementId) ──────────────────────────────────────────────

describe("tauriClient promoteSelection", () => {
  it("prefixes the bodyId, marshals picks, returns minted element ids", async () => {
    let args: { snapshotId?: number; bodyId?: string; picks?: { topoKey: string }[] } | undefined;
    mockIPC((cmd, payload) => {
      if (cmd === "promote_selection") {
        args = payload as typeof args;
        return [{ topoKey: "f:2", elementId: "el_abc", kind: "face", bodyId: "body_uuid-1" }];
      }
    });
    const out = await createTauriClient().promoteSelection("uuid-1", [
      { topoKey: "f:2", anchor: { worldPoint: [1, 2, 3] } },
    ]);
    expect(out[0].elementId).toBe("el_abc");
    expect(args?.bodyId).toBe("body_uuid-1"); // bare uuid prefixed to the wire form
    expect(args?.snapshotId).toBe(0); // no regen published yet ⇒ default snapshot 0
    expect(args?.picks?.[0].topoKey).toBe("f:2");
  });

  it("forwards the published snapshotId from document-changed (not 0)", async () => {
    let args: { snapshotId?: number } | undefined;
    mockIPC((cmd, payload) => {
      if (cmd === "promote_selection") {
        args = payload as typeof args;
        return [{ topoKey: "f:2", elementId: "el_abc", kind: "face", bodyId: "body_uuid-1" }];
      }
    }, { shouldMockEvents: true });
    const client = createTauriClient();
    // Subscribing starts the lazy event listeners so document-changed is observed.
    const unsub = client.onDocumentChanged(() => {});
    await tick();

    // A regen publishes snapshot 5012 (the mesh the pick is scoped to).
    await emit("document-changed", {
      revision: 3,
      snapshotId: 5012,
      changedBodies: [{ bodyId: "b1", meshKey: "b1:coarse:3" }],
      removedBodies: [],
    });
    await tick();

    await client.promoteSelection("uuid-1", [{ topoKey: "f:2" }]);
    expect(args?.snapshotId).toBe(5012); // the REAL published snapshot, not 0
    unsub();
  });
});

describe("tauriClient prepareEdgeOp", () => {
  it("uses the published snapshot and preserves mutually-exclusive edge addresses", async () => {
    let args: Record<string, unknown> | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "prepare_edge_op") {
          args = payload as Record<string, unknown>;
          return {
            snapshotId: 5012,
            targetBodyId: "body_uuid-1",
            edges: [{ topoKey: "e:4", elementId: "el_4", bodyId: "body_uuid-1", kind: "edge", picked: true }],
            refusal: null,
          };
        }
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    const unsub = client.onDocumentChanged(() => {});
    await tick();
    await emit("document-changed", {
      revision: 3,
      snapshotId: 5012,
      changedBodies: [],
      removedBodies: [],
    });
    await tick();

    const result = await client.prepareEdgeOp({
      mode: "Fillet",
      chainTangentEdges: true,
      pickedEdges: [
        { bodyId: "uuid-1", topoKey: "e:4" },
        { elementId: "el_9" },
      ],
    });

    expect(result.edges[0].topoKey).toBe("e:4");
    expect(args?.snapshotId).toBe(5012);
    expect(args?.mode).toBe("Fillet");
    expect(args?.chainTangentEdges).toBe(true);
    expect(args?.pickedEdges).toEqual([
      { bodyId: "body_uuid-1", topoKey: "e:4", elementId: null },
      { bodyId: null, topoKey: null, elementId: "el_9" },
    ]);
    unsub();
  });

  it("rejects a prepared closure whose echoed snapshot is stale", async () => {
    mockIPC(
      (cmd) =>
        cmd === "prepare_edge_op"
          ? {
              snapshotId: 8,
              targetBodyId: "body_uuid-1",
              edges: [{ topoKey: "e:4", elementId: "el_4", bodyId: "body_uuid-1", kind: "edge", picked: true }],
              refusal: null,
            }
          : undefined,
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await expect(
      client.prepareEdgeOp({
        snapshotId: 7,
        mode: "Fillet",
        chainTangentEdges: true,
        pickedEdges: [{ bodyId: "uuid-1", topoKey: "e:4" }],
      }),
    ).rejects.toThrow("stale");
  });
});

describe("tauriClient prepareOffsetFace", () => {
  const request = {
    chainTangentFaces: true,
    distanceType: "Offset" as const,
    pickedFaces: [{ bodyId: "uuid-1", topoKey: "f:4" }],
  };
  const result = {
    snapshotId: 7,
    targetBodyId: "body_uuid-1",
    faces: [{ topoKey: "f:4", picked: true }],
    currentDims: {},
    refusal: null,
  };

  it("rejects a closure whose echoed snapshot differs from the admitted head", async () => {
    mockIPC(
      (cmd) => (cmd === "prepare_offset_face" ? { ...result, snapshotId: 8 } : undefined),
      { shouldMockEvents: true },
    );
    const client = createTauriClient();

    await expect(client.prepareOffsetFace({ ...request, snapshotId: 7 })).rejects.toThrow("stale");
  });

  it("rejects a once-valid closure when the published head moves while it waits", async () => {
    let resolve!: (value: typeof result) => void;
    mockIPC(
      (cmd) =>
        cmd === "prepare_offset_face"
          ? new Promise<typeof result>((r) => {
              resolve = r;
            })
          : undefined,
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    const unsub = client.onDocumentChanged(() => {});
    await tick();
    await emit("document-changed", { revision: 7, snapshotId: 7, changedBodies: [], removedBodies: [] });
    await tick();

    const pending = client.prepareOffsetFace(request);
    await tick();
    await emit("document-changed", { revision: 8, snapshotId: 8, changedBodies: [], removedBodies: [] });
    await tick();
    resolve(result);

    await expect(pending).rejects.toThrow("stale");
    unsub();
  });
});

// ── Projection hydration bridge (projection-updated → documentStore) ──────────

describe("tauriClient projection hydration", () => {
  it("hydrates documentStore from projection-updated and drops stale revisions", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });
    const client = createTauriClient();
    const seen: unknown[] = [];
    const unsub = client.onProjectionUpdated((p) => seen.push(p));
    await tick(); // let the lazy listen() register

    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 2 });
    await emit("projection-updated", {
      status: "ready",
      revision: 5,
      title: "Opened",
      dirty: true,
      bodies: { b1: { id: "b1", name: "B1", visible: true } },
      sketches: {},
      features: [{ id: "f1", kind: "extrude", label: "Extrude", valueText: "10.0 mm", status: "ok" }],
    });
    expect(documentStore.getState().revision).toBe(5);
    expect(documentStore.getState().bodies.b1.name).toBe("B1");
    expect(documentStore.getState().title).toBe("Opened");
    expect(seen).toHaveLength(1);

    // A stale (lower-revision) projection must NOT clobber the newer state.
    await emit("projection-updated", { status: "ready", revision: 3, title: "STALE", dirty: false, bodies: {}, sketches: {}, features: [] });
    expect(documentStore.getState().revision).toBe(5);
    expect(documentStore.getState().title).toBe("Opened");

    unsub();
  });
});

// ── getProjection (one-shot authoritative pull; missed-event race recovery) ───

describe("tauriClient getProjection", () => {
  it("invokes get_projection and hydrates documentStore through applyProjectionToStore", async () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 2 });
    const seen: string[] = [];
    mockIPC((cmd) => {
      seen.push(cmd);
      if (cmd === "get_projection") {
        return {
          status: "ready",
          revision: 6,
          title: "Recovered",
          dirty: false,
          bodies: { b1: { id: "b1", name: "B1", visible: true } },
          sketches: {},
          features: [],
        };
      }
    });
    const client = createTauriClient();
    const p = await client.getProjection();
    expect(seen).toContain("get_projection");
    expect(p.revision).toBe(6);
    expect(documentStore.getState().revision).toBe(6);
    expect(documentStore.getState().title).toBe("Recovered");
    expect(documentStore.getState().bodies.b1?.name).toBe("B1");
  });

  it("drops a stale (lower-revision) projection without clobbering newer state", async () => {
    documentStore.getState().applySnapshot({ ...seedMockDocument(), revision: 9 });
    mockIPC((cmd) =>
      cmd === "get_projection"
        ? { status: "ready", revision: 3, title: "STALE", dirty: false, bodies: {}, sketches: {}, features: [] }
        : undefined,
    );
    await createTauriClient().getProjection();
    expect(documentStore.getState().revision).toBe(9);
  });
});

// ── regen-finished correlation (prompt, no 8 s wait) ──────────────────────────

describe("tauriClient regen-finished correlation", () => {
  const op: OperationOp = {
    opType: "Boolean",
    params: { operation: "Union", targetBodyId: "t", toolBodyId: "u" },
  } as OperationOp;

  it("resolves a no-geometry edit from regen-finished (not the timeout)", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "apply_edit_command") {
          setTimeout(() => void emit("regen-finished", { revision: 12, outcome: "noop" }), 0);
          return readyProjection(11);
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(5000); // long: regen-finished must win, not the timeout
    const res = await createTauriClient().applyOperation(op);
    expect(res.revision).toBe(12); // from regen-finished, not the pre-regen 11
    expect(res.changedBodies).toEqual([]); // noop → no body delta
    expect(res.terminal).toBe("noop");
  });

  it("IGNORES a superseded regen-finished and resolves off the covering publish (rapid-commit)", async () => {
    // A stale superseded report (sourceRevision below our commit's R) must NOT
    // resolve the awaiter with empty bodies — the reported cross-commit bug. The
    // from-0 regen that covers R (source >= R) publishes the body via document-changed
    // and settles the commit on its regen-finished sibling.
    mockIPC(
      (cmd) => {
        if (cmd === "apply_edit_command") {
          setTimeout(() => {
            void emit("regen-finished", { revision: 8, sourceRevision: 3, outcome: "superseded" });
            void emit("document-changed", {
              revision: 8,
              changedBodies: [{ bodyId: "b8", meshKey: "b8:coarse:8" }],
              removedBodies: [],
            });
            void emit("regen-finished", { revision: 8, sourceRevision: 8, outcome: "published" });
          }, 0);
          return readyProjection(5); // R = 5
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300);
    const res = await createTauriClient().applyOperation(op);
    expect(res.revision).toBe(8); // resolved off the covering publish, not the superseded report
    expect(res.changedBodies).toEqual([{ bodyId: "b8", meshKey: "b8:coarse:8" }]);
    expect(res.errorMessage).toBeUndefined();
  });

  it("resolves a failed regen-finished as a failure carrying the worker message", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "apply_edit_command") {
          setTimeout(
            () =>
              void emit("regen-finished", {
                revision: 12,
                sourceRevision: 12,
                outcome: "failed",
                message: "worker crashed: boom",
                diagnostics: [
                  {
                    severity: "error",
                    code: "FILLET_WALKING_FAILED",
                    message: "worker crashed: boom",
                    stage: "build",
                  },
                ],
              }),
            0,
          );
          return readyProjection(11); // R = 11
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300);
    const res = await createTauriClient().applyOperation(op);
    expect(res.revision).toBe(12);
    expect(res.changedBodies).toEqual([]); // failure → empty bodies (no throw)
    expect(res.errorMessage).toBe("worker crashed: boom");
    expect(res.diagnostics?.[0]?.code).toBe("FILLET_WALKING_FAILED");
    expect(res.terminal).toBe("failed");
  });

  it("does NOT resolve off a regen-finished whose sourceRevision is below the commit's R", async () => {
    // A failed report for an EARLIER commit (sourceRevision < R) must be ignored;
    // the awaiter falls through to the safety timeout (null → pre-regen projection).
    mockIPC(
      (cmd) => {
        if (cmd === "apply_edit_command") {
          setTimeout(
            () => void emit("regen-finished", { revision: 4, sourceRevision: 2, outcome: "failed", message: "old" }),
            0,
          );
          return readyProjection(7); // R = 7 > sourceRevision 2
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(40);
    const res = await createTauriClient().applyOperation(op);
    expect(res.revision).toBe(7); // fell back to the pre-regen projection revision
    expect(res.changedBodies).toEqual([]);
    expect(res.errorMessage).toBeUndefined(); // the stale failure never resolved us
  });

  it("a published regen whose failedSteps names THIS op's recordId resolves as FAILURE, not success (finding 1)", async () => {
    // The regen PUBLISHES other records' bodies (document-changed) but THIS op's step
    // errored — the awaiter must correlate to the regen-finished sibling and fail with
    // empty bodies + the step message, NOT settle success off the document-changed.
    let recordId: string | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          recordId = (payload as { command: { record: { recordId: string } } }).command.record.recordId;
          setTimeout(() => {
            void emit("document-changed", {
              revision: 10,
              changedBodies: [{ bodyId: "other", meshKey: "other#10" }], // OTHER records' body
              removedBodies: [],
            });
            void emit("regen-finished", {
              revision: 10,
              sourceRevision: 9,
              outcome: "published",
              failedSteps: [
                {
                  recordId: recordId as string,
                  message: "extrude self-intersects",
                  diagnostics: [
                    {
                      severity: "error",
                      code: "FILLET_WALKING_FAILED",
                      message: "extrude self-intersects",
                    },
                  ],
                },
              ],
            });
          }, 0);
          return readyProjection(9); // R = 9
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300);
    const res = await createTauriClient().applyOperation(op);
    expect(res.changedBodies).toEqual([]); // NOT the published "other" body
    expect(res.errorMessage).toBe("extrude self-intersects");
    expect(res.diagnostics?.[0]?.code).toBe("FILLET_WALKING_FAILED");
    expect(res.revision).toBe(10);
  });

  it("scopes changedBodies to affectedBodies[recordId] — the op's own bodies + split children (finding 2)", async () => {
    let recordId: string | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          recordId = (payload as { command: { record: { recordId: string } } }).command.record.recordId;
          setTimeout(() => {
            void emit("document-changed", {
              revision: 12,
              changedBodies: [
                { bodyId: "other", meshKey: "other#12" }, // a sibling record's body
                { bodyId: "mine:0", meshKey: "mine0#12" },
                { bodyId: "mine:1", meshKey: "mine1#12" }, // split child
              ],
              removedBodies: [],
            });
            void emit("regen-finished", {
              revision: 12,
              sourceRevision: 11,
              outcome: "published",
              affectedBodies: { [recordId as string]: ["mine:0", "mine:1"] },
            });
          }, 0);
          return readyProjection(11);
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300);
    const res = await createTauriClient().applyOperation(op);
    // ONLY this op's bodies, mapped to their document-changed meshKeys (not "other").
    expect(res.changedBodies).toEqual([
      { bodyId: "mine:0", meshKey: "mine0#12" },
      { bodyId: "mine:1", meshKey: "mine1#12" },
    ]);
    expect(res.errorMessage).toBeUndefined();
  });

  it("resolves immediately (no 8s stall) when a fresh op lands PAST the cursor — the positional trap fallback (finding 4 / H7a)", async () => {
    // A tail draft (idx >= appliedOps) fires NO regen events. This is the
    // POSITIONAL check (H7a): the record itself must sit at/beyond appliedOps in
    // `features`, not merely "some appliedOps<totalOps gap exists somewhere" —
    // a count-only check would also misfire on a legitimate mid-history insert
    // that coexists with OTHER unrelated drafts (see the sibling test below).
    let recordId = "";
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          recordId = (payload as { command: { record: { recordId: string } } }).command.record.recordId;
          return {
            ...(readyProjection(5, [{ id: "f0" }, { id: "f1" }, { id: recordId }]) as object),
            appliedOps: 2,
            totalOps: 3,
          };
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(5000); // long: the immediate resolve must win, not the timeout
    const start = Date.now();
    const res = await createTauriClient().applyOperation(op);
    expect(Date.now() - start).toBeLessThan(1000); // did not wait on the safety timeout
    expect(res.changedBodies).toEqual([]);
    expect(res.errorMessage).toMatch(/rolled back/);
    expect(res.revision).toBe(5);
  });

  it("does NOT treat a mid-history insert as inert (idx < appliedOps) and hints where it landed (H7a)", async () => {
    // The H7a-fixed record lands INSIDE the applied prefix (idx=2 < appliedOps=3)
    // even though an UNRELATED pre-existing draft still sits beyond the cursor
    // (totalOps=4) — a count-only `appliedOps < totalOps` check would wrongly
    // flag this commit as inert; the positional check must not.
    let recordId = "";
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") {
          recordId = (payload as { command: { record: { recordId: string } } }).command.record.recordId;
          return {
            ...(readyProjection(5, [{ id: "f0" }, { id: "f1" }, { id: recordId }, { id: "unrelated-draft" }]) as object),
            appliedOps: 3,
            totalOps: 4,
          };
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300); // short: no regen-finished/document-changed follows — must not error
    const res = await createTauriClient().applyOperation(op);
    expect(res.errorMessage).toBeUndefined();
    expect(res.revision).toBe(5);
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Inserted at step 3 of 4 — roll to end to rebuild later features",
    );
  });
});

// ── Sticky rebuild-failure hint lifecycle (SAVE/OPEN-CONNECT-RACE rider) ──────

describe("tauriClient sticky rebuild-failure hint", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));

  /** Client + the UNCORRELATED failure (no commit awaiter — the open-replay shape). */
  async function raiseFailedHint(): Promise<void> {
    mockIPC(() => undefined, { shouldMockEvents: true });
    createTauriClient();
    await flush(); // ensureEvents is eager; listeners registered
    await emit("regen-finished", {
      revision: 1,
      outcome: "failed",
      message: "worker not connected: restarting or failed",
    });
    await flush();
    const hint = viewportStore.getState().statusHint;
    expect(hint).toMatchObject({ severity: "error", sticky: true });
    expect(hint?.message).toContain("Geometry rebuild failed — ");
  }

  it("a later CLEAN publish clears the hint", async () => {
    await raiseFailedHint();
    await emit("regen-finished", { revision: 2, outcome: "published" });
    await flush();
    expect(viewportStore.getState().statusHint).toBeNull();
  });

  it("superseded / cancelled / noop retain the hint — nothing was rebuilt", async () => {
    await raiseFailedHint();
    for (const outcome of ["superseded", "cancelled", "noop"]) {
      await emit("regen-finished", { revision: 3, outcome });
      await flush();
      expect(viewportStore.getState().statusHint?.message, outcome).toContain(
        "Geometry rebuild failed — ",
      );
    }
  });

  it("a publish never clears an unrelated sticky hint", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });
    createTauriClient();
    await flush();
    viewportStore.getState().setStatusHint("Isolation on — Esc or ⇧I to exit", { sticky: true });
    await emit("regen-finished", { revision: 2, outcome: "published" });
    await flush();
    expect(viewportStore.getState().statusHint?.message).toContain("Isolation");
  });
});

// ── Correlation reset on document replacement (finding 3) ─────────────────────

describe("tauriClient correlation reset on document replacement", () => {
  const op: OperationOp = {
    opType: "Boolean",
    params: { operation: "Union", targetBodyId: "t", toolBodyId: "u" },
  } as OperationOp;

  it("open(docB) drops docA's buffered publish so docB's first commit can't settle off it", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "open_document") return { documentId: "docB", title: "B" };
        if (cmd === "apply_edit_command") {
          // docB's first commit is a NOOP (only a regen-finished, no document-changed):
          // its success bodies must come from nothing, not docA's stale buffer.
          setTimeout(() => void emit("regen-finished", { revision: 2, sourceRevision: 1, outcome: "noop" }), 0);
          return readyProjection(1); // docB R = 1
        }
      },
      { shouldMockEvents: true },
    );
    __setRegenTimeoutForTests(300);
    const client = createTauriClient();
    const unsub = client.onDocumentChanged(() => {});
    await tick(); // start the listeners
    // Buffer a HIGH-revision docA publish.
    await emit("document-changed", {
      revision: 999,
      changedBodies: [{ bodyId: "b_docA", meshKey: "bA#999" }],
      removedBodies: [],
    });
    await tick();
    await client.openDocument("/docB.onecad"); // resets the correlation buffers

    // Without the reset, setTarget(1) would buffer docA's rev-999 change and the noop
    // regen-finished would settle docB's commit with docA's body.
    const res = await client.applyOperation(op);
    expect(res.changedBodies).toEqual([]);
    unsub();
  });
});

// ── getSketch feeds the preview lane the plane (MODEL-HARDEN W0.5) ─────────────

describe("tauriClient getSketch preview-lane plane seam", () => {
  it("caches the get_sketch plane into the local preview lane", async () => {
    let capturedLane: ReturnType<typeof localSolverModule.createLocalSolverLane> | undefined;
    const realCreate = localSolverModule.createLocalSolverLane;
    const spy = vi
      .spyOn(localSolverModule, "createLocalSolverLane")
      .mockImplementation((deps) => {
        const lane = realCreate(deps);
        vi.spyOn(lane, "cacheSketchPlane");
        capturedLane = lane;
        return lane;
      });
    mockIPC((cmd, payload) => {
      if (cmd === "get_sketch")
        return {
          sketchId: (payload as { sketchId: string }).sketchId,
          plane: XZ_PLANE,
          entities: [],
          constraints: [],
          dof: 0,
          status: "FullyConstrained",
        };
    });
    const client = createTauriClient();
    await client.getSketch("sk-plane");
    expect(capturedLane?.cacheSketchPlane).toHaveBeenCalledWith(
      "sk-plane",
      expect.objectContaining({ kind: "XZ" }),
    );
    spy.mockRestore();
  });
});

// ── sketch-solved event + sketch error path ───────────────────────────────────

describe("tauriClient sketch-solved + errors", () => {
  it("receives the sketch-solved event (mirrors the solve result)", async () => {
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return { sketchId: (payload as { sketchId: string }).sketchId, plane: XZ_PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    await emit("sketch-solved", { sketchId: "u", sketchRevision: 3, dof: 1, status: "UnderConstrained", solvedPositions: {} });
    await tick();
    expect(__lastSketchSolvedForTests()?.sketchRevision).toBe(3);
  });

  it("surfaces a rejected sketch_upsert as an Error with the ApiError kind", async () => {
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return { sketchId: (payload as { sketchId: string }).sketchId, plane: XZ_PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
        if (cmd === "sketch_upsert") return Promise.reject({ kind: "opFailed", message: "solve boom" });
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    await expect(
      client.sketchUpsert("sk", [{ id: "e1", type: "Line", p0: [0, 0], p1: [1, 0] }], []),
    ).rejects.toThrow(/opFailed: solve boom/);
  });
});

// ── Transactional id-map: a rejected upsert leaves the live map untouched ──────

describe("tauriClient sketchUpsert transactional id-map", () => {
  type Op = { op: string; entity?: { kind: string } };
  const points = (ops: Op[]): Op[] => ops.filter((o) => o.op === "addEntity" && o.entity?.kind === "point");

  it("re-emits the full add ops after a rejected upsert (map not mutated), then dedups once committed", async () => {
    const seenOps: Op[][] = [];
    let failNext = true;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return { sketchId: (payload as { sketchId: string }).sketchId, plane: XZ_PLANE, entities: [], constraints: [], dof: 4, status: "UnderConstrained" };
        if (cmd === "sketch_upsert") {
          seenOps.push((payload as { ops: Op[] }).ops);
          if (failNext) {
            failNext = false;
            return Promise.reject({ kind: "opFailed", message: "solve boom" });
          }
          return { sketchId: (payload as { sketchId: string }).sketchId, sketchRevision: 1, dof: 3, status: "UnderConstrained", solvedPositions: {} };
        }
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    const line = [{ id: "e1", type: "Line" as const, p0: [0, 0] as [number, number], p1: [40, 0] as [number, number] }];

    // 1) REJECTED upsert of the line.
    await expect(client.sketchUpsert("sk", line, [])).rejects.toThrow(/opFailed/);
    // 2) The SAME line again SUCCEEDS — because the rejected call left the id-map
    //    untouched, this still marshals the full add ops (2 synth points + line).
    await client.sketchUpsert("sk", line, []);
    // 3) A THIRD upsert of the SAME line now diffs to ZERO adds — the clone committed.
    await client.sketchUpsert("sk", line, []);

    expect(seenOps).toHaveLength(3);
    expect(points(seenOps[0])).toHaveLength(2); // rejected attempt marshalled 2 points
    expect(points(seenOps[1])).toHaveLength(2); // NOT deduped → map untouched by (1)
    expect(seenOps[2]).toEqual([]); // committed after (2) → no re-add
  });
});

// ── Metadata-only EditCommands: no regen ⇒ no correlation awaiter ─────────────
//
// `SetVisibility` / `RenameBody` / `RenameSketch` are `RegenHint::None` on the Rust
// side (edit/session.rs), so `apply_edit_command` publishes a projection and NOTHING
// else ever arrives. Routed through the normal correlated `applyEdit`, every eye
// click would sit out the full 8 s safety timeout before resolving. These pin that
// they settle off the command's own projection, with no timer advanced and no event
// emitted. A mock-client test could not see this at all — the mock resolves
// unconditionally (a false green).

describe("tauriClient metadata-only edit commands", () => {
  afterEach(() => {
    vi.useRealTimers();
    __setRegenTimeoutForTests(8000);
  });

  /** Settle-without-regen probe: resolves only if no awaiter is registered. */
  async function settlesWithoutRegen(command: unknown): Promise<{
    settled: boolean;
    result: unknown;
  }> {
    const client = createTauriClient();
    vi.useFakeTimers();
    let settled = false;
    const promise = client.applyEditCommand(command as never).then((r) => {
      settled = true;
      return r;
    });
    // Flush microtasks ONLY — no timer fires, and no document-changed /
    // regen-finished is emitted, so an awaiter-backed path stays pending here.
    await vi.advanceTimersByTimeAsync(0);
    return { settled, result: settled ? await promise : null };
  }

  it("setVisibility resolves off the command projection alone (no awaiter, no 8s stall)", async () => {
    const seen: unknown[] = [];
    mockIPC((cmd, payload) => {
      if (cmd === "apply_edit_command") {
        seen.push((payload as { command: unknown }).command);
        return readyProjection(7, [
          { id: "f1", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "5.0 mm", status: "ok" },
        ]);
      }
    });
    __setRegenTimeoutForTests(60_000); // any awaiter would visibly stall

    const { settled, result } = await settlesWithoutRegen({
      cmd: "setVisibility",
      target: { body: "b-uuid" },
      visible: false,
    });
    expect(settled).toBe(true);
    expect(result).toEqual({
      revision: 7,
      changedBodies: [],
      removedBodies: [],
      features: [
        { id: "f1", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "5.0 mm", status: "ok" },
      ],
      opLabel: "Hide",
    });
    expect(seen[0]).toEqual({ cmd: "setVisibility", target: { body: "b-uuid" }, visible: false });
  });

  it("renameBody / renameSketch take the same metadata-only transport", async () => {
    mockIPC((cmd) => (cmd === "apply_edit_command" ? readyProjection(3) : undefined));
    __setRegenTimeoutForTests(60_000);
    const a = await settlesWithoutRegen({ cmd: "renameBody", body: "b", name: "Housing" });
    expect(a.settled).toBe(true);
    vi.useRealTimers();
    const b = await settlesWithoutRegen({ cmd: "renameSketch", sketch: "s", name: "Profile" });
    expect(b.settled).toBe(true);
  });

  it("a TIMELINE command still registers the awaiter (the metadata set is not a blanket bypass)", async () => {
    mockIPC((cmd) => (cmd === "apply_edit_command" ? readyProjection(3) : undefined), {
      shouldMockEvents: true,
    });
    __setRegenTimeoutForTests(60_000);
    // removeOperation dirties the timeline (RegenHint::ToEnd) — it MUST wait for the
    // correlated regen, so it is still pending at this point.
    const { settled } = await settlesWithoutRegen({ cmd: "removeOperation", record: "f3" });
    expect(settled).toBe(false);
  });
});

// ── SP-2 W4: gesture target kinds + the `curves` channel (SCHEMA §7.4) ────────

describe("tauriClient gesture targets + curves channel", () => {
  type BeginPayload = { sketchId: string; dragPoint: string; target?: Record<string, unknown> };
  type UpsertOp = { op: string; entity?: { kind: string; id: string } };

  /** Enter a sketch holding a Line, a Circle and an Arc, and expose the minted
   *  backend uuids (read off the upsert ops — the id-map is private). */
  async function sketchWithCurves(drag?: () => unknown, end?: () => unknown) {
    const minted = new Map<string, string>();
    let begin: BeginPayload | undefined;
    mockIPC(
      (cmd, payload) => {
        if (cmd === "apply_edit_command") return readyProjection(1);
        if (cmd === "enter_sketch")
          return { sketchId: (payload as { sketchId: string }).sketchId, plane: XZ_PLANE, entities: [], constraints: [], dof: 4, status: "UnderConstrained" };
        if (cmd === "sketch_upsert") {
          // addEntity order per marshalUpsert: the synthesized points, then the
          // owning entity — so the LAST id of each kind is the entity's own.
          for (const o of (payload as { ops: UpsertOp[] }).ops ?? []) {
            if (o.op === "addEntity" && o.entity) minted.set(o.entity.kind, o.entity.id);
          }
          return { sketchId: "u", sketchRevision: 1, dof: 3, status: "UnderConstrained", solvedPositions: {}, solvedCurves: {} };
        }
        if (cmd === "begin_gesture") {
          begin = payload as BeginPayload;
          return { gestureId: 7, ready: true };
        }
        if (cmd === "solve_drag") return drag?.();
        if (cmd === "end_gesture") return end?.();
      },
      { shouldMockEvents: true },
    );
    const client = createTauriClient();
    await client.enterSketch({ newOnPlane: "XZ", sketchId: "sk" });
    await client.sketchUpsert(
      "sk",
      [
        { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
        { id: "c1", type: "Circle", center: [0, 20], radius: 5 },
        { id: "a1", type: "Arc", center: [60, 0], radius: 4, start: [64, 0], end: [60, 4] },
      ],
      [],
    );
    return { client, minted, getBegin: () => begin };
  }

  it("omits `target` entirely when none is given (pre-SP-2 request unchanged)", async () => {
    const { client, getBegin } = await sketchWithCurves();
    await client.beginGesture("sk", "e1.Start");
    expect(getBegin()).toBeDefined();
    expect("target" in getBegin()!).toBe(false);
  });

  it("marshals entityBody/radius/arcEnd against the ENTITY uuid", async () => {
    const { client, minted, getBegin } = await sketchWithCurves();

    await client.beginGesture("sk", "e1.Start", { kind: "entityBody", entityId: "e1", grab: [1, 2] });
    expect(getBegin()!.target).toEqual({ kind: "entityBody", entity: minted.get("line"), grab: [1, 2] });

    await client.beginGesture("sk", "c1.Center", { kind: "radius", entityId: "c1", grab: [3, 4] });
    expect(getBegin()!.target).toEqual({ kind: "radius", entity: minted.get("circle"), grab: [3, 4] });

    await client.beginGesture("sk", "a1.End", { kind: "arcEnd", entityId: "a1", role: "End" });
    expect(getBegin()!.target).toEqual({ kind: "arcEnd", entity: minted.get("arc"), role: "End" });
    // No `grab` key at all when the caller omits it (the kind ignores it anyway).
    expect("grab" in getBegin()!.target!).toBe(false);
    // Every kind is a real uuid, never a frontend id.
    expect(minted.get("arc")).toMatch(/^[0-9a-f-]{36}$/);
  });

  it("a `point` target uses the POINT ref ladder (dragPoint's own mapping)", async () => {
    const { client, getBegin } = await sketchWithCurves();
    await client.beginGesture("sk", "e1.Start", { kind: "point", entityId: "e1.Start" });
    const { target, dragPoint } = getBegin()!;
    expect(target).toEqual({ kind: "point", entity: dragPoint });
    expect(dragPoint).toMatch(/^[0-9a-f-]{36}$/);
  });

  it("THROWS on an unmapped entity rather than sending a non-uuid", async () => {
    const { client, getBegin } = await sketchWithCurves();
    await expect(
      client.beginGesture("sk", "e1.Start", { kind: "radius", entityId: "ghost" }),
    ).rejects.toThrow(/radius target ghost is not a mapped sketch entity/);
    // The command never fired — no half-open gesture was armed.
    expect(getBegin()).toBeUndefined();
  });

  it("reverse-maps `curves` on BOTH solveDrag and endGesture", async () => {
    const uuids = new Map<string, string>();
    const { client, minted } = await sketchWithCurves(
      () => ({
        gestureId: 7,
        seq: 1,
        status: "success",
        dof: 1,
        conflicting: [],
        positions: {},
        curves: { [uuids.get("circle")!]: { radius: 12.5 } },
        solveMicros: 3,
        superseded: false,
      }),
      () => ({
        sketchId: "u",
        sketchRevision: 9,
        dof: 0,
        status: "FullyConstrained",
        solvedPositions: {},
        solvedCurves: { [uuids.get("arc")!]: { radius: 7, startAngle: 0.25, endAngle: 1.5 } },
      }),
    );
    for (const [k, v] of minted) uuids.set(k, v);

    await client.beginGesture("sk", "c1.Center", { kind: "radius", entityId: "c1" });
    const step = await client.solveDrag([9, 9]);
    // Keyed by the FRONTEND id "c1" — a `radius` drag reports NO position at all.
    expect(step?.curves).toEqual({ c1: { radius: 12.5 } });
    expect(step?.positions).toEqual({});

    const done = await client.endGesture([9, 9]);
    expect(done.solvedCurves).toEqual({ a1: { radius: 7, startAngle: 0.25, endAngle: 1.5 } });
  });

  it("curves default to {} when the backend omits the key", async () => {
    const { client } = await sketchWithCurves(
      () => ({ gestureId: 7, seq: 1, status: "success", dof: 1, conflicting: [], positions: {}, solveMicros: 0, superseded: false }),
      () => ({ sketchId: "u", sketchRevision: 2, dof: 0, status: "FullyConstrained", solvedPositions: {} }),
    );
    await client.beginGesture("sk", "e1.Start");
    expect((await client.solveDrag([1, 1]))?.curves).toEqual({});
    expect((await client.endGesture())?.solvedCurves).toEqual({});
  });
});
