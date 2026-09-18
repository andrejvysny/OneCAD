/*
 * The armed HOLE cluster (WP-C T3) — render + dispatch.
 *
 * The rule under test is CONDITIONAL RENDERING: only the active profile's
 * dimension pair may be on screen. This is not cosmetic — a field that is visible
 * implies a field that is in the record, and SCHEMA §7.3 makes carrying the OTHER
 * profile's block a rejected edit. A cluster that rendered both would invite the
 * user to author something the backend refuses.
 *
 * Same portal harness as `ModelToolChips.test.tsx`: a fake engine hosts the chip
 * in the document so the portaled controls are queryable.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act, fireEvent, within } from "@testing-library/react";
import { ModelToolChips } from "./ModelToolChips";
import { ActiveToolInspector } from "@/features/inspector/ActiveToolInspector";
import { ModelOperationBar } from "./ModelOperationBar";
import { toolChipStore, type HoleChipHandlers, type HoleChipOpts } from "@/stores/toolChipStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { documentStore } from "@/stores/documentStore";

const WORLD: [number, number, number] = [0, 0, 0];

/**
 * Mirrors ModelToolController.showHoleChip: production always pairs showHole
 * with setContext("hole", { kind: "faces", ... }), and missingRequiredTargetMessage
 * (activeToolPresentation.ts) blocks confirm unless that context is present and
 * its affected body resolves in documentStore. Seed both so confirm-gating tests
 * exercise the real gate instead of the "no context" fallback message.
 */
function showHoleWithContext(
  diameter: number,
  world: [number, number, number],
  handlers: HoleChipHandlers,
  opts: HoleChipOpts,
): void {
  documentStore.setState({
    bodies: { body1: { id: "body1", name: "Body 1", visible: true } },
  });
  toolChipStore.getState().showHole(diameter, world, handlers, opts);
  toolChipStore.getState().setContext("hole", {
    tool: "hole",
    kind: "faces",
    affectedBodies: [{ bodyId: "body1" }],
    faces: [],
  });
}

function fakeEngine(): ViewportEngine {
  return {
    mountChip: (_id: string, el: HTMLElement) => document.body.appendChild(el),
    unmountChip: (_id: string, el: HTMLElement) => el.remove(),
  } as unknown as ViewportEngine;
}

function handlers(): HoleChipHandlers {
  return {
    onValue: vi.fn(),
    onHoleType: vi.fn(),
    onDepth: vi.fn(),
    onCbDiameter: vi.fn(),
    onCbDepth: vi.fn(),
    onCsDiameter: vi.fn(),
    onCsAngle: vi.fn(),
    onStandard: vi.fn(),
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  };
}

const renderHoleUi = () =>
  render(<><ModelToolChips /><ModelOperationBar /><ActiveToolInspector /></>);

