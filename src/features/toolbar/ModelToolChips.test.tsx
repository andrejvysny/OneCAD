/*
 * ModelToolChips (M6b chips) — render + dispatch. The chip content is portaled
 * into an engine-owned host node; in a test we inject a minimal fake engine whose
 * mountChip attaches that host to document.body so the portaled controls are
 * queryable, then assert each chip's controls dispatch through the chip-store
 * callbacks the ModelToolController registers.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act, fireEvent, within } from "@testing-library/react";
import { ModelToolChips } from "./ModelToolChips";
import { toolChipStore } from "@/stores/toolChipStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";

const WORLD: [number, number, number] = [0, 0, 0];

/**
 * Reveal the extrude cluster's collapsed controls. Everything but the dimension
 * now lives behind `⋯` (see `ExtrudeChipControls`), so a test that asserts an
 * end condition, the draft segment, ⇔ or a boolean segment must open it first.
 */
const openOverflow = (): void => {
  fireEvent.click(screen.getByTestId("chip-overflow"));
};

/** Reveal the fillet cluster's [Fillet|Chamfer] toggle + chamfer second leg,
 *  which now live behind their own `⋯` (see `EdgeOpChipControls`). */
const openFilletOverflow = (): void => {
  fireEvent.click(screen.getByTestId("chip-fillet-overflow"));
};

/** Reveal the revolve cluster's New Body / Add / Cut segments, which now live
 *  behind their own `⋯` (see `RevolveChipControls`). */
const openRevolveOverflow = (): void => {
  fireEvent.click(screen.getByTestId("chip-revolve-overflow"));
};

/** A fake engine that hosts the chip in the document so the portal is queryable. */
function fakeEngine(): ViewportEngine {
  return {
    mountChip: (_id: string, el: HTMLElement) => document.body.appendChild(el),
    unmountChip: (_id: string, el: HTMLElement) => el.remove(),
  } as unknown as ViewportEngine;
}

