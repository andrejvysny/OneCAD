/*
 * SKETCH-ON-FACE R7 — the projected boundary SURVIVES a finish + re-entry + edit,
 * full frontend stack over the REAL tauriClient marshaller.
 *
 * This is the sketch-on-face sibling of `sketchMultiObject.test.ts`, and it exists
 * for the same reason: the mock lane's `sketchUpsert` full-REPLACES the session,
 * so it can never see the id-map diff that `marshalUpsert` emits — and the id-map
 * diff is exactly where locked geometry gets destroyed. Two distinct ways it can
 * happen, both pinned here:
 *
 *   1. a stale id-map after re-entry makes the first edit read every hydrated
 *      entity as "mapped but absent" and emit a `removeEntity` for it
 *      (`seedIdMapFromWire`'s REBASE), and
 *   2. even with a correct map, a `removeEntity` on a `referenceLocked` entity is
 *      the one op the backend refuses outright — failing the WHOLE batch, so the
 *      user's own edit is lost too (`marshalUpsert`'s reference-lock latch).
 *
 * The harness is a faithful miniature of the Rust lane: `add_sketch_on_face`
 * stores a sketch already seeded with the projected boundary (locked points +
 * locked lines + one `Fixed` per point, mirroring
 * `onecad-core::sketch::projected_sketch_content`), and it REJECTS any op that
 * would mutate locked geometry, exactly as `SketchError::ReferenceLocked` does.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { createTauriClient } from "./tauriClient";
import { lookupMockFace } from "./mockFaceGeometry";
import { SketchController } from "@/tools/sketch/SketchController";
import { flushSketchMutations } from "@/tools/sketch/sketchService";
import type { CadClient } from "./client";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { toolStore, type SketchTool } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { selectionStore } from "@/stores/selectionStore";
import { settingsStore } from "@/stores/settingsStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

// ── A faithful miniature of the Rust sketch lane (host-face flavour) ─────────

type TypedEntity =
  | { kind: "point"; id: string; at: [number, number]; referenceLocked?: boolean }
  | { kind: "line"; id: string; start: string; end: string; referenceLocked?: boolean };
type TypedConstraint = { kind: string; id: string; [k: string]: unknown };
type SketchEditOpWire =
  | { op: "addEntity"; entity: { id: string; [k: string]: unknown } }
  | { op: "removeEntity"; entity: string }
  | { op: "addConstraint"; constraint: TypedConstraint }
  | { op: "removeConstraint"; constraint: string }
  | { op: "setEntityConstruction"; entity: string; construction: boolean }
  | { op: "setEntityPositions"; positions: [string, [number, number]][] }
  | { op: string; [k: string]: unknown };

interface StoredSketch {
  id: string;
  plane: Record<string, unknown>;
  entities: TypedEntity[];
  constraints: TypedConstraint[];
}

/** The frame + boundary the mock lane's analytic table reports for the box top. */
function topFace(): { plane: Record<string, unknown>; corners: [number, number][] } {
  const found = lookupMockFace("body1", undefined, "f:4");
  if (found.kind !== "planar" || found.geometry.boundary.kind !== "polygon") {
    throw new Error("expected the box top face to be a planar polygon");
  }
  const { origin, xAxis, yAxis, normal } = found.geometry.plane;
  return { plane: { kind: "custom", origin, xAxis, yAxis, normal }, corners: found.geometry.boundary.corners };
}

/** `projected_sketch_content`: one locked Point per boundary point (each pinned by
 *  a `Fixed`), then the locked curves referencing them. */
function seedProjectedBoundary(sketch: StoredSketch, corners: [number, number][]): void {
  const ids = corners.map((at, i) => {
    const id = `rp${i}`;
    sketch.entities.push({ kind: "point", id, at, referenceLocked: true });
    sketch.constraints.push({ kind: "fixed", id: `rc${i}`, point: id, at });
    return id;
  });
  ids.forEach((id, i) => {
    sketch.entities.push({
      kind: "line",
      id: `rl${i}`,
      start: id,
      end: ids[(i + 1) % ids.length],
      referenceLocked: true,
    });
  });
}

const LOCKED_OPS = new Set(["removeEntity", "setEntityConstruction", "setEntityPositions"]);

/** Rust `SketchError::ReferenceLocked` — a geometry-mutating op on locked
 *  geometry is refused, and the refusal fails the WHOLE upsert batch. */
