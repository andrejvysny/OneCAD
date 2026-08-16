/*
 * The drag payload and the drop routing.
 *
 * Both halves are deliberately free of the engine: the codec is a `DataTransfer`
 * contract and the routing is a pure function of a pick plus a modifier, so a
 * test drives them with a stub and no WebGL.
 */
import { describe, it, expect, vi } from "vitest";

import {
  MATERIAL_DND_TYPE,
  applyMaterialDrop,
  carriesMaterial,
  decodeMaterialDrag,
  encodeMaterialDrag,
  installMaterialDropTarget,
  routeMaterialDrop,
  type MaterialDataTransfer,
  type MaterialDropDeps,
} from "./assignDragDrop";

/** A `DataTransfer` stand-in — jsdom has none. */
function stubTransfer(initial: Record<string, string> = {}): MaterialDataTransfer & {
  data: Record<string, string>;
} {
  const data = { ...initial };
  return {
    data,
    get types() {
      return Object.keys(data);
    },
    getData: (format: string) => data[format] ?? "",
    setData: (format: string, value: string) => {
      data[format] = value;
    },
    effectAllowed: "none",
    dropEffect: "none",
  };
}

describe("the drag payload", () => {
  it("round-trips a material id under its own MIME type", () => {
    const dt = stubTransfer();
    encodeMaterialDrag(dt, "mat_1");

    expect(dt.data[MATERIAL_DND_TYPE]).toBe("mat_1");
    expect(decodeMaterialDrag(dt)).toBe("mat_1");
    // Copy, not move: dropping a material on a body does not take it out of the
    // library.
    expect(dt.effectAllowed).toBe("copy");
  });

  it("recognises the drag from its TYPE alone", () => {
    // `dragover` runs in the browser's protected mode, where `getData` returns
    // "" for every type and only the type list is legible. A dragover gate built
    // on `decodeMaterialDrag` would therefore never fire.
    const dt = stubTransfer({ [MATERIAL_DND_TYPE]: "" });
    expect(carriesMaterial(dt)).toBe(true);
    expect(decodeMaterialDrag(dt)).toBeNull();
  });

  it("ignores a foreign drag", () => {
    const dt = stubTransfer({ "text/plain": "hello" });
    expect(carriesMaterial(dt)).toBe(false);
    expect(decodeMaterialDrag(dt)).toBeNull();
    expect(carriesMaterial(null)).toBe(false);
  });
});

describe("routeMaterialDrop", () => {
  const face = { bodyId: "body1", kind: "face" as const, elementId: "el_top" };

  it("routes a body-face drop to the BODY when Alt is not held (Fusion's default)", () => {
    expect(routeMaterialDrop(face, false)).toEqual({ kind: "body", bodyId: "body1" });
  });

  it("routes it to the FACE when Alt is held", () => {
    expect(routeMaterialDrop(face, true)).toEqual({
      kind: "face",
      bodyId: "body1",
      elementId: "el_top",
    });
  });

  it("falls back to the body for an unpromoted face — an override needs an ElementId", () => {
    expect(routeMaterialDrop({ bodyId: "body1", kind: "face" }, true)).toEqual({
      kind: "body",
      bodyId: "body1",
    });
  });

  it("routes an edge hit to its body — an edge wears no material of its own", () => {
    expect(routeMaterialDrop({ bodyId: "body1", kind: "edge" }, true)).toEqual({
      kind: "body",
      bodyId: "body1",
    });
  });

  it("routes a miss nowhere", () => {
    expect(routeMaterialDrop(null, false)).toEqual({ kind: "none" });
  });
});

function dropDeps(hit: Parameters<typeof routeMaterialDrop>[0], onCanvas = true) {
  const assignBody = vi.fn(async () => {});
  const assignFace = vi.fn(async () => {});
  const deps: MaterialDropDeps = {
    onCanvas: () => onCanvas,
    pick: () => hit,
    assignBody,
    assignFace,
  };
  return { deps, assignBody, assignFace };
}