describe("ModelToolChips (M6b)", () => {
  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
  });
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
  });

  it("renders nothing while cleared", () => {
    render(<ModelToolChips />);
    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();
  });

  it("shell chip renders a mm dimension input", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    expect(screen.getByLabelText("Dimension value")).toHaveValue("2");
    expect(screen.getByText("mm")).toBeInTheDocument();
  });

  it("linear-pattern chip dispatches axis / count / apply", () => {
    const onAxis = vi.fn();
    const onCount = vi.fn();
    const onSpacing = vi.fn();
    const onApply = vi.fn();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, { onAxis, onCount, onSpacing, onApply }),
    );

    // Axis toggle: X active, click Y.
    expect(screen.getByRole("button", { name: "X" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "Y" }));
    expect(onAxis).toHaveBeenCalledWith("Y");

    // Count stepper shows 3; +/− dispatch neighbours.
    expect(screen.getByTestId("pattern-count")).toHaveTextContent("3");
    fireEvent.click(screen.getByRole("button", { name: "More instances" }));
    expect(onCount).toHaveBeenCalledWith(4);
    fireEvent.click(screen.getByRole("button", { name: "Fewer instances" }));
    expect(onCount).toHaveBeenCalledWith(2);

    // Spacing input present + Apply commits.
    expect(screen.getByLabelText("Dimension value")).toHaveValue("20");
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(onApply).toHaveBeenCalledTimes(1);
  });

  it("circular-pattern chip renders a degree input + axis toggle", () => {
    const handlers = { onAxis: vi.fn(), onCount: vi.fn(), onAngle: vi.fn(), onApply: vi.fn() };
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showCircularPattern("Z", 4, 360, WORLD, handlers));
    expect(screen.getByRole("button", { name: "Z" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByLabelText("Dimension value")).toHaveValue("360");
    expect(screen.getByText("°")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(handlers.onApply).toHaveBeenCalled();
  });

  it("mirror chip dispatches plane pick + apply", () => {
    const onPlane = vi.fn();
    const onApply = vi.fn();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showMirror("XY", WORLD, { onPlane, onApply }));
    expect(screen.getByRole("button", { name: "XY" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "YZ" }));
    expect(onPlane).toHaveBeenCalledWith("YZ");
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(onApply).toHaveBeenCalled();
  });

  // ── WP0 red test — every armed model tool has a visible cancel control ───────

  it.each([
    { kind: "booleanOp", setup: () => toolChipStore.getState().showBoolean("Union", WORLD, vi.fn(), vi.fn()) },
    {
      kind: "linearPattern",
      setup: () => toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onSpacing: vi.fn(), onApply: vi.fn() }),
    },
    {
      kind: "circularPattern",
      setup: () => toolChipStore.getState().showCircularPattern("Z", 4, 360, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onAngle: vi.fn(), onApply: vi.fn() }),
    },
    { kind: "mirror", setup: () => toolChipStore.getState().showMirror("XY", WORLD, { onPlane: vi.fn(), onApply: vi.fn() }) },
  ])("$kind chip has a visible cancel button", ({ setup }) => {
    render(<ModelToolChips />);
    act(() => setup());
    expect(screen.getByRole("button", { name: /cancel|✕/i })).toBeInTheDocument();
  });

  // ── MODEL-HARDEN Wave 1: the armed extrude / revolve commit cluster ─────────

  const extrudeHandlers = () => ({
    onValue: vi.fn(),
    onSymmetric: vi.fn(),
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  });

  it("armed extrude cluster: value + ⇔ toggle + ✓/✕ dispatch through the handlers", () => {
    const h = extrudeHandlers();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showExtrude(24.5, WORLD, h, { symmetric: false }));

    expect(screen.getByLabelText("Dimension value")).toHaveValue("24.5");
    expect(screen.getByText("mm")).toBeInTheDocument();

    openOverflow();
    const sym = screen.getByTestId("chip-symmetric");
    expect(sym).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(sym);
    expect(h.onSymmetric).toHaveBeenCalledWith(true);

    fireEvent.click(screen.getByTestId("chip-confirm"));
    expect(h.onConfirm).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTestId("chip-cancel"));
    expect(h.onCancel).toHaveBeenCalledTimes(1);
  });

  it("⇔ toggle reflects a pressed seed; re-edit (showSymmetric:false) hides it", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showExtrude(10, WORLD, extrudeHandlers(), { symmetric: true }));
    openOverflow();
    expect(screen.getByTestId("chip-symmetric")).toHaveAttribute("aria-pressed", "true");

    act(() => toolChipStore.getState().showExtrude(10, WORLD, extrudeHandlers(), { showSymmetric: false }));
    openOverflow();
    expect(screen.queryByTestId("chip-symmetric")).toBeNull();
    // ✓ / ✕ still present in a re-edit cluster.
    expect(screen.getByTestId("chip-confirm")).toBeInTheDocument();
  });

  it("chip-input Enter applies the typed value THEN confirms, firing onConfirm once", () => {
    const h = extrudeHandlers();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showExtrude(10, WORLD, h));

    const input = screen.getByLabelText("Dimension value");
    fireEvent.change(input, { target: { value: "25" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(h.onValue).toHaveBeenCalledWith(25); // typed value applied first
    expect(h.onConfirm).toHaveBeenCalledTimes(1); // then confirm, exactly once
  });

  // ── Wave 2: the New Body / Add / Cut segment group ──────────────────────────

  it("renders the boolean segment group and fires onBooleanMode on a pick", () => {
    const h = { ...extrudeHandlers(), onBooleanMode: vi.fn() };
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showExtrude(10, WORLD, h, {
        showBooleanSegments: true,
        canBoolean: true,
        booleanMode: "NewBody",
      }),
    );

    openOverflow();
    expect(screen.getByTestId("chip-bool-newbody")).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByTestId("chip-bool-cut"));
    expect(h.onBooleanMode).toHaveBeenCalledWith("Cut");
  });

  it("disables Add/Cut (title 'Needs an existing body') when no boolean target exists", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showExtrude(10, WORLD, { ...extrudeHandlers(), onBooleanMode: vi.fn() }, {
        showBooleanSegments: true,
        canBoolean: false,
      }),
    );
    openOverflow();
    expect(screen.getByTestId("chip-bool-add")).toBeDisabled();
    expect(screen.getByTestId("chip-bool-cut")).toBeDisabled();
    expect(screen.getByTestId("chip-bool-newbody")).not.toBeDisabled();
    expect(screen.getByRole("group", { name: "Boolean mode" })).toHaveAttribute("title", "Needs an existing body");
  });

  it("omits the boolean segment group in a re-edit cluster (showBooleanSegments off)", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showExtrude(10, WORLD, extrudeHandlers(), { showBooleanSegments: false }));
    openOverflow();
    expect(screen.queryByTestId("chip-bool-cut")).toBeNull();
  });

  it("region-select chip renders the count + confirm / cancel", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showRegionSelect(2, WORLD, { onConfirm, onCancel }));

    expect(screen.getByTestId("chip-region-count")).toHaveTextContent("2 regions");
    act(() => toolChipStore.getState().setCount(1));
    expect(screen.getByTestId("chip-region-count")).toHaveTextContent("1 region");

    fireEvent.click(screen.getByTestId("chip-confirm"));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTestId("chip-cancel"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  // ── FILLET-CHAMFER-UNIFY: the [Fillet|Chamfer] segments ─────────────────────

  const edgeOpCluster = (opts: Record<string, unknown> = {}, onEdgeOp = vi.fn()) => {
    act(() =>
      toolChipStore.getState().showFillet(
        2,
        WORLD,
        vi.fn(),
        { onConfirm: vi.fn(), onCancel: vi.fn() },
        { showEdgeOpSegments: true, onEdgeOp, ...opts },
      ),
    );
    return onEdgeOp;
  };

  it("the fillet overflow readout tracks the active op and marks Chamfer non-default", () => {
    render(<ModelToolChips />);
    edgeOpCluster({ edgeOp: "Fillet" });
    expect(screen.getByTestId("chip-fillet-overflow-readout")).toHaveTextContent("Fillet");
    expect(screen.queryByTestId("chip-fillet-overflow-dot")).toBeNull();

    act(() => toolChipStore.getState().setEdgeOp("Chamfer"));
    expect(screen.getByTestId("chip-fillet-overflow-readout")).toHaveTextContent("Chamfer");
    expect(screen.getByTestId("chip-fillet-overflow-dot")).toBeInTheDocument();
  });

  it("armed edge-op cluster renders the segments behind `⋯`, marks the active op, and dispatches", () => {
    render(<ModelToolChips />);
    const onEdgeOp = edgeOpCluster({ edgeOp: "Fillet" });
    openFilletOverflow();

    expect(screen.getByTestId("chip-edgeop-fillet")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-edgeop-chamfer")).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(screen.getByTestId("chip-edgeop-chamfer"));
    expect(onEdgeOp).toHaveBeenCalledWith("Chamfer");
  });

  // A drag that re-types the op only calls `setEdgeOp` — the pressed segment is
  // the readout of what the direction chose, so it has to follow that alone.
  it("the pressed segment follows a live setEdgeOp (the drag-direction flip)", () => {
    render(<ModelToolChips />);
    edgeOpCluster({ edgeOp: "Fillet" });
    openFilletOverflow();
    act(() => toolChipStore.getState().setEdgeOp("Chamfer"));
    expect(screen.getByTestId("chip-edgeop-chamfer")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-edgeop-fillet")).toHaveAttribute("aria-pressed", "false");
  });

  // Opt-in only. (The EDGE-OP re-edit now DOES ask for them — W3 — but a bare
  // `showFillet` with no opts must still render neither the overflow nor its
  // segments.)
  it("omits the overflow (and its segments) unless the arm asks for them", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showFillet(2, WORLD, vi.fn()));
    expect(screen.queryByTestId("chip-fillet-overflow")).toBeNull();
    expect(screen.queryByTestId("chip-edgeop-fillet")).toBeNull();
  });

  // ── chamfer second distance (SCHEMA §7.3, 2026-08-03 — WP-C T2a) ──────────

  const d2Field = () => screen.getByLabelText("Second distance") as HTMLInputElement;

  it("the second-distance field appears ONLY while the armed op is a Chamfer", () => {
    render(<ModelToolChips />);
    edgeOpCluster({ edgeOp: "Fillet" });
    openFilletOverflow();
    // A Fillet has no second leg and SCHEMA §7.3 forbids it from carrying one.
    expect(screen.queryByTestId("chip-chamfer-d2")).toBeNull();

    act(() => toolChipStore.getState().setEdgeOp("Chamfer"));
    expect(screen.getByTestId("chip-chamfer-d2")).toBeInTheDocument();
    // It never shadows the FIRST distance's locator.
    expect(screen.getByLabelText("Dimension value")).toBeInTheDocument();

    act(() => toolChipStore.getState().setEdgeOp("Fillet"));
    expect(screen.queryByTestId("chip-chamfer-d2")).toBeNull();
  });

  it("an equal-leg chamfer shows `=`, and typing a number authors the second leg", () => {
    render(<ModelToolChips />);
    const onDistance2 = vi.fn();
    edgeOpCluster({ edgeOp: "Chamfer", onDistance2 });
    openFilletOverflow();
    expect(d2Field().value).toBe("=");

    fireEvent.change(d2Field(), { target: { value: "2.5" } });
    fireEvent.blur(d2Field());
    expect(onDistance2).toHaveBeenCalledWith(2.5);
  });

  it("clearing the field back to `=` (or empty) clears the second leg", () => {
    render(<ModelToolChips />);
    const onDistance2 = vi.fn();
    edgeOpCluster({ edgeOp: "Chamfer", distance2: 2.5, onDistance2 });
    openFilletOverflow();
    expect(d2Field().value).toBe("2.5"); // a re-edit opens seeded

    fireEvent.change(d2Field(), { target: { value: "" } });
    fireEvent.blur(d2Field());
    expect(onDistance2).toHaveBeenCalledWith(null);
    expect(d2Field().value).toBe("=");

    // …and the literal glyph is accepted as the same gesture.
    act(() => toolChipStore.getState().setDistance2(2.5));
    onDistance2.mockClear();
    fireEvent.change(d2Field(), { target: { value: "=" } });
    fireEvent.blur(d2Field());
    expect(onDistance2).toHaveBeenCalledWith(null);
  });

  it("a non-positive or unparseable second leg is REVERTED, never authored", () => {
    render(<ModelToolChips />);
    const onDistance2 = vi.fn();
    edgeOpCluster({ edgeOp: "Chamfer", distance2: 2.5, onDistance2 });
    openFilletOverflow();
    for (const bad of ["0", "-1", "2abc"]) {
      fireEvent.change(d2Field(), { target: { value: bad } });
      fireEvent.blur(d2Field());
      expect(onDistance2).not.toHaveBeenCalled();
      expect(d2Field().value).toBe("2.5");
    }
  });

  it("Enter in the second-distance field applies the value THEN confirms (single fire)", () => {
    render(<ModelToolChips />);
    const onDistance2 = vi.fn();
    const onConfirm = vi.fn();
    act(() =>
      toolChipStore.getState().showFillet(
        1,
        WORLD,
        vi.fn(),
        { onConfirm, onCancel: vi.fn() },
        { showEdgeOpSegments: true, edgeOp: "Chamfer", onDistance2 },
      ),
    );
    openFilletOverflow();
    fireEvent.change(d2Field(), { target: { value: "3" } });
    fireEvent.keyDown(d2Field(), { key: "Enter" });
    expect(onDistance2).toHaveBeenCalledWith(3);
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  // Shell shares the value-chip branch (`shellThickness`) — it must never grow an
  // edge-op overflow just by living next door.
  it("the shell chip renders NO edge-op overflow", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showShell(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }),
    );
    expect(screen.getByTestId("chip-confirm")).toBeInTheDocument(); // the armed cluster IS up
    expect(screen.queryByTestId("chip-fillet-overflow")).toBeNull();
  });

  it("armed revolve cluster: degree value + Axis reset + ✓/✕ dispatch through the handlers", () => {
    const h = { onValue: vi.fn(), onResetAxis: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() };
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showRevolve(360, WORLD, h));

    expect(screen.getByLabelText("Dimension value")).toHaveValue("360");
    expect(screen.getByText("°")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Axis" }));
    expect(h.onResetAxis).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTestId("chip-confirm"));
    expect(h.onConfirm).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTestId("chip-cancel"));
    expect(h.onCancel).toHaveBeenCalledTimes(1);
  });

  it("omits the revolve overflow unless the arm asks for boolean segments", () => {
    const h = { onValue: vi.fn(), onResetAxis: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() };
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showRevolve(360, WORLD, h));
    expect(screen.queryByTestId("chip-revolve-overflow")).toBeNull();
  });

  it("the revolve overflow holds New Body / Add / Cut and dispatches onBooleanMode", () => {
    const h = { onValue: vi.fn(), onResetAxis: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn(), onBooleanMode: vi.fn() };
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showRevolve(360, WORLD, h, {
        showBooleanSegments: true,
        canBoolean: true,
        booleanMode: "NewBody",
      }),
    );
    expect(screen.queryByTestId("chip-bool-cut")).toBeNull(); // collapsed by default
    expect(screen.getByTestId("chip-revolve-overflow-readout")).toHaveTextContent("New");

    openRevolveOverflow();
    expect(screen.getByTestId("chip-bool-newbody")).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByTestId("chip-bool-cut"));
    expect(h.onBooleanMode).toHaveBeenCalledWith("Cut");
  });

  it("the axisPick chip shows the hint and dispatches ✕ through onCancel", () => {
    const onCancel = vi.fn();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showRevolveAxisPick(WORLD, { onCancel }));

    expect(screen.getByTestId("chip-revolve-axis-hint")).toHaveTextContent("Pick an axis line");
    expect(screen.queryByLabelText("Dimension value")).toBeNull(); // no value yet to edit
    fireEvent.click(screen.getByTestId("chip-cancel"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});

// ── MODEL-OPS W1: end-condition segments ────────────────────────────────────
describe("extrude end-condition segments", () => {
  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
  });

  const showExtrude = (opts: Record<string, unknown>, onEndCondition = vi.fn()) => {
    act(() => {
      toolChipStore.getState().showExtrude(
        10,
        WORLD,
        {
          onValue: vi.fn(),
          onSymmetric: vi.fn(),
          onConfirm: vi.fn(),
          onCancel: vi.fn(),
          onEndCondition,
        },
        { showEndConditions: true, ...opts },
      );
    });
    openOverflow(); // the segments live behind `⋯` now
    return onEndCondition;
  };

  it("is absent unless the arm asks for it (re-edit shows value + ✓/✕ only)", () => {
    render(<ModelToolChips />);
    showExtrude({ showEndConditions: false });
    expect(screen.queryByTestId("chip-end-blind")).toBeNull();
  });

  // ThroughAll / ToNext / ToFace all need something to reach; the worker fails
  // them outright with no body ("ToNext requires an existing target body"), so
  // they are not offered rather than offered-and-doomed.
  it("disables the body-reaching conditions when no body exists", () => {
    render(<ModelToolChips />);
    showExtrude({ canUseBodyEnds: false });
    expect(screen.getByTestId("chip-end-blind")).toBeEnabled();
    expect(screen.getByTestId("chip-end-throughall")).toBeDisabled();
    expect(screen.getByTestId("chip-end-tonext")).toBeDisabled();
    expect(screen.getByTestId("chip-end-toface")).toBeDisabled();
  });

  it("enables them once a body exists and dispatches the pick", () => {
    render(<ModelToolChips />);
    const onEnd = showExtrude({ canUseBodyEnds: true });
    expect(screen.getByTestId("chip-end-tonext")).toBeEnabled();
    fireEvent.click(screen.getByTestId("chip-end-tonext"));
    expect(onEnd).toHaveBeenCalledWith("ToNext");
  });

  it("marks the active condition pressed", () => {
    render(<ModelToolChips />);
    showExtrude({ canUseBodyEnds: true, endCondition: "ThroughAll" });
    expect(screen.getByTestId("chip-end-throughall")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-end-blind")).toHaveAttribute("aria-pressed", "false");
  });

  // A distance is meaningless for the derived end conditions — the kernel computes
  // it — so the numeric input and the symmetric toggle hide rather than showing a
  // value that does not drive the result.
  it("hides the distance input and the ⇔ toggle for a non-Blind condition", () => {
    render(<ModelToolChips />);
    showExtrude({ canUseBodyEnds: true, endCondition: "Blind" });
    expect(screen.queryByRole("button", { name: /symmetric/i })).not.toBeNull();
    act(() => {
      toolChipStore.setState({ endCondition: "ThroughAll" });
    });
    expect(screen.queryByRole("button", { name: /symmetric/i })).toBeNull();
  });
});