describe("HoleChipCluster", () => {
  let h: HoleChipHandlers;

  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
    h = handlers();
  });
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
  });

  it("a simple hole shows the profile segments, Ø, depth and the standards button — and NO conditional pair", () => {
    renderHoleUi();
    act(() => showHoleWithContext(6.6, WORLD, h, { holeType: "simple", depth: null }));

    expect(screen.getByTestId("chip-hole-simple")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByLabelText("Hole diameter (mm)")).toHaveValue("6.6");
    // Through-all reads as a word, never as a blank field.
    expect(screen.getByTestId("chip-hole-depth")).toHaveValue("Thru");
    expect(screen.getByTestId("chip-hole-std")).toBeInTheDocument();
    expect(screen.getByTestId("model-operation-done")).toBeInTheDocument();

    expect(screen.queryByTestId("chip-hole-cb-diameter")).toBeNull();
    expect(screen.queryByTestId("chip-hole-cs-diameter")).toBeNull();
    expect(screen.queryByTestId("chip-hole-cs-90")).toBeNull();
    // Operation identity lives in the stable strip now (spec §4.2); the label
    // keeps the diameter glyph beside the number.
    expect(screen.getByTestId("model-operation-title")).toHaveTextContent("Hole");
    expect(screen.getByTestId("chip-hole-prefix")).toHaveTextContent("⌀");
    const chip = within(screen.getByTestId("operation-label"));
    expect(chip.getByLabelText("Hole diameter (mm)")).toHaveValue("6.6");
    expect(chip.queryByTestId("chip-hole-simple")).toBeNull();
  });

  it("a counterbore shows ONLY the cb pair", () => {
    renderHoleUi();
    act(() =>
      showHoleWithContext(6.6, WORLD, h, {
        holeType: "counterbore",
        depth: 20,
        cbDiameter: 11,
        cbDepth: 6.8,
      }),
    );
    expect(screen.getByTestId("chip-hole-counterbore")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-hole-cb-diameter")).toHaveValue("11");
    expect(screen.getByTestId("chip-hole-cb-depth")).toHaveValue("6.8");
    expect(screen.getByTestId("chip-hole-depth")).toHaveValue("20");
    expect(screen.queryByTestId("chip-hole-cs-diameter")).toBeNull();
    expect(screen.queryByTestId("chip-hole-cs-90")).toBeNull();
  });

  it("a countersink shows ONLY the cs pair, with the four SCHEMA angles as segments", () => {
    renderHoleUi();
    act(() =>
      showHoleWithContext(6.6, WORLD, h, {
        holeType: "countersink",
        depth: 20,
        csDiameter: 12.4,
        csAngleDeg: 90,
      }),
    );
    expect(screen.getByTestId("chip-hole-cs-diameter")).toHaveValue("12.4");
    for (const a of [82, 90, 100, 120]) {
      expect(screen.getByTestId(`chip-hole-cs-${a}`)).toBeInTheDocument();
    }
    // A free-text angle would let the user author a value the backend refuses,
    // so the control is a segmented pick and 90° is the seeded one.
    expect(screen.getByTestId("chip-hole-cs-90")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-hole-cs-82")).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByTestId("chip-hole-cb-diameter")).toBeNull();
  });

  it("dispatches the profile flip, the angle pick and ✓/✕", () => {
    renderHoleUi();
    act(() =>
      showHoleWithContext(6.6, WORLD, h, { holeType: "countersink", csDiameter: 12.4, csAngleDeg: 90 }),
    );
    fireEvent.click(screen.getByTestId("chip-hole-counterbore"));
    expect(h.onHoleType).toHaveBeenCalledWith("counterbore");
    fireEvent.click(screen.getByTestId("chip-hole-cs-82"));
    expect(h.onCsAngle).toHaveBeenCalledWith(82);
    fireEvent.click(screen.getByTestId("model-operation-done"));
    expect(h.onConfirm).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("model-operation-cancel"));
    expect(h.onCancel).toHaveBeenCalled();
  });

  it("the depth field authors a blind depth, and 'Thru' takes it back to through-all", () => {
    renderHoleUi();
    act(() => showHoleWithContext(6.6, WORLD, h, { depth: null }));
    const depth = screen.getByTestId("chip-hole-depth");

    fireEvent.change(depth, { target: { value: "14" } });
    fireEvent.blur(depth);
    expect(h.onDepth).toHaveBeenCalledWith(14);

    act(() => showHoleWithContext(6.6, WORLD, h, { depth: 14 }));
    const again = screen.getByTestId("chip-hole-depth");
    fireEvent.change(again, { target: { value: "thru" } });
    fireEvent.blur(again);
    expect(h.onDepth).toHaveBeenCalledWith(null);

    // A value the length parser refuses REVERTS rather than authoring a partial.
    (h.onDepth as ReturnType<typeof vi.fn>).mockClear();
    fireEvent.change(again, { target: { value: "abc" } });
    fireEvent.blur(again);
    expect(h.onDepth).not.toHaveBeenCalled();
    expect(again).toHaveValue("14");
  });

  it("the standards popover is closed until asked, and a pick reports thread + fit", () => {
    renderHoleUi();
    act(() => showHoleWithContext(6.6, WORLD, h, { holeType: "counterbore" }));
    expect(screen.queryByTestId("chip-hole-std-panel")).toBeNull();

    fireEvent.click(screen.getByTestId("chip-hole-std"));
    expect(screen.getByTestId("chip-hole-std-panel")).toBeInTheDocument();
    // The whole M3…M12 range is offered in both ISO 273 series.
    for (const t of ["M3", "M4", "M5", "M6", "M8", "M10", "M12"]) {
      expect(screen.getByTestId(`chip-hole-std-${t}-close`)).toBeInTheDocument();
      expect(screen.getByTestId(`chip-hole-std-${t}-normal`)).toBeInTheDocument();
    }

    fireEvent.click(screen.getByTestId("chip-hole-std-M6-normal"));
    expect(h.onStandard).toHaveBeenCalledWith("M6", "normal");
    // The pick closes the popover — it is a one-shot filler, not a mode.
    expect(screen.queryByTestId("chip-hole-std-panel")).toBeNull();
  });

  // WP-U10: the cell prints the mm value a pick would apply, never `·`, and
  // names it fully so a screen reader (or a role/name query) can tell M3-close
  // from M3-normal.
  it("the Std cells print the fit's value, and name it with thread + fit + unit", () => {
    renderHoleUi();
    act(() => showHoleWithContext(6.6, WORLD, h, { holeType: "simple" }));
    fireEvent.click(screen.getByTestId("chip-hole-std"));

    const m3Close = screen.getByTestId("chip-hole-std-M3-close");
    expect(m3Close).toHaveTextContent("Ø3.2");
    expect(m3Close).toHaveAccessibleName("M3 close fit Ø3.2 mm");

    const m3Normal = screen.getByTestId("chip-hole-std-M3-normal");
    expect(m3Normal).toHaveTextContent("Ø3.4");
    expect(m3Normal).toHaveAccessibleName("M3 normal fit Ø3.4 mm");

    fireEvent.click(screen.getByTestId("chip-hole-thread"));
    const m3Tap = screen.getByTestId("chip-hole-std-M3-thread");
    expect(m3Tap).toHaveTextContent("Ø2.5");
    expect(m3Tap).toHaveAccessibleName("M3 tap drill Ø2.5 mm");
  });

  it("the thread toggle (WP-T1) swaps the fit pair for a single tap-drill pick", () => {
    renderHoleUi();
    act(() => showHoleWithContext(6.6, WORLD, h, { holeType: "simple" }));
    fireEvent.click(screen.getByTestId("chip-hole-std"));

    const toggle = screen.getByTestId("chip-hole-thread");
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    // Off: the ordinary fit pair is there and NO thread row.
    expect(screen.getByTestId("chip-hole-std-M6-close")).toBeInTheDocument();
    expect(screen.queryByTestId("chip-hole-std-M6-thread")).toBeNull();

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    // On: the fit pair is gone, replaced by ONE pick per size.
    expect(screen.queryByTestId("chip-hole-std-M6-close")).toBeNull();
    expect(screen.queryByTestId("chip-hole-std-M6-normal")).toBeNull();
    expect(screen.getByTestId("chip-hole-std-M6-thread")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("chip-hole-std-M6-thread"));
    expect(h.onStandard).toHaveBeenCalledWith("M6", "normal", true);
    expect(screen.queryByTestId("chip-hole-std-panel")).toBeNull();
  });

  it("a cb dimension edit dispatches the raw millimetres", () => {
    renderHoleUi();
    act(() =>
      showHoleWithContext(6.6, WORLD, h, { holeType: "counterbore", cbDiameter: 11, cbDepth: 6.8 }),
    );
    const cbd = screen.getByTestId("chip-hole-cb-diameter");
    fireEvent.change(cbd, { target: { value: "15" } });
    fireEvent.blur(cbd);
    expect(h.onCbDiameter).toHaveBeenCalledWith(15);
  });

  it("keeps invalid secondary text keyed until that field is corrected", () => {
    renderHoleUi();
    act(() =>
      showHoleWithContext(6.6, WORLD, h, {
        holeType: "counterbore",
        cbDiameter: 11,
        cbDepth: 6.8,
      }),
    );
    const cbDepth = screen.getByTestId("chip-hole-cb-depth");
    fireEvent.change(cbDepth, { target: { value: "abc" } });
    expect(screen.getByTestId("model-operation-done")).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Hole diameter (mm)"), { target: { value: "8" } });
    expect(screen.getByTestId("model-operation-done")).toBeDisabled();
    expect(toolChipStore.getState().rawInputErrors).toHaveProperty("hole-cb-depth");
    act(() => toolChipStore.setState({ holeType: "countersink" }));
    expect(screen.getByTestId("model-operation-done")).toBeEnabled();
    expect(toolChipStore.getState().rawInputErrors).not.toHaveProperty("hole-cb-depth");
  });
});
