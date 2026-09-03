/*
 * MULTI-OBJECT SKETCH SESSION — full frontend stack over the REAL tauriClient
 * marshaller (the mock lane cannot see this seam: its `sketchUpsert` full-replaces
 * the session, so it never exercises the id-map DIFF that `marshalUpsert` emits).
 *
 * User-reported defect: draw a rectangle, then a SEPARATE disconnected line in the
 * SAME sketch session — both render while editing — press Finish → only the
 * latest-drawn object survives.
 *
 * The harness is a faithful miniature of the Rust sketch lane behind `mockIPC`:
 * it stores the TYPED document sketch (`SketchEditOp` semantics from
 * `onecad-core::edit::session::apply_sketch_ops`) and answers `enter_sketch` /
 * `get_sketch` in the WORKER WIRE form (`worker::wire::wire_entity`). So the
 * assertions here are about what the FRONTEND sends and re-reads, end to end:
 * SketchController → sketchService queue → tauriClient → marshalUpsert.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { createTauriClient } from "./tauriClient";
import { SketchController } from "@/tools/sketch/SketchController";
import { flushSketchMutations } from "@/tools/sketch/sketchService";
import type { CadClient } from "./client";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { toolStore, type SketchTool } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { settingsStore } from "@/stores/settingsStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

// ── A faithful miniature of the Rust sketch lane ─────────────────────────────

type WireScalar = { value: number };
type TypedEntity =
  | { kind: "point"; id: string; at: [number, number]; construction?: boolean }
  | { kind: "line"; id: string; start: string; end: string; construction?: boolean }
  | { kind: "circle"; id: string; center: string; radius: number; construction?: boolean }
  | {
      kind: "arc";
      id: string;
      center: string;
      radius: number;
      startAngle: number;
      endAngle: number;
      construction?: boolean;
    };
type TypedConstraint = { kind: string; id: string; [k: string]: unknown };
type SketchEditOpWire =
  | { op: "addEntity"; entity: TypedEntity }
  | { op: "removeEntity"; entity: string }
  | { op: "addConstraint"; constraint: TypedConstraint }
  | { op: "removeConstraint"; constraint: string }
  | { op: "setDimension"; constraint: string; value: WireScalar }
  | { op: "setEntityPositions"; positions: [string, [number, number]][] };

interface StoredSketch {
  id: string;
  plane: Record<string, unknown>;
  entities: TypedEntity[];
  constraints: TypedConstraint[];
}

const XY_PLANE = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/** Entity ids a typed constraint references (mirrors Rust `Constraint::entities`). */
function constraintRefs(c: TypedConstraint): string[] {
  const keys = ["point", "point1", "point2", "line", "line1", "line2", "entity", "entity1", "entity2", "curve", "axis"];
  return keys.map((k) => c[k]).filter((v): v is string => typeof v === "string");
}

/** Entity ids a typed entity references (mirrors Rust `referenced_entities`). */
function entityRefs(e: TypedEntity): string[] {
  if (e.kind === "line") return [e.start, e.end];
  if (e.kind === "circle" || e.kind === "arc") return [e.center];
  return [];
}

/** Rust `cascade_remove_entity`: remove the seed + every transitive referrer, then
 *  drop constraints that referenced anything removed. */
function cascadeRemove(sketch: StoredSketch, seed: string): void {
  const removed = new Set([seed]);
  for (;;) {
    const next = sketch.entities.filter(
      (e) => !removed.has(e.id) && entityRefs(e).some((r) => removed.has(r)),
    );
    if (next.length === 0) break;
    for (const e of next) removed.add(e.id);
  }
  sketch.entities = sketch.entities.filter((e) => !removed.has(e.id));
  sketch.constraints = sketch.constraints.filter(
    (c) => !constraintRefs(c).some((r) => removed.has(r)),
  );
}