describe("applyMaterialDrop", () => {
  it("assigns the body for a plain drop", async () => {
    const d = dropDeps({ bodyId: "body1", kind: "face", elementId: "el_top" });
    await applyMaterialDrop(
      { clientX: 10, clientY: 20, altKey: false, materialId: "mat_1" },
      d.deps,
    );

    expect(d.assignBody).toHaveBeenCalledWith("body1", "mat_1");
    expect(d.assignFace).not.toHaveBeenCalled();
  });

  it("assigns the face override for an Alt drop", async () => {
    const d = dropDeps({ bodyId: "body1", kind: "face", elementId: "el_top" });
    await applyMaterialDrop({ clientX: 10, clientY: 20, altKey: true, materialId: "mat_1" }, d.deps);

    expect(d.assignFace).toHaveBeenCalledWith("el_top", "mat_1");
    expect(d.assignBody).not.toHaveBeenCalled();
  });

  it("writes NOTHING for a drop outside the canvas, even over a body", async () => {
    // The containment test is the whole reason a window-level listener is safe:
    // the picker's raycast is not clipped to the canvas rect, so a point over
    // the sidebar can still resolve a hit behind it.
    const d = dropDeps({ bodyId: "body1", kind: "face", elementId: "el_top" }, false);
    const route = await applyMaterialDrop(
      { clientX: 10, clientY: 20, altKey: false, materialId: "mat_1" },
      d.deps,
    );

    expect(route).toEqual({ kind: "none" });
    expect(d.assignBody).not.toHaveBeenCalled();
    expect(d.assignFace).not.toHaveBeenCalled();
  });
});

describe("installMaterialDropTarget", () => {
  function fakeWindow() {
    const handlers: Record<string, EventListener[]> = {};
    return {
      handlers,
      addEventListener: (type: string, fn: EventListener) => {
        (handlers[type] ??= []).push(fn);
      },
      removeEventListener: (type: string, fn: EventListener) => {
        handlers[type] = (handlers[type] ?? []).filter((h) => h !== fn);
      },
    };
  }

  it("accepts our drag over the canvas and declines everything else", () => {
    const target = fakeWindow();
    const d = dropDeps({ bodyId: "body1", kind: "face", elementId: "el_top" });
    installMaterialDropTarget(target, () => d.deps);

    const dt = stubTransfer({ [MATERIAL_DND_TYPE]: "" });
    const accepted = { dataTransfer: dt, clientX: 1, clientY: 2, preventDefault: vi.fn() };
    target.handlers.dragover[0](accepted as unknown as Event);
    expect(accepted.preventDefault).toHaveBeenCalled();
    expect(dt.dropEffect).toBe("copy");

    const foreign = {
      dataTransfer: stubTransfer({ "text/plain": "x" }),
      clientX: 1,
      clientY: 2,
      preventDefault: vi.fn(),
    };
    target.handlers.dragover[0](foreign as unknown as Event);
    // Untouched: every other drag on the page behaves as it did before.
    expect(foreign.preventDefault).not.toHaveBeenCalled();
  });

  it("routes a real drop, and stops listening once disposed", () => {
    const target = fakeWindow();
    const d = dropDeps({ bodyId: "body1", kind: "face", elementId: "el_top" });
    const dispose = installMaterialDropTarget(target, () => d.deps);

    const drop = {
      dataTransfer: stubTransfer({ [MATERIAL_DND_TYPE]: "mat_1" }),
      clientX: 1,
      clientY: 2,
      altKey: false,
      preventDefault: vi.fn(),
    };
    target.handlers.drop[0](drop as unknown as Event);
    expect(d.assignBody).toHaveBeenCalledWith("body1", "mat_1");

    dispose();
    expect(target.handlers.dragover).toEqual([]);
    expect(target.handlers.drop).toEqual([]);
  });
});