// ── WP-C3: the [Draft] segment ──────────────────────────────────────────────
describe("extrude draft segment", () => {
  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
  });

  const showExtrude = (opts: Record<string, unknown>, onDraftAngle = vi.fn()) => {
    act(() => {
      toolChipStore.getState().showExtrude(
        10,
        WORLD,
        {
          onValue: vi.fn(),
          onSymmetric: vi.fn(),
          onConfirm: vi.fn(),
          onCancel: vi.fn(),
          onDraftAngle,
        },
        { showDraft: true, ...opts },
      );
    });
    openOverflow(); // the segment lives behind `⋯` now
    return onDraftAngle;
  };

  /** The draft input, scoped past the cluster's own depth `Dimension value`. */
  const draftInput = () => within(screen.getByTestId("chip-draft-input")).getByLabelText("Dimension value");

  it("is absent unless the arm asks for it", () => {
    render(<ModelToolChips />);
    showExtrude({ showDraft: false });
    expect(screen.queryByTestId("chip-draft")).toBeNull();
  });

  it("opens COLLAPSED at 0 so the common no-draft extrude keeps a small chip", () => {
    render(<ModelToolChips />);
    showExtrude({});
    expect(screen.getByTestId("chip-draft")).toHaveTextContent("Draft");
    expect(screen.queryByTestId("chip-draft-input")).toBeNull();
  });

  it("expands on click and dispatches the typed degrees", () => {
    render(<ModelToolChips />);
    const onDraft = showExtrude({});

    fireEvent.click(screen.getByTestId("chip-draft"));
    fireEvent.change(draftInput(), { target: { value: "10" } });
    fireEvent.keyDown(draftInput(), { key: "Enter" });
    expect(onDraft).toHaveBeenCalledWith(10);
  });

  // A drafted prism looks nearly identical to a straight one at small angles, so
  // the number itself is the only honest readout that a draft is armed.
  it("a NON-ZERO draft opens expanded and shows the angle in the label", () => {
    render(<ModelToolChips />);
    showExtrude({ draftAngleDeg: 12 });
    expect(screen.getByTestId("chip-draft")).toHaveTextContent("Draft 12°");
    expect(draftInput()).toHaveValue("12");
    expect(screen.getByText("°")).toBeInTheDocument();
  });
});