function assertNotLocked(sketch: StoredSketch, ops: SketchEditOpWire[]): void {
  const locked = new Set(sketch.entities.filter((e) => e.referenceLocked).map((e) => e.id));
  for (const op of ops) {
    if (!LOCKED_OPS.has(op.op)) continue;
    const targets =
      op.op === "setEntityPositions"
        ? ((op as { positions: [string, [number, number]][] }).positions ?? []).map(([id]) => id)
        : [String((op as { entity: string }).entity)];
    for (const t of targets) {
      if (locked.has(t)) throw new Error(`referenceLocked: ${op.op} on ${t}`);
    }
  }
}

function entityRefs(e: TypedEntity): string[] {
  return e.kind === "line" ? [e.start, e.end] : [];
}

function cascadeRemove(sketch: StoredSketch, seed: string): void {
  const removed = new Set([seed]);
  for (;;) {
    const next = sketch.entities.filter((e) => !removed.has(e.id) && entityRefs(e).some((r) => removed.has(r)));
    if (next.length === 0) break;
    for (const e of next) removed.add(e.id);
  }
  sketch.entities = sketch.entities.filter((e) => !removed.has(e.id));
  sketch.constraints = sketch.constraints.filter(
    (c) => !Object.values(c).some((v) => typeof v === "string" && removed.has(v)),
  );
}

function applyOps(sketch: StoredSketch, ops: SketchEditOpWire[]): void {
  assertNotLocked(sketch, ops);
  for (const op of ops) {
    switch (op.op) {
      case "addEntity": {
        const e = (op as { entity: Record<string, unknown> }).entity;
        const id = String(e.id);
        if (sketch.entities.some((x) => x.id === id)) throw new Error(`duplicate entity id ${id}`);
        if (e.kind === "line") {
          const [start, end] = [String(e.start), String(e.end)];
          for (const ref of [start, end]) {
            if (!sketch.entities.some((x) => x.id === ref)) throw new Error(`dangling ref ${ref}`);
          }
          sketch.entities.push({ kind: "line", id, start, end });
        } else if (e.kind === "point") {
          sketch.entities.push({ kind: "point", id, at: e.at as [number, number] });
        }
        break;
      }
      case "removeEntity":
        cascadeRemove(sketch, String((op as { entity: string }).entity));
        break;
      case "addConstraint":
        sketch.constraints.push((op as { constraint: TypedConstraint }).constraint);
        break;
      case "removeConstraint": {
        const id = String((op as { constraint: string }).constraint);
        sketch.constraints = sketch.constraints.filter((c) => c.id !== id);
        break;
      }
      default:
        break;
    }
  }
}

/** Rust `worker::wire::wire_entity` / `wire_constraint`. */
function toWireEntities(sketch: StoredSketch): unknown[] {
  return sketch.entities.map((e) =>
    e.kind === "point"
      ? { id: e.id, type: "Point", at: e.at, construction: false, ...(e.referenceLocked ? { referenceLocked: true } : {}) }
      : {
          id: e.id,
          type: "Line",
          p0Ref: e.start,
          p1Ref: e.end,
          construction: false,
          ...(e.referenceLocked ? { referenceLocked: true } : {}),
        },
  );
}
function toWireConstraints(sketch: StoredSketch): unknown[] {
  return sketch.constraints.map((c) =>
    c.kind === "fixed"
      ? { id: c.id, type: "Fixed", entities: [c.point] }
      : { id: c.id, type: "Coincident", entities: [c.point1, c.point2].filter(Boolean) },
  );
}

interface Backend {
  sketches: Map<string, StoredSketch>;
  upsertOps: SketchEditOpWire[][];
  onFaceCalls: Record<string, unknown>[];
  upsertErrors: string[];
}