function applyOps(sketch: StoredSketch, ops: SketchEditOpWire[]): void {
  for (const op of ops) {
    switch (op.op) {
      case "addEntity": {
        if (sketch.entities.some((e) => e.id === op.entity.id)) {
          throw new Error(`duplicate entity id ${op.entity.id}`);
        }
        for (const ref of entityRefs(op.entity)) {
          if (!sketch.entities.some((e) => e.id === ref)) {
            throw new Error(`dangling ref ${ref}`);
          }
        }
        sketch.entities.push(op.entity);
        break;
      }
      case "removeEntity":
        cascadeRemove(sketch, op.entity);
        break;
      case "addConstraint":
        sketch.constraints.push(op.constraint);
        break;
      case "removeConstraint":
        sketch.constraints = sketch.constraints.filter((c) => c.id !== op.constraint);
        break;
      case "setDimension": {
        const c = sketch.constraints.find((x) => x.id === op.constraint);
        if (!c) throw new Error(`constraint ${op.constraint} not found`);
        c.value = op.value;
        break;
      }
      case "setEntityPositions":
        for (const [eid, at] of op.positions) {
          const e = sketch.entities.find((x) => x.id === eid);
          if (!e || e.kind !== "point") throw new Error(`entity ${eid} is not a point`);
          e.at = at;
        }
        break;
    }
  }
}

/** Rust `worker::wire::wire_entity` — the shape `enter_sketch`/`get_sketch` return. */
function toWireEntities(sketch: StoredSketch): unknown[] {
  const at = (id: string): [number, number] | null => {
    const p = sketch.entities.find((e) => e.id === id);
    return p && p.kind === "point" ? p.at : null;
  };
  const out: unknown[] = [];
  for (const e of sketch.entities) {
    if (e.kind === "point") out.push({ id: e.id, type: "Point", at: e.at, construction: !!e.construction });
    else if (e.kind === "line")
      out.push({ id: e.id, type: "Line", p0Ref: e.start, p1Ref: e.end, construction: !!e.construction });
    else if (e.kind === "circle") {
      const c = at(e.center);
      if (c) out.push({ id: e.id, type: "Circle", center: c, centerRef: e.center, radius: e.radius });
    } else {
      const c = at(e.center);
      if (c)
        out.push({
          id: e.id,
          type: "Arc",
          center: c,
          centerRef: e.center,
          radius: e.radius,
          startAngle: e.startAngle,
          endAngle: e.endAngle,
        });
    }
  }
  return out;
}

interface Backend {
  sketches: Map<string, StoredSketch>;
  upsertOps: SketchEditOpWire[][];
  calls: string[];
}