/*
 * OffsetFace cluster (SCHEMA §7.3). Two rules with no other visible surface:
 * WHICH distance types the segment group offers (planarity- and count-driven,
 * decided by the controller and passed through the store), and the fact that a
 * refused value is NEVER written back into the field.
 */
describe("offset-face cluster", () => {
  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
  });
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
  });

  const handlers = () => ({
    onValue: vi.fn(),
    onDistanceType: vi.fn(),
    onChainTangent: vi.fn(),
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  });

  const show = (h: ReturnType<typeof handlers>, opts: Record<string, unknown> = {}) => {
    act(() => toolChipStore.getState().showOffsetFace(2.5, WORLD, h, opts));
  };

  it("renders the distance, the ✓/✕ pair and the tangent toggle", () => {
    render(<ModelToolChips />);
    show(handlers());
    expect(screen.getByLabelText("Dimension value")).toHaveValue("2.5");
    expect(screen.getByTestId("chip-confirm")).toBeInTheDocument();
    expect(screen.getByTestId("chip-offset-tangent")).toHaveAttribute("aria-pressed", "true");
  });

  it("renders NO segment group for a multi-face closure (Offset is the only option)", () => {
    render(<ModelToolChips />);
    // One segment is not a choice — offering a lone "Offset" button would imply
    // there is something else to pick.
    show(handlers(), { distanceTypes: ["Offset"] });
    expect(screen.queryByTestId("chip-offset-type-offset")).toBeNull();
    expect(screen.queryByTestId("chip-offset-type-radius")).toBeNull();
  });

  it("a PLANAR single face offers Offset + Total only", () => {
    render(<ModelToolChips />);
    show(handlers(), { distanceTypes: ["Offset", "Total"] });
    expect(screen.getByTestId("chip-offset-type-offset")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-offset-type-total")).toBeInTheDocument();
    // ABSENT, not disabled: a planar face does not have a radius to grey out.
    expect(screen.queryByTestId("chip-offset-type-radius")).toBeNull();
    expect(screen.queryByTestId("chip-offset-type-diameter")).toBeNull();
  });

  it("a CURVED single face offers Offset + Radius + Diameter only", () => {
    render(<ModelToolChips />);
    show(handlers(), { distanceTypes: ["Offset", "Radius", "Diameter"], distanceType: "Diameter" });
    expect(screen.getByTestId("chip-offset-type-diameter")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-offset-type-radius")).toBeInTheDocument();
    expect(screen.queryByTestId("chip-offset-type-total")).toBeNull();
  });

  it("dispatches the picked distance type and the tangent toggle", () => {
    render(<ModelToolChips />);
    const h = handlers();
    show(h, { distanceTypes: ["Offset", "Total"] });
    fireEvent.click(screen.getByTestId("chip-offset-type-total"));
    expect(h.onDistanceType).toHaveBeenCalledWith("Total");
    fireEvent.click(screen.getByTestId("chip-offset-tangent"));
    expect(h.onChainTangent).toHaveBeenCalledWith(false);
  });

  it("DISABLES the tangent toggle for Total (single face, chain off by definition)", () => {
    render(<ModelToolChips />);
    const h = handlers();
    show(h, { distanceType: "Total", chainTangentFaces: false, distanceTypes: ["Offset", "Total"] });
    const toggle = screen.getByTestId("chip-offset-tangent");
    expect(toggle).toBeDisabled();
    fireEvent.click(toggle);
    expect(h.onChainTangent).not.toHaveBeenCalled();
  });

  it("keeps showing the last VALID value while the error state is set", () => {
    render(<ModelToolChips />);
    show(handlers(), { valueError: true });
    // SCHEMA §7.3 forbids clamping, so the refused entry is simply not stored —
    // the field still reads the number that WAS accepted.
    expect(screen.getByLabelText("Dimension value")).toHaveValue("2.5");
  });
});