function installBackend(): Backend {
  const backend: Backend = { sketches: new Map(), upsertOps: [], onFaceCalls: [], upsertErrors: [] };
  const face = topFace();
  const session = (id: string): StoredSketch => {
    const s = backend.sketches.get(id);
    if (!s) throw new Error(`unknown sketch ${id}`);
    return s;
  };
  const sessionDto = (id: string): unknown => {
    const s = session(id);
    return {
      sketchId: id,
      plane: s.plane,
      entities: toWireEntities(s),
      constraints: toWireConstraints(s),
      dof: 0,
      status: "FullyConstrained",
      conflicting: [],
    };
  };
  mockIPC((cmd, payload) => {
    const p = (payload ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "add_sketch_on_face": {
        backend.onFaceCalls.push(p);
        // The FE-minted id is adopted VERBATIM as the document SketchId.
        const id = String(p.sketchId);
        const sketch: StoredSketch = { id, plane: face.plane, entities: [], constraints: [] };
        seedProjectedBoundary(sketch, face.corners);
        backend.sketches.set(id, sketch);
        return {
          sketchId: id,
          plane: face.plane,
          entityCount: sketch.entities.length,
          constraintCount: sketch.constraints.length,
          faceCount: 1,
          hasClosedBoundary: true,
          projectedBoundaryVersion: 1,
        };
      }
      case "apply_edit_command": {
        const command = p.command as { cmd: string; sketch?: string | { id: string } } | undefined;
        if (command?.cmd === "addSketch" && typeof command.sketch === "object") {
          backend.sketches.set(command.sketch.id, {
            id: command.sketch.id,
            plane: face.plane,
            entities: [],
            constraints: [],
          });
        }
        if (command?.cmd === "deleteSketch") backend.sketches.delete(String(command.sketch));
        return { status: "ready", revision: 1, title: "D", dirty: true, bodies: {}, sketches: {}, features: [] };
      }
      case "enter_sketch":
      case "get_sketch":
        return sessionDto(String(p.sketchId));
      case "sketch_upsert": {
        const ops = (p.ops ?? []) as SketchEditOpWire[];
        backend.upsertOps.push(ops);
        try {
          applyOps(session(String(p.sketchId)), ops);
        } catch (e) {
          backend.upsertErrors.push(e instanceof Error ? e.message : String(e));
          throw e;
        }
        return {
          sketchId: String(p.sketchId),
          sketchRevision: backend.upsertOps.length,
          dof: 0,
          status: "FullyConstrained",
          conflicting: [],
          solvedPositions: {},
        };
      }
      case "finish_sketch":
      case "get_sketch_regions":
        return { regions: [] };
      case "cancel_sketch":
        return null;
      case "face_sketch_plane":
        return face.plane;
      case "promote_selection":
        return [{ topoKey: "f:4", elementId: "el_top", kind: "face", bodyId: "body1" }];
      case "get_projection":
        return { status: "ready", revision: 1, title: "D", dirty: false, bodies: {}, sketches: {}, features: [] };
      default:
        return null;
    }
  });
  return backend;
}

// ── Engine stub (1:1 screen↔plane, no WebGL) ─────────────────────────────────

function makeEngineMock() {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => null),
    clearPlanePickerHover: vi.fn(),
    datumHitTest: vi.fn(() => null),
    setDatumHover: vi.fn(),
    enterSketch: vi.fn(),
    exitSketch: vi.fn(),
    setSketchDrawingActive: vi.fn(),
    setSketchPreview: vi.fn(),
    moveChip: vi.fn(),
    setSketchGhost: vi.fn(),
    setSketchTrimGhost: vi.fn(),
    setSketchAngleReference: vi.fn(),
    setSketchAnglePreview: vi.fn(),
    setSketchSnap: vi.fn(),
    updateSketchSession: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    setExtrudeHandle: vi.fn(),
    probeMaterial: vi.fn(() => null),
    getCameraDistance: vi.fn(() => 100),
  };
}

function disableSnapping(): void {
  const s = settingsStore.getState();
  for (const key of ["grid", "sketchGuideLines", "sketchGuidePoints", "quadrant", "intersection", "onCurve"] as const) {
    s.setSnap(key, false);
  }
}

