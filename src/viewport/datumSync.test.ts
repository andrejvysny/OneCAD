/*
 * DatumSync store wiring: the initial sweep pushes the projection's datums into
 * the datum VISUALS, add / remove / frame-change re-sync, and the selection store
 * drives the hover + selection tint.
 *
 * The visuals are a fake. They are published by the viewport CONTRIBUTION now
 * (`modules/modeling/datumViewport`), not by the engine, so the module seam is
 * what gets mocked — DatumSync no longer knows the engine exists.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { DatumSync, datumVisuals } from "./datumSync";
import { documentStore, type DatumMeta } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { getDatumVisuals, onDatumVisualsChanged } from "@/modules/modeling/datumViewport";

vi.mock("@/modules/modeling/datumViewport", () => ({
  getDatumVisuals: vi.fn(),
  // The real one fires IMMEDIATELY with the current value — that immediate call
  // is the catch-up path, so a fake that only registers would hide the bug it
  // exists to prevent.
  onDatumVisualsChanged: vi.fn((listener: (v: unknown) => void) => {
    listener((getDatumVisuals as unknown as () => unknown)());
    return () => {};
  }),
}));

function datum(id: string, name: string, z = 10): DatumMeta {
  return {
    id,
    name,
    basePlaneId: "XY",
    offset: z,
    plane: {
      kind: "custom",
      origin: [0, 0, z],
      xAxis: [0, 1, 0],
      yAxis: [-1, 0, 0],
      normal: [0, 0, 1],
    },
    resolvedValid: true,
  };
}

function fakeVisuals() {
  return {
    sync: vi.fn(),
    hitTest: vi.fn(() => null),
    setHover: vi.fn(),
    setSelected: vi.fn(),
    setGhost: vi.fn(),
    ghostVisible: false,
  };
}

let sync: DatumSync;
let engine: ReturnType<typeof fakeVisuals>;

function attach(): void {
  sync = new DatumSync();
  engine = fakeVisuals();
  vi.mocked(getDatumVisuals).mockReturnValue(engine);
  sync.attach();
}

beforeEach(() => {
  documentStore.getState().applyChange({ datums: {} });
  selectionStore.getState().set([]);
  selectionStore.getState().setHover(null);
});

afterEach(() => {
  sync?.detach();
  documentStore.getState().applyChange({ datums: {} });
  selectionStore.getState().set([]);
  selectionStore.getState().setHover(null);
});

describe("datumVisuals", () => {
  it("carries the BACKEND-RESOLVED plane through verbatim", () => {
    const d = datum("d1", "Datum 1", 12);
    const [v] = datumVisuals({ d1: d });
    expect(v).toEqual({ id: "d1", name: "Datum 1", plane: d.plane, resolvedValid: true });
    // Same object identity: nothing re-derives the frame on this side.
    expect(v.plane).toBe(d.plane);
  });
});

describe("DatumSync", () => {
  it("sweeps the existing projection datums into the engine on attach", () => {
    documentStore.getState().addDatum(datum("d1", "Datum 1"));
    attach();
    expect(engine.sync).toHaveBeenCalledTimes(1);
    expect(engine.sync.mock.calls[0][0]).toEqual([
      expect.objectContaining({ id: "d1", name: "Datum 1" }),
    ]);
  });

  it("an added datum re-syncs; a removed one re-syncs without it", () => {
    attach();
    engine.sync.mockClear();

    documentStore.getState().addDatum(datum("d1", "Datum 1"));
    expect(engine.sync).toHaveBeenCalledTimes(1);
    expect(engine.sync.mock.calls[0][0].map((d: { id: string }) => d.id)).toEqual(["d1"]);

    documentStore.getState().addDatum(datum("d2", "Datum 2"));
    expect(engine.sync.mock.lastCall?.[0].map((d: { id: string }) => d.id)).toEqual(["d1", "d2"]);

    documentStore.getState().removeDatum("d1");
    expect(engine.sync.mock.lastCall?.[0].map((d: { id: string }) => d.id)).toEqual(["d2"]);
  });

  it("a frame change re-syncs the new plane (the layer diffs the frame itself)", () => {
    documentStore.getState().addDatum(datum("d1", "Datum 1", 10));
    attach();
    engine.sync.mockClear();

    documentStore.getState().addDatum(datum("d1", "Datum 1", 30));

    expect(engine.sync).toHaveBeenCalledTimes(1);
    expect(engine.sync.mock.lastCall?.[0][0].plane.origin).toEqual([0, 0, 30]);
  });

  it("a projection change that leaves `datums` identical does NOT re-sync", () => {
    attach();
    engine.sync.mockClear();
    documentStore.getState().applyChange({ revision: 42, dirty: true });
    expect(engine.sync).not.toHaveBeenCalled();
  });

  it("mirrors datum selection + hover into the layer, ignoring other entity kinds", () => {
    documentStore.getState().addDatum(datum("d1", "Datum 1"));
    attach();
    engine.setSelected.mockClear();
    engine.setHover.mockClear();

    selectionStore.getState().set([
      { kind: "datum", id: "d1" },
      { kind: "sketch", id: "sketch2" },
    ]);
    expect(engine.setSelected).toHaveBeenLastCalledWith(["d1"]);

    selectionStore.getState().setHover({ kind: "datum", id: "d1" });
    expect(engine.setHover).toHaveBeenLastCalledWith("d1");

    // A body hover is not a datum hover — the tint must clear, not stick.
    selectionStore.getState().setHover({ kind: "body", id: "body1" });
    expect(engine.setHover).toHaveBeenLastCalledWith(null);
  });

  it("detach stops both subscriptions", () => {
    attach();
    sync.detach();
    engine.sync.mockClear();
    engine.setSelected.mockClear();

    documentStore.getState().addDatum(datum("d1", "Datum 1"));
    selectionStore.getState().set([{ kind: "datum", id: "d1" }]);

    expect(engine.sync).not.toHaveBeenCalled();
    expect(engine.setSelected).not.toHaveBeenCalled();
  });
});

describe("DatumSync — the visuals arriving LATE", () => {
  it("replays the full sweep when the contribution attaches after attach()", () => {
    // The two halves start independently: DatumSync attaches with the document,
    // the datum layer attaches when the viewport host reconciles. If the layer
    // lands second, a one-shot push at attach() went into a null and the datums
    // stay invisible until some unrelated store change happens to re-push them.
    const captured: ((v: unknown) => void)[] = [];
    vi.mocked(onDatumVisualsChanged).mockImplementationOnce((listener) => {
      captured.push(listener as (v: unknown) => void); // captured, deliberately NOT fired
      return () => {};
    });
    vi.mocked(getDatumVisuals).mockReturnValue(undefined as never);

    documentStore.getState().applyChange({ datums: { d1: datum("d1", "Base") } });
    sync = new DatumSync();
    sync.attach();

    const late = fakeVisuals();
    expect(late.sync).not.toHaveBeenCalled();

    vi.mocked(getDatumVisuals).mockReturnValue(late);
    for (const listener of captured) listener(late);

    expect(late.sync).toHaveBeenCalledWith([
      expect.objectContaining({ id: "d1", name: "Base" }),
    ]);
    expect(late.setSelected).toHaveBeenCalled();
  });
});