/*
 * ANCHOR LIFECYCLE — the extrude chip travels with its arrow, and the arrow moves
 * every drag frame. The mount effect UNMOUNTS on an anchor change (the host is
 * detached from the DOM), so the live position MUST come through `moveChip`, not
 * through a `worldPos` store write: a detached input loses focus, and
 * `DimensionInput` commits on blur.
 */
describe("ModelToolChips anchor lifecycle", () => {
  function trackingEngine() {
    const mountChip = vi.fn(
      (_id: string, el: HTMLElement, _world?: unknown, _placement?: unknown) =>
        document.body.appendChild(el),
    );
    const unmountChip = vi.fn((_id: string, el: HTMLElement) => el.remove());
    const moveChip = vi.fn();
    setViewportEngine({ mountChip, unmountChip, moveChip } as unknown as ViewportEngine);
    return { mountChip, unmountChip, moveChip };
  }

  beforeEach(() => toolChipStore.getState().clear());
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
  });

  it("mounts ONCE per arm and forwards the axis placement", () => {
    const { mountChip, unmountChip } = trackingEngine();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showExtrude(
        10,
        [0, 0, 10],
        { onValue: vi.fn(), onSymmetric: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() },
        { anchorAxisFrom: [0, 0, 0], anchorOffsetPx: 56 },
      ),
    );
    expect(mountChip).toHaveBeenCalledTimes(1);
    expect(mountChip.mock.calls[0][3]).toEqual({ axisFrom: [0, 0, 0], offsetPx: 56 });
    expect(unmountChip).not.toHaveBeenCalled();
  });

  it("a live VALUE change never remounts the chip, and keeps the focused input", () => {
    const { mountChip, unmountChip } = trackingEngine();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showExtrude(
        10,
        [0, 0, 10],
        { onValue: vi.fn(), onSymmetric: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() },
        { anchorAxisFrom: [0, 0, 0], anchorOffsetPx: 56 },
      ),
    );
    const host = screen.getByTestId("model-tool-chip");
    const input = screen.getByLabelText("Dimension value");
    input.focus();

    // 30 drag frames' worth of store churn — value + symmetric, exactly what the
    // controller writes. The anchor is NOT among them by design.
    act(() => {
      for (let i = 0; i < 30; i++) {
        toolChipStore.getState().setValue(10 + i);
        toolChipStore.getState().setSymmetric(i % 2 === 0);
      }
    });

    expect(mountChip).toHaveBeenCalledTimes(1);
    expect(unmountChip).not.toHaveBeenCalled();
    expect(screen.getByTestId("model-tool-chip")).toBe(host);
    expect(document.activeElement).toBe(screen.getByLabelText("Dimension value"));
  });

  it("a chip WITHOUT an axis placement passes no placement at all", () => {
    const { mountChip } = trackingEngine();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    expect(mountChip.mock.calls[0][3]).toEqual({ axisFrom: undefined, offsetPx: undefined });
  });
});

