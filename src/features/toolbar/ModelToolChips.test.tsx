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
    expect(screen.getByTestId("chip-symmetric")).toHaveAttribute("aria-pressed", "true");

    act(() => toolChipStore.getState().showExtrude(10, WORLD, extrudeHandlers(), { showSymmetric: false }));
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
    expect(screen.getByTestId("chip-bool-add")).toBeDisabled();
    expect(screen.getByTestId("chip-bool-cut")).toBeDisabled();
    expect(screen.getByTestId("chip-bool-newbody")).not.toBeDisabled();
    expect(screen.getByRole("group", { name: "Boolean mode" })).toHaveAttribute("title", "Needs an existing body");
  });

  it("omits the boolean segment group in a re-edit cluster (showBooleanSegments off)", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showExtrude(10, WORLD, extrudeHandlers(), { showBooleanSegments: false }));
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

  it("armed edge-op cluster renders the segments, marks the active op, and dispatches", () => {
    render(<ModelToolChips />);
    const onEdgeOp = edgeOpCluster({ edgeOp: "Fillet" });

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
    act(() => toolChipStore.getState().setEdgeOp("Chamfer"));
    expect(screen.getByTestId("chip-edgeop-chamfer")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-edgeop-fillet")).toHaveAttribute("aria-pressed", "false");
  });

  // Opt-in only. (The EDGE-OP re-edit now DOES ask for them — W3 — but a bare
  // `showFillet` with no opts must still render none.)
  it("omits the segments unless the arm asks for them", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showFillet(2, WORLD, vi.fn()));
    expect(screen.queryByTestId("chip-edgeop-fillet")).toBeNull();
  });

  // Shell shares the value-chip branch (`shellThickness`) — it must never grow an
  // edge-op picker just by living next door.
  it("the shell chip renders NO edge-op segments", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showShell(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }),
    );
    expect(screen.getByTestId("chip-confirm")).toBeInTheDocument(); // the armed cluster IS up
    expect(screen.queryByTestId("chip-edgeop-fillet")).toBeNull();
    expect(screen.queryByTestId("chip-edgeop-chamfer")).toBeNull();
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