const tick = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("sketch on a face — the projected boundary survives (real tauriClient marshalling)", () => {
  let backend: Backend;
  let client: CadClient;
  let engine: ReturnType<typeof makeEngineMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(() => {
    resetStores();
    disableSnapping();
    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {};
    backend = installBackend();
    client = createTauriClient();
    engine = makeEngineMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engine as unknown as ViewportEngine,
      client,
      container,
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    clearMocks();
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  function click(x: number, y: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  }

  async function setTool(tool: SketchTool): Promise<void> {
    toolStore.getState().setTool(tool);
    await tick();
  }

  /** Select the box top face, then enter sketch mode — the real face-entry path
   *  (`tryEnterOnSelectedFace`), which runs BEFORE the plane picker. */
  async function enterOnFace(): Promise<string> {
    selectionStore.getState().set([
      { kind: "face", id: "body1#f:4", bodyId: "body1", topoKey: "f:4", elementId: "el_top", anchor: { worldPoint: [0, 0, 15] } },
    ]);
    toolStore.getState().setMode("sketch");
    for (let i = 0; i < 12; i++) await tick();
    const session = sketchStore.getState().session;
    expect(session, "sketch-on-face session should be open").not.toBeNull();
    return session!.sketchId;
  }

  async function reEnter(sketchId: string): Promise<void> {
    toolStore.getState().setMode("model");
    for (let i = 0; i < 12; i++) await tick();
    viewportStore.getState().setActiveSketch(sketchId);
    toolStore.getState().setMode("sketch", sketchId);
    for (let i = 0; i < 12; i++) await tick();
  }

  const lockedIds = (): string[] =>
    (sketchStore.getState().session?.entities ?? []).filter((e) => e.referenceLocked).map((e) => e.id);

  it("R7: the projected boundary survives finish → re-entry → the FIRST edit, with ZERO removeEntity for it", async () => {
    const sketchId = await enterOnFace();

    // The session opens ALREADY carrying the host face's boundary as locked
    // geometry (4 lines; the owned corner points ride on them).
    expect(backend.onFaceCalls).toHaveLength(1);
    expect(backend.onFaceCalls[0].sketchId).toBe(sketchId);
    expect(lockedIds()).toHaveLength(4);
    const lockedBefore = new Set(lockedIds());

    // Draw one user line in the same session.
    await setTool("line");
    click(0, 0);
    click(20, 10);
    await flushSketchMutations();
    expect(sketchStore.getState().session!.entities).toHaveLength(5);

    // Finish → re-enter → make the FIRST edit of the new session.
    await reEnter(sketchId);
    expect(sketchStore.getState().session!.entities).toHaveLength(5);
    expect(new Set(lockedIds())).toEqual(lockedBefore); // same ids, still locked

    await setTool("line");
    click(60, 60);
    click(90, 60);
    await flushSketchMutations();

    // THE PIN. Not one op may remove a locked entity — the backend refuses it
    // outright, so a single stray removal fails the whole batch and the user's
    // line is lost along with the boundary.
    const removals = backend.upsertOps
      .flat()
      .filter((o) => o.op === "removeEntity")
      .map((o) => String((o as { entity: string }).entity));
    expect(
      removals.filter((id) => lockedBefore.has(id)),
      `marshalUpsert emitted removeEntity for locked reference geometry: ${JSON.stringify(removals)}`,
    ).toEqual([]);
    expect(backend.upsertErrors, "no upsert may be refused as referenceLocked").toEqual([]);

    // …and the boundary is still in the DOCUMENT (4 locked lines + 4 locked
    // points), beside both user lines.
    const stored = backend.sketches.get(sketchId)!;
    expect(stored.entities.filter((e) => e.referenceLocked)).toHaveLength(8);
    expect(stored.entities.filter((e) => e.kind === "line" && !e.referenceLocked)).toHaveLength(2);
    expect(stored.constraints.filter((c) => c.kind === "fixed")).toHaveLength(4);

    // One more round trip: after another finish + re-entry it is all still there.
    await reEnter(sketchId);
    expect(sketchStore.getState().session!.entities).toHaveLength(6);
    expect(lockedIds()).toHaveLength(4);
  });

  it("marshal-zero-ops: re-marshalling a hydrated locked session emits NOTHING for it", async () => {
    const sketchId = await enterOnFace();
    await reEnter(sketchId);

    // A no-op upsert of the session exactly as hydrated: the id-map was rebased
    // from the wire, so the diff must be empty — no re-ADD of the boundary (which
    // would duplicate it) and no REMOVE (which the backend refuses).
    const session = sketchStore.getState().session!;
    const before = backend.upsertOps.length;
    await client.sketchUpsert(session.sketchId, session.entities, session.constraints);
    expect(backend.upsertOps.slice(before).flat()).toEqual([]);
  });

  it("a locked entity dropped from the session array is SKIPPED, never marshalled as a removal", async () => {
    // The latch's real job: a tool/hydration gap that loses a locked entity from
    // `next.entities` must not turn into the one op the backend refuses. (The UI
    // has no way to do this today — which is exactly why it is pinned here.)
    const sketchId = await enterOnFace();
    const session = sketchStore.getState().session!;
    const survivors = session.entities.filter((e) => !e.referenceLocked);
    const before = backend.upsertOps.length;
    await client.sketchUpsert(session.sketchId, survivors, []);
    const ops = backend.upsertOps.slice(before).flat();
    expect(ops.filter((o) => o.op === "removeEntity")).toEqual([]);
    expect(backend.upsertErrors).toEqual([]);
    expect(backend.sketches.get(sketchId)!.entities.filter((e) => e.referenceLocked)).toHaveLength(8);
  });

  it("a construction FLIP on locked geometry is swallowed, not marshalled", async () => {
    const sketchId = await enterOnFace();
    const session = sketchStore.getState().session!;
    const flipped = session.entities.map((e) => (e.referenceLocked ? { ...e, construction: true } : e));
    const before = backend.upsertOps.length;
    await client.sketchUpsert(session.sketchId, flipped, session.constraints);
    const ops = backend.upsertOps.slice(before).flat();
    expect(ops.filter((o) => o.op === "setEntityConstruction")).toEqual([]);
    expect(backend.upsertErrors).toEqual([]);
    expect(backend.sketches.get(sketchId)!.entities.filter((e) => e.referenceLocked)).toHaveLength(8);
  });
});