/*
 * THE COLLAPSED EXTRUDE CHIP. The point of the overflow is that the chip carries
 * a dimension and nothing else — but a setting the user cannot see must still be
 * ANNOUNCED, and a boolean mode the drag direction changes on its own must be
 * readable without opening anything.
 */
describe("extrude overflow", () => {
  const handlers = () => ({
    onValue: vi.fn(),
    onSymmetric: vi.fn(),
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
    onBooleanMode: vi.fn(),
    onEndCondition: vi.fn(),
    onDraftAngle: vi.fn(),
  });
  const show = (opts: Record<string, unknown> = {}, h = handlers()) => {
    act(() =>
      toolChipStore.getState().showExtrude(10, WORLD, h, {
        showEndConditions: true,
        showBooleanSegments: true,
        showDraft: true,
        canBoolean: true,
        ...opts,
      }),
    );
    return h;
  };

  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
  });
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
  });

  it("collapses to the dimension, ⋯, ✓ and ✕ — nothing else", () => {
    render(<ModelToolChips />);
    show();
    expect(screen.getByLabelText("Dimension value")).toBeInTheDocument();
    expect(screen.getByTestId("chip-overflow")).toBeInTheDocument();
    expect(screen.getByTestId("chip-confirm")).toBeInTheDocument();
    expect(screen.getByTestId("chip-cancel")).toBeInTheDocument();
    for (const hidden of ["chip-end-blind", "chip-draft", "chip-symmetric", "chip-bool-cut"]) {
      expect(screen.queryByTestId(hidden)).toBeNull();
    }
  });

  it("the ⋯ button READS OUT the resolved boolean mode", () => {
    render(<ModelToolChips />);
    show({ booleanMode: "Add" });
    expect(screen.getByTestId("chip-mode-readout")).toHaveTextContent("Add");
    // …and follows a mode the DRAG resolved, with no user interaction.
    act(() => toolChipStore.getState().setBooleanMode("Cut"));
    expect(screen.getByTestId("chip-mode-readout")).toHaveTextContent("Cut");
  });

  it("a re-edit with no boolean segments shows the neutral ⋯ glyph", () => {
    render(<ModelToolChips />);
    show({ showBooleanSegments: false });
    expect(screen.getByTestId("chip-mode-readout")).toHaveTextContent("⋯");
  });

  it("raises a dot when a HIDDEN setting is non-default, and not otherwise", () => {
    render(<ModelToolChips />);
    show();
    expect(screen.queryByTestId("chip-overflow-dot")).toBeNull();

    show({ draftAngleDeg: 7 });
    expect(screen.getByTestId("chip-overflow-dot")).toBeInTheDocument();

    show({ symmetric: true });
    expect(screen.getByTestId("chip-overflow-dot")).toBeInTheDocument();

    show({ endCondition: "ThroughAll", canUseBodyEnds: true });
    expect(screen.getByTestId("chip-overflow-dot")).toBeInTheDocument();
  });

  it("toggles open/closed, and an OUTSIDE press dismisses it", () => {
    render(<ModelToolChips />);
    show();
    openOverflow();
    expect(screen.getByTestId("chip-overflow-panel")).toBeInTheDocument();

    openOverflow();
    expect(screen.queryByTestId("chip-overflow-panel")).toBeNull();

    openOverflow();
    fireEvent.pointerDown(document.body);
    expect(screen.queryByTestId("chip-overflow-panel")).toBeNull();
  });

  it("a press INSIDE the panel keeps it open", () => {
    render(<ModelToolChips />);
    const h = show();
    openOverflow();
    fireEvent.pointerDown(screen.getByTestId("chip-bool-cut"));
    fireEvent.click(screen.getByTestId("chip-bool-cut"));
    expect(h.onBooleanMode).toHaveBeenCalledWith("Cut");
    expect(screen.getByTestId("chip-overflow-panel")).toBeInTheDocument();
  });

  it("a fresh arm reopens COLLAPSED", () => {
    render(<ModelToolChips />);
    show();
    openOverflow();
    expect(screen.getByTestId("chip-overflow-panel")).toBeInTheDocument();
    // A new arm anchors somewhere else; the panel must not survive it.
    act(() =>
      toolChipStore.getState().showExtrude(10, [0, 0, 40], handlers(), {
        showEndConditions: true,
        showBooleanSegments: true,
        showDraft: true,
      }),
    );
    expect(screen.queryByTestId("chip-overflow-panel")).toBeNull();
  });
});