function installBackend(): Backend {
  const backend: Backend = { sketches: new Map(), upsertOps: [], calls: [] };
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
      // The wire constraint form is out of scope here (the id-map seeding path is
      // covered by sketchWireMap.test.ts); an empty list keeps re-entry honest
      // about ENTITIES, which is what this defect is about.
      constraints: [],
      dof: 0,
      status: "UnderConstrained",
      conflicting: [],
    };
  };
  mockIPC((cmd, payload) => {
    backend.calls.push(cmd);
    const p = (payload ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "apply_edit_command": {
        const command = p.command as { cmd: string; sketch?: { id: string } } | undefined;
        if (command?.cmd === "addSketch" && command.sketch) {
          backend.sketches.set(command.sketch.id, {
            id: command.sketch.id,
            plane: XY_PLANE,
            entities: [],
            constraints: [],
          });
        }
        if (command?.cmd === "deleteSketch") backend.sketches.delete(String((command as unknown as { sketch: string }).sketch));
        return { status: "ready", revision: 1, title: "D", dirty: true, bodies: {}, sketches: {}, features: [] };
      }
      case "enter_sketch":
      case "get_sketch":
        return sessionDto(String(p.sketchId));
      case "sketch_upsert": {
        const ops = (p.ops ?? []) as SketchEditOpWire[];
        backend.upsertOps.push(ops);
        applyOps(session(String(p.sketchId)), ops);
        return {
          sketchId: String(p.sketchId),
          sketchRevision: backend.upsertOps.length,
          dof: 0,
          status: "UnderConstrained",
          conflicting: [],
          solvedPositions: {},
        };
      }
      case "finish_sketch":
      case "get_sketch_regions":
        return { regions: [] };
      case "cancel_sketch":
        return null;
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
    // DATUM W1: the plane-pick path consults the datum layer first.
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
    setSketchProjectedIds: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    // Isotropic 1px-per-unit metric: matches `planePixelWorld: 1` above, so a
    // plane distance IS a screen-pixel distance in these tests.
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
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

describe("multi-object sketch session (real tauriClient marshalling)", () => {
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

  /** Enter a fresh sketch through the plane-pick path the app really uses. */
  async function enterFreshSketch(): Promise<void> {
    engine.planePickerHitTest.mockReturnValue("XY" as unknown as null);
    toolStore.getState().setMode("sketch");
    await tick();
    await tick();
    click(0, 0); // plane pick → openSession
    for (let i = 0; i < 10; i++) await tick();
    expect(sketchStore.getState().session).not.toBeNull();
  }

  it("a rectangle plus a separate line: BOTH survive finish and re-entry", async () => {
    await enterFreshSketch();
    const sketchId = sketchStore.getState().session!.sketchId;

    // Gesture 1 — rectangle (4 lines, one commit).
    await setTool("rect");
    click(0, 0);
    click(40, 20);
    await flushSketchMutations();
    expect(sketchStore.getState().session!.entities).toHaveLength(4);

    // Gesture 2 — a SEPARATE, disconnected line.
    await setTool("line");
    click(80, 60);
    click(120, 60);
    await flushSketchMutations();
    expect(sketchStore.getState().session!.entities).toHaveLength(5);

    // No upsert may ever REMOVE geometry in an add-only session.
    const removals = backend.upsertOps.flat().filter((o) => o.op === "removeEntity");
    expect(removals, `marshalUpsert emitted removals in an add-only session: ${JSON.stringify(removals)}`).toEqual([]);

    // The backend document holds 4+1 curves plus their synthesized endpoints.
    const stored = backend.sketches.get(sketchId)!;
    expect(stored.entities.filter((e) => e.kind === "line")).toHaveLength(5);

    // Finish (SketchController.exit(): cancelSketch then finishSketch).
    toolStore.getState().setMode("model");
    for (let i = 0; i < 10; i++) await tick();
    expect(backend.sketches.get(sketchId)!.entities.filter((e) => e.kind === "line")).toHaveLength(5);

    // Re-enter (tree double-click → setMode("sketch", id)) — both objects back.
    viewportStore.getState().setActiveSketch(sketchId);
    toolStore.getState().setMode("sketch", sketchId);
    for (let i = 0; i < 10; i++) await tick();
    expect(sketchStore.getState().session!.entities).toHaveLength(5);
  });

  it("drawing in a RE-ENTERED sketch keeps the geometry drawn before the re-entry", async () => {
    await enterFreshSketch();
    const sketchId = sketchStore.getState().session!.sketchId;

    // Session 1 — a rectangle.
    await setTool("rect");
    click(0, 0);
    click(40, 20);
    await flushSketchMutations();
    expect(sketchStore.getState().session!.entities).toHaveLength(4);

    // Leave (Finish), then re-open the SAME sketch.
    toolStore.getState().setMode("model");
    for (let i = 0; i < 10; i++) await tick();
    viewportStore.getState().setActiveSketch(sketchId);
    toolStore.getState().setMode("sketch", sketchId);
    for (let i = 0; i < 10; i++) await tick();
    expect(sketchStore.getState().session!.entities).toHaveLength(4);

    // Session 2 — a separate standalone line drawn on top of the hydrated rect.
    await setTool("line");
    click(80, 60);
    click(120, 60);
    await flushSketchMutations();

    // An add-only gesture must NEVER marshal a removal: the stale id-map entries
    // from session 1 (frontend ids `e1`..`e4`) are absent from the re-hydrated
    // array (backend UUIDs), so a naive id diff deletes everything drawn before.
    const removals = backend.upsertOps.flat().filter((o) => o.op === "removeEntity");
    expect(
      removals,
      `re-entry left stale id-map entries → marshalUpsert deleted the previous ` +
        `session's geometry: ${JSON.stringify(removals)}`,
    ).toEqual([]);

    const lines = backend.sketches.get(sketchId)!.entities.filter((e) => e.kind === "line");
    expect(lines, "backend sketch must hold the 4 rectangle lines + the new line").toHaveLength(5);

    // And it must still be there after Finish + another re-entry.
    toolStore.getState().setMode("model");
    for (let i = 0; i < 10; i++) await tick();
    viewportStore.getState().setActiveSketch(sketchId);
    toolStore.getState().setMode("sketch", sketchId);
    for (let i = 0; i < 10; i++) await tick();
    expect(sketchStore.getState().session!.entities).toHaveLength(5);
  });
});