// ── P4: critical mode closure ───────────────────────────────────────────────
describe("critical mode closure", () => {
  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
  });
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
  });

  it("extrude overflow offers every authored Boolean mode", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showExtrude(
        10,
        WORLD,
        {
          onValue: vi.fn(),
          onSymmetric: vi.fn(),
          onConfirm: vi.fn(),
          onCancel: vi.fn(),
          onBooleanMode: vi.fn(),
        },
        { showBooleanSegments: true, canBoolean: true, booleanMode: "NewBody" },
      ),
    );
    openOverflow();
    expect(screen.getByTestId("chip-bool-newbody")).toBeInTheDocument();
    expect(screen.getByTestId("chip-bool-add")).toBeInTheDocument();
    expect(screen.getByTestId("chip-bool-cut")).toBeInTheDocument();
    expect(screen.getByTestId("chip-bool-intersect")).toBeInTheDocument();
  });

  it("revolve overflow offers every authored Boolean mode", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showRevolve(
        360,
        WORLD,
        {
          onValue: vi.fn(),
          onResetAxis: vi.fn(),
          onConfirm: vi.fn(),
          onCancel: vi.fn(),
          onBooleanMode: vi.fn(),
        },
        { showBooleanSegments: true, canBoolean: true, booleanMode: "NewBody" },
      ),
    );
    openRevolveOverflow();
    expect(screen.getByTestId("chip-bool-newbody")).toBeInTheDocument();
    expect(screen.getByTestId("chip-bool-add")).toBeInTheDocument();
    expect(screen.getByTestId("chip-bool-cut")).toBeInTheDocument();
    expect(screen.getByTestId("chip-bool-intersect")).toBeInTheDocument();
  });

  it("mirror chip has no fuse/union toggle", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showMirror("XY", WORLD, { onPlane: vi.fn(), onApply: vi.fn() }),
    );
    expect(screen.getByRole("button", { name: "XY" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Apply" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Fuse|Union/i })).toBeNull();
  });

  it("linear-pattern chip has no fuse/union toggle", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, {
        onAxis: vi.fn(),
        onCount: vi.fn(),
        onSpacing: vi.fn(),
        onApply: vi.fn(),
      }),
    );
    expect(screen.getByRole("button", { name: "Apply" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Fuse|Union/i })).toBeNull();
  });

  it("circular-pattern chip has no fuse/union toggle", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showCircularPattern("Z", 4, 360, WORLD, {
        onAxis: vi.fn(),
        onCount: vi.fn(),
        onAngle: vi.fn(),
        onApply: vi.fn(),
      }),
    );
    expect(screen.getByRole("button", { name: "Apply" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Fuse|Union/i })).toBeNull();
  });
});
