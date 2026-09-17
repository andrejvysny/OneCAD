/*
 * ModelToolChips (M6b chips) — render + dispatch. The chip content is portaled
 * into an engine-owned host node; in a test we inject a minimal fake engine whose
 * mountChip attaches that host to document.body so the portaled controls are
 * queryable, then assert each chip's controls dispatch through the chip-store
 * callbacks the ModelToolController registers.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render as testingRender, screen, act, fireEvent, within } from "@testing-library/react";
import type { ReactElement } from "react";
import { ModelToolChips } from "./ModelToolChips";
import { ActiveToolInspector } from "@/features/inspector/ActiveToolInspector";
import { MODEL_TOOL_CHIP_ID, toolChipStore } from "@/stores/toolChipStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { toolChipPlacementStore } from "@/stores/toolChipPlacementStore";
import { viewportWorkAreaStore } from "@/stores/viewportWorkAreaStore";
import { documentStore } from "@/stores/documentStore";
import { activeToolPresentation } from "@/tools/modelTools/activeToolPresentation";

const WORLD: [number, number, number] = [0, 0, 0];

function seedToolContextProjection(): void {
  documentStore.setState({
    bodies: { body1: { id: "body1", name: "Body 1", visible: true } },
    sketches: {
      sketch1: { id: "sketch1", name: "Sketch 1", visible: true, geometryToken: "g1" },
    },
  });
}

function publishBodyContext(tool: "linearPattern" | "circularPattern" | "mirror"): void {
  seedToolContextProjection();
  toolChipStore.getState().setContext(tool, { tool, kind: "bodies", bodies: [{ bodyId: "body1" }] });
}

function publishProfileContext(tool: "extrudeDepth" | "revolveAngle"): void {
  seedToolContextProjection();
  toolChipStore.getState().setContext(tool, {
    tool,
    kind: "profile",
    sketch: { sketchId: "sketch1" },
    regionIds: ["region1"],
    hostBodies: [],
    direction: tool === "extrudeDepth"
      ? { kind: "normal", vector: [0, 0, 1] }
      : { kind: "sketchLine", lineId: "line1" },
  });
}

/** Render the compact viewport primary alongside inspector-only secondaries. */
const render = (ui: ReactElement) => testingRender(<>{ui}<ActiveToolInspector /></>);
const renderExtrudeUi = () => render(<ModelToolChips />);

/** Fillet secondaries are always docked in the active-tool inspector. */
const openFilletOverflow = (): void => {};

/** Reveal the revolve cluster's New Body / Add / Cut segments, which now live
 *  behind their own `⋯` (see `RevolveChipControls`). */
const openRevolveOverflow = (): void => {
  // Revolve secondaries are docked in the active-tool inspector.
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
    toolChipPlacementStore.getState().reset();
    viewportWorkAreaStore.getState().reset();
  });

  it("renders nothing while cleared", () => {
    render(<ModelToolChips />);
    expect(screen.queryByRole("button", { name: "Confirm" })).toBeNull();
  });

  it("shell chip renders a mm dimension input", () => {
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    expect(screen.getByLabelText("Thickness (mm)")).toHaveValue("2");
    expect(screen.getByText("mm")).toBeInTheDocument();
  });

  it("invalid raw primary text blocks both Enter and clicked Confirm", () => {
    const onConfirm = vi.fn();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showShell(2, WORLD, vi.fn(), {
        onConfirm,
        onCancel: vi.fn(),
      }),
    );
    const input = screen.getByLabelText("Thickness (mm)");
    fireEvent.change(input, { target: { value: "12abc" } });
    expect(screen.getByTestId("chip-confirm")).toBeDisabled();
    fireEvent.keyDown(input, { key: "Enter" });
    fireEvent.click(screen.getByTestId("chip-confirm"));
    expect(onConfirm).not.toHaveBeenCalled();
    expect(toolChipStore.getState().validation).toMatchObject({
      status: "invalid",
      draft: "12abc",
    });
  });

  it("linear-pattern chip dispatches axis / count / confirm", () => {
    const onAxis = vi.fn();
    const onCount = vi.fn();
    const onSpacing = vi.fn();
    const onConfirm = vi.fn();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, { onAxis, onCount, onSpacing, onConfirm }),
    );
    act(() => publishBodyContext("linearPattern"));

    // Axis toggle: X active, click Y.
    expect(screen.getByRole("button", { name: "X" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "Y" }));
    expect(onAxis).toHaveBeenCalledWith("Y");

    // The Total field shows 3; +/− dispatch neighbours (U6: it is an editable
    // field now, so the count reads off `value`, not text content).
    expect(screen.getByTestId("pattern-count")).toHaveValue("3");
    fireEvent.click(screen.getByRole("button", { name: "More instances" }));
    expect(onCount).toHaveBeenCalledWith(4);
    fireEvent.click(screen.getByRole("button", { name: "Fewer instances" }));
    expect(onCount).toHaveBeenCalledWith(2);

    // Spacing input present + ✓ commits (U2: one confirm vocabulary).
    expect(screen.getByLabelText("Spacing (mm)")).toHaveValue("20");
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  /*
   * U6 — the one range policy, at the chip. The stepper stops at 12 so the
   * common case stays one click per instance; TYPING reaches the worker's 128.
   * An out-of-range entry is refused and marked, never clamped: a clamp would
   * commit a count the user never saw previewed.
   */
  /*
   * U4/D18 — the OperationHUD's shared result-summary slot. A body-lifecycle
   * operation must state what it will produce BEFORE Apply: a count alone does
   * not say whether the source survives, and the audit found that unanswerable
   * without committing first. It doubles as the tool's accessible status, which
   * is why it carries `role="status"` and lives OUTSIDE the aria-hidden canvas.
   */
  it("renders the result summary as the HUD's accessible status", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, {
        onAxis: vi.fn(),
        onCount: vi.fn(),
        onSpacing: vi.fn(),
        onConfirm: vi.fn(),
      }),
    );
    act(() => toolChipStore.getState().setResultSummary("Linear pattern · 3 total · 2 new bodies · source retained"));

    const summary = screen.getByTestId("chip-result-summary");
    expect(summary).toHaveTextContent("2 new bodies · source retained");
    expect(summary).toHaveAttribute("role", "status");
    expect(summary).toHaveAttribute("aria-live", "polite");
  });

  it("shows no summary slot for an operation with no body lifecycle to state", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showFillet(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }),
    );
    expect(screen.queryByTestId("chip-result-summary")).toBeNull();
  });

  it("Total accepts a typed count past the stepper bound, and refuses past the worker maximum", () => {
    const onCount = vi.fn();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, {
        onAxis: vi.fn(),
        onCount,
        onSpacing: vi.fn(),
        onConfirm: vi.fn(),
      }),
    );

    const total = screen.getByTestId("pattern-count");
    fireEvent.change(total, { target: { value: "20" } });
    expect(onCount).toHaveBeenCalledWith(20);

    onCount.mockClear();
    fireEvent.change(total, { target: { value: "200" } });
    expect(onCount).not.toHaveBeenCalled();
    // The text stays exactly as typed — editable, not rewritten — and is marked.
    expect(total).toHaveValue("200");
    expect(total).toHaveAttribute("aria-invalid", "true");
  });

  it("a step replaces an invalid count draft and clears only its own validity error", () => {
    const onCount = vi.fn();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, {
        onAxis: vi.fn(),
        onCount,
        onSpacing: vi.fn(),
        onConfirm: vi.fn(),
      }),
    );
    act(() => toolChipStore.getState().setRawValueValidity("other-field", false, "bad"));

    const total = screen.getByTestId("pattern-count");
    fireEvent.change(total, { target: { value: "200" } });
    expect(total).toHaveAttribute("aria-invalid", "true");
    fireEvent.click(screen.getByRole("button", { name: "More instances" }));

    expect(onCount).toHaveBeenCalledWith(4);
    expect(total).toHaveValue("4");
    expect(total).toHaveAttribute("aria-invalid", "false");
    expect(toolChipStore.getState().rawInputErrors).not.toHaveProperty("pattern-count");
    expect(toolChipStore.getState().rawInputErrors).toHaveProperty("other-field");
  });

  it("a fresh same-count pattern arm clears the previous count draft", () => {
    render(<ModelToolChips />);
    const show = () => toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, {
      onAxis: vi.fn(),
      onCount: vi.fn(),
      onSpacing: vi.fn(),
      onConfirm: vi.fn(),
    });
    act(show);
    fireEvent.change(screen.getByTestId("pattern-count"), { target: { value: "200" } });
    expect(screen.getByTestId("pattern-count")).toHaveAttribute("aria-invalid", "true");

    act(show);
    expect(screen.getByTestId("pattern-count")).toHaveValue("3");
    expect(screen.getByTestId("pattern-count")).toHaveAttribute("aria-invalid", "false");
    expect(toolChipStore.getState().rawInputErrors).not.toHaveProperty("pattern-count");
  });

  it("the stepper stops at its own bounds without clamping a typed value", () => {
    const onCount = vi.fn();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showLinearPattern("X", 12, 20, WORLD, {
        onAxis: vi.fn(),
        onCount,
        onSpacing: vi.fn(),
        onConfirm: vi.fn(),
      }),
    );
    expect(screen.getByRole("button", { name: "More instances" })).toBeDisabled();
    // …but typing straight past it still works.
    fireEvent.change(screen.getByTestId("pattern-count"), { target: { value: "40" } });
    expect(onCount).toHaveBeenCalledWith(40);
  });

  it("circular-pattern chip renders a degree input + axis toggle", () => {
    const handlers = { onAxis: vi.fn(), onCount: vi.fn(), onAngle: vi.fn(), onConfirm: vi.fn() };
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showCircularPattern("Z", 4, 360, WORLD, handlers));
    act(() => publishBodyContext("circularPattern"));
    expect(screen.getByRole("button", { name: "Z" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByLabelText("Angle (°)")).toHaveValue("360");
    expect(screen.getByText("°")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(handlers.onConfirm).toHaveBeenCalled();
  });

  it("mirror chip dispatches plane pick + confirm", () => {
    const onPlane = vi.fn();
    const onConfirm = vi.fn();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showMirror("XY", WORLD, { onPlane, onConfirm }));
    act(() => publishBodyContext("mirror"));
    expect(screen.getByRole("button", { name: "XY" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "YZ" }));
    expect(onPlane).toHaveBeenCalledWith("YZ");
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(onConfirm).toHaveBeenCalled();
  });

  it("mirror chip dispatches the fuse toggle and reflects the seeded value", () => {
    const onFuse = vi.fn();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore
        .getState()
        .showMirror("XY", WORLD, { onPlane: vi.fn(), onConfirm: vi.fn(), onFuse }),
    );
    const toggle = screen.getByTestId("chip-mirror-fuse");
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(toggle);
    expect(onFuse).toHaveBeenCalledWith(true);

    // The controller owns the value: the button follows the store write-back.
    act(() => toolChipStore.getState().setFuse(true));
    expect(screen.getByTestId("chip-mirror-fuse")).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByTestId("chip-mirror-fuse"));
    expect(onFuse).toHaveBeenLastCalledWith(false);
  });

  it("mirror chip seeded fused (a re-edit) opens with the toggle pressed", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore
        .getState()
        .showMirror("YZ", WORLD, { onPlane: vi.fn(), onConfirm: vi.fn(), onFuse: vi.fn() }, { fuse: true }),
    );
    expect(screen.getByTestId("chip-mirror-fuse")).toHaveAttribute("aria-pressed", "true");
  });

  // ── WP0 red test — every armed model tool has a visible cancel control ───────

  it.each([
    { kind: "booleanOp", setup: () => toolChipStore.getState().showBoolean("Union", WORLD, { onOp: vi.fn(), onConfirm: vi.fn() }) },
    {
      kind: "linearPattern",
      setup: () => toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onSpacing: vi.fn(), onConfirm: vi.fn() }),
    },
    {
      kind: "circularPattern",
      setup: () => toolChipStore.getState().showCircularPattern("Z", 4, 360, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onAngle: vi.fn(), onConfirm: vi.fn() }),
    },
    { kind: "mirror", setup: () => toolChipStore.getState().showMirror("XY", WORLD, { onPlane: vi.fn(), onConfirm: vi.fn() }) },
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

  it("armed extrude keeps value + badge + ✓/✕ in the viewport and moves ⇔ to inspector", () => {
    const h = extrudeHandlers();
    renderExtrudeUi();
    act(() => toolChipStore.getState().showExtrude(24.5, WORLD, h, { symmetric: false }));
    act(() => publishProfileContext("extrudeDepth"));

    expect(screen.getByLabelText("Depth (mm)")).toHaveValue("24.5");
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

  // WP-U10: the chip host is a named group, and its field is reachable by
  // role + accessible name alone — no testid needed for either query.
  it("the extrude chip's host and its depth field are reachable by role/name", () => {
    renderExtrudeUi();
    act(() => toolChipStore.getState().showExtrude(24.5, WORLD, extrudeHandlers()));

    expect(screen.getByRole("group", { name: "Extrude options" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Depth (mm)" })).toHaveValue("24.5");
  });

  it("⇔ toggle reflects a pressed seed; re-edit (showSymmetric:false) hides it", () => {
    renderExtrudeUi();
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

    const input = screen.getByLabelText("Depth (mm)");
    fireEvent.change(input, { target: { value: "25" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(h.onValue).toHaveBeenCalledWith(25); // typed value applied first
    expect(h.onConfirm).toHaveBeenCalledTimes(1); // then confirm, exactly once
  });

  // ── Wave 2: the New Body / Add / Cut segment group ──────────────────────────

  it("renders the boolean segment group and fires onBooleanMode on a pick", () => {
    const h = { ...extrudeHandlers(), onBooleanMode: vi.fn() };
    renderExtrudeUi();
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
    renderExtrudeUi();
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
    renderExtrudeUi();
    act(() => toolChipStore.getState().showExtrude(10, WORLD, extrudeHandlers(), { showBooleanSegments: false }));
    expect(screen.queryByTestId("chip-bool-cut")).toBeNull();
  });

  it("region-select chip renders the count + confirm / cancel", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showRegionSelect(2, WORLD, { onConfirm, onCancel }));
    act(() => {
      seedToolContextProjection();
      toolChipStore.getState().setContext("regionSelect", {
        tool: "regionSelect",
        kind: "regions",
        sketch: { sketchId: "sketch1" },
        selectedRegionIds: ["region1", "region2"],
      });
    });

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

  it("the fillet badge tracks the active operation", () => {
    render(<ModelToolChips />);
    edgeOpCluster({ edgeOp: "Fillet" });
    expect(screen.getByTestId("chip-edgeop-badge")).toHaveTextContent("Fillet");

    act(() => toolChipStore.getState().setEdgeOp("Chamfer"));
    expect(screen.getByTestId("chip-edgeop-badge")).toHaveTextContent("Chamfer");
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
    expect(screen.getByLabelText("Distance (mm)")).toBeInTheDocument();

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

  // Spec §8.2 (H4b W3): a second leg outside the authoring domain is RAW-INVALID —
  // the typed text stays, nothing is authored or normalized, and the ✓ is refused.
  // An explicitly empty field (or `=`) is still the equal-leg answer.
  it("a second leg below the 0.1 mm floor, non-positive or unparseable is raw-invalid, kept and never authored", () => {
    render(<ModelToolChips />);
    const onDistance2 = vi.fn();
    edgeOpCluster({ edgeOp: "Chamfer", distance2: 2.5, onDistance2 });
    openFilletOverflow();
    for (const [bad, message] of [
      ["0.05", /Must be at least 0\.1 mm/],
      ["0", /Must be at least 0\.1 mm/],
      ["-1", /Must be at least 0\.1 mm/],
      ["2abc", /Enter a complete numeric value/],
    ] as const) {
      fireEvent.change(d2Field(), { target: { value: bad } });
      fireEvent.blur(d2Field());
      expect(onDistance2).not.toHaveBeenCalled();
      expect(d2Field().value).toBe(bad);
      const validation = toolChipStore.getState().validation;
      expect(validation.status).toBe("invalid");
      expect(validation.status === "invalid" ? validation.message : "").toMatch(message);
    }
    // Escape reverts the text and clears the field's own error.
    fireEvent.keyDown(d2Field(), { key: "Escape" });
    expect(d2Field().value).toBe("2.5");
    expect(toolChipStore.getState().validation.status).toBe("valid");
  });

  it("Enter on an out-of-domain second leg neither authors nor confirms", () => {
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
    fireEvent.change(d2Field(), { target: { value: "0.05" } });
    fireEvent.keyDown(d2Field(), { key: "Enter" });
    expect(onDistance2).not.toHaveBeenCalled();
    expect(onConfirm).not.toHaveBeenCalled();
    expect(d2Field().value).toBe("0.05");
  });

  it("Enter in the second leg confirms through the same fresh gate as the primary field", () => {
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
    // The PRIMARY field holds an incomplete draft: the op is not confirmable.
    act(() => toolChipStore.getState().setRawValueValidity("primary", false, "1."));
    fireEvent.change(d2Field(), { target: { value: "3" } });
    fireEvent.keyDown(d2Field(), { key: "Enter" });
    expect(onDistance2).toHaveBeenCalledWith(3);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("while applying, the edge-op secondaries render inert: segments, second leg, angle, flip", () => {
    render(<ModelToolChips />);
    edgeOpCluster({
      edgeOp: "Chamfer",
      onDistance2: vi.fn(),
      onChamferAngle: vi.fn(),
      showChamferFlip: true,
      onChamferFlip: vi.fn(),
    });
    for (const id of ["chip-edgeop-fillet", "chip-edgeop-chamfer", "chip-chamfer-d2", "chip-chamfer-angle", "chip-chamfer-flip"]) {
      expect(screen.getByTestId(id)).not.toBeDisabled();
    }
    act(() => toolChipStore.getState().setPreviewLifecycle({ status: "applying", arm: 1, request: 1 }));
    for (const id of ["chip-edgeop-fillet", "chip-edgeop-chamfer", "chip-chamfer-d2", "chip-chamfer-angle", "chip-chamfer-flip"]) {
      expect(screen.getByTestId(id), id).toBeDisabled();
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

  // ── chamfer angle (SCHEMA §7.3 distance-angle mode) ───────────────────────

  const angleField = () => screen.getByLabelText("Chamfer angle") as HTMLInputElement;

  it("the angle field appears ONLY while the armed op is a Chamfer, beside the second leg", () => {
    render(<ModelToolChips />);
    edgeOpCluster({ edgeOp: "Fillet" });
    openFilletOverflow();
    expect(screen.queryByTestId("chip-chamfer-angle")).toBeNull();

    act(() => toolChipStore.getState().setEdgeOp("Chamfer"));
    expect(screen.getByTestId("chip-chamfer-angle")).toBeInTheDocument();
    expect(screen.getByTestId("chip-chamfer-d2")).toBeInTheDocument();
    expect(screen.getByLabelText("Distance (mm)")).toBeInTheDocument(); // d1 unshadowed

    act(() => toolChipStore.getState().setEdgeOp("Fillet"));
    expect(screen.queryByTestId("chip-chamfer-angle")).toBeNull();
  });

  it("an angle-less chamfer shows an empty field, and typing a number authors the angle", () => {
    render(<ModelToolChips />);
    const onChamferAngle = vi.fn();
    edgeOpCluster({ edgeOp: "Chamfer", onChamferAngle });
    openFilletOverflow();
    expect(angleField().value).toBe("");

    fireEvent.change(angleField(), { target: { value: "30" } });
    fireEvent.blur(angleField());
    expect(onChamferAngle).toHaveBeenCalledWith(30);

    // Clearing it back to empty clears the mode.
    act(() => toolChipStore.getState().setChamferAngle(30));
    onChamferAngle.mockClear();
    fireEvent.change(angleField(), { target: { value: "" } });
    fireEvent.blur(angleField());
    expect(onChamferAngle).toHaveBeenCalledWith(null);
  });

  it("an angle outside (0, 180) is REVERTED, never authored", () => {
    render(<ModelToolChips />);
    const onChamferAngle = vi.fn();
    edgeOpCluster({ edgeOp: "Chamfer", chamferAngleDeg: 30, onChamferAngle });
    openFilletOverflow();
    for (const bad of ["0", "180", "181", "-5", "30abc"]) {
      fireEvent.change(angleField(), { target: { value: bad } });
      fireEvent.blur(angleField());
      expect(onChamferAngle).not.toHaveBeenCalled();
      expect(angleField().value).toBe("30");
    }
  });

  it("the two chamfer modes clear each other VISIBLY (last authored wins)", () => {
    // The controller owns the exclusion (the FSM refuses to hold both) and pushes
    // BOTH values back; what this pins is that the chip actually re-renders the
    // cleared one, so the user sees which mode they are in.
    render(<ModelToolChips />);
    edgeOpCluster({ edgeOp: "Chamfer", distance2: 2.5 });
    openFilletOverflow();
    expect(d2Field().value).toBe("2.5");
    expect(angleField().value).toBe("");

    act(() => {
      toolChipStore.getState().setChamferAngle(30);
      toolChipStore.getState().setDistance2(null); // what the FSM read-back does
    });
    expect(angleField().value).toBe("30");
    expect(d2Field().value).toBe("="); // the second leg emptied in front of the user

    act(() => {
      toolChipStore.getState().setDistance2(4);
      toolChipStore.getState().setChamferAngle(null);
    });
    expect(d2Field().value).toBe("4");
    expect(angleField().value).toBe("");
  });

  it("Enter in the angle field applies the value THEN confirms (single fire)", () => {
    render(<ModelToolChips />);
    const onChamferAngle = vi.fn();
    const onConfirm = vi.fn();
    act(() =>
      toolChipStore.getState().showFillet(
        1,
        WORLD,
        vi.fn(),
        { onConfirm, onCancel: vi.fn() },
        { showEdgeOpSegments: true, edgeOp: "Chamfer", onChamferAngle },
      ),
    );
    openFilletOverflow();
    fireEvent.change(angleField(), { target: { value: "45" } });
    fireEvent.keyDown(angleField(), { key: "Enter" });
    expect(onChamferAngle).toHaveBeenCalledWith(45);
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

  it("disables Cancel once the operation is applying", () => {
    const onCancel = vi.fn();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showShell(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel }),
    );
    act(() =>
      toolChipStore.getState().setPreviewLifecycle({ status: "applying", arm: 1, request: 1 }),
    );

    fireEvent.click(screen.getByTestId("chip-cancel"));
    expect(screen.getByTestId("chip-cancel")).toBeDisabled();
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("armed revolve cluster: degree value + Axis reset + ✓/✕ dispatch through the handlers", () => {
    const h = { onValue: vi.fn(), onResetAxis: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() };
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showRevolve(360, WORLD, h));
    act(() => publishProfileContext("revolveAngle"));

    expect(screen.getByLabelText("Angle (°)")).toHaveValue("360");
    expect(screen.getByText("°")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Change axis" }));
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
    expect(screen.getByTestId("chip-bool-cut")).toBeInTheDocument();
    expect(screen.getByTestId("chip-revolve-badge")).toHaveTextContent("NewBody");

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
    return onEndCondition;
  };

  it("is absent unless the arm asks for it (re-edit shows value + ✓/✕ only)", () => {
    renderExtrudeUi();
    showExtrude({ showEndConditions: false });
    expect(screen.queryByTestId("chip-end-blind")).toBeNull();
  });

  // ThroughAll / ToNext / ToFace all need something to reach; the worker fails
  // them outright with no body ("ToNext requires an existing target body"), so
  // they are not offered rather than offered-and-doomed.
  it("disables the body-reaching conditions when no body exists", () => {
    renderExtrudeUi();
    showExtrude({ canUseBodyEnds: false });
    expect(screen.getByTestId("chip-end-blind")).toBeEnabled();
    expect(screen.getByTestId("chip-end-throughall")).toBeDisabled();
    expect(screen.getByTestId("chip-end-tonext")).toBeDisabled();
    expect(screen.getByTestId("chip-end-toface")).toBeDisabled();
  });

  it("enables them once a body exists and dispatches the pick", () => {
    renderExtrudeUi();
    const onEnd = showExtrude({ canUseBodyEnds: true });
    expect(screen.getByTestId("chip-end-tonext")).toBeEnabled();
    fireEvent.click(screen.getByTestId("chip-end-tonext"));
    expect(onEnd).toHaveBeenCalledWith("ToNext");
  });

  // `ThroughAll` is a tool EXTENT: with NewBody there is no body to reach through
  // and the worker refuses the pair by name (`EXTRUDE_THROUGH_ALL_NO_TARGET`), so
  // the segment is disabled under NewBody even when bodies exist — and back on
  // for Add/Cut.
  it("disables Through all under NewBody even when bodies exist", () => {
    renderExtrudeUi();
    showExtrude({ canUseBodyEnds: true, booleanMode: "NewBody" });
    expect(screen.getByTestId("chip-end-throughall")).toBeDisabled();
    expect(screen.getByTestId("chip-end-tonext")).toBeEnabled();
  });

  it("offers Through all again under Add", () => {
    renderExtrudeUi();
    showExtrude({ canUseBodyEnds: true, booleanMode: "Add" });
    expect(screen.getByTestId("chip-end-throughall")).toBeEnabled();
  });

  it("marks the active condition pressed", () => {
    renderExtrudeUi();
    showExtrude({ canUseBodyEnds: true, booleanMode: "Cut", endCondition: "ThroughAll" });
    expect(screen.getByTestId("chip-end-throughall")).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("chip-end-blind")).toHaveAttribute("aria-pressed", "false");
  });

  // A distance is meaningless for the derived end conditions — the kernel computes
  // it — so the numeric input and the symmetric toggle hide rather than showing a
  // value that does not drive the result.
  it("hides the distance input and the ⇔ toggle for a non-Blind condition", () => {
    renderExtrudeUi();
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

  /** The draft input, scoped past the cluster's own `Depth (mm)` field. */
  const draftInput = () => within(screen.getByTestId("chip-draft-input")).getByLabelText("Draft angle (°)");

  it("is absent unless the arm asks for it", () => {
    renderExtrudeUi();
    showExtrude({ showDraft: false });
    expect(screen.queryByTestId("chip-draft-input")).toBeNull();
  });

  it("keeps the draft field visible in the inspector at zero", () => {
    renderExtrudeUi();
    showExtrude({});
    expect(draftInput()).toHaveValue("0");
  });

  it("dispatches typed degrees without hiding the draft field", () => {
    renderExtrudeUi();
    const onDraft = showExtrude({});

    fireEvent.change(draftInput(), { target: { value: "10" } });
    fireEvent.keyDown(draftInput(), { key: "Enter" });
    expect(onDraft).toHaveBeenCalledWith(10);
  });

  it("a non-zero draft stays visible as its editable value", () => {
    renderExtrudeUi();
    showExtrude({ draftAngleDeg: 12 });
    expect(draftInput()).toHaveValue("12");
    expect(screen.getByText("°")).toBeInTheDocument();
  });

  // Spec §9.2, TODO.md SESSION 37 H2: the draft is a blur-commit field, so the
  // Escape's own blur must not commit the abandoned text either.
  it("Escape restores the edit-start draft, clears its raw error and commits nothing typed", () => {
    renderExtrudeUi();
    const onDraft = showExtrude({ draftAngleDeg: 12 });
    draftInput().focus();
    fireEvent.change(draftInput(), { target: { value: "5x" } });
    expect(toolChipStore.getState().rawInputErrors).toHaveProperty("extrude-draft");
    fireEvent.keyDown(draftInput(), { key: "Escape" });
    expect(toolChipStore.getState().rawInputErrors).toEqual({});
    expect(draftInput()).toHaveValue("12");

    draftInput().focus();
    fireEvent.change(draftInput(), { target: { value: "20" } });
    fireEvent.keyDown(draftInput(), { key: "Escape" });
    expect(onDraft).not.toHaveBeenCalledWith(20);
    expect(onDraft).toHaveBeenLastCalledWith(12);
    expect(draftInput()).toHaveValue("12");
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
    expect(screen.getByLabelText("Offset (mm)")).toHaveValue("2.5");
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
    expect(screen.getByLabelText("Offset (mm)")).toHaveValue("2.5");
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
    const setChipScreenPosition = vi.fn();
    const clientToViewport = vi.fn((x: number, y: number) => ({ x, y }));
    const invalidate = vi.fn();
    setViewportEngine({
      mountChip,
      unmountChip,
      moveChip,
      setChipScreenPosition,
      clientToViewport,
      invalidate,
    } as unknown as ViewportEngine);
    return { mountChip, unmountChip, moveChip, setChipScreenPosition };
  }

  beforeEach(() => {
    toolChipStore.getState().clear();
    toolChipPlacementStore.getState().reset();
    viewportWorkAreaStore.getState().reset();
  });
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
    toolChipPlacementStore.getState().reset();
    viewportWorkAreaStore.getState().reset();
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
    // `avoidValueHandle` rides on EVERY arm (MC-R9): the chip and the value arrow
    // share this anchor, so a chip that does not opt out of the arrow's box covers
    // its grab area and the press meant for the arrow lands on the chip.
    expect(mountChip.mock.calls[0][3]).toEqual({
      axisFrom: [0, 0, 0],
      offsetPx: 56,
      avoidValueHandle: true,
      screenPosition: undefined,
      constrainToSafeRect: true,
      onPlacementStatus: expect.any(Function),
    });
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
    const input = screen.getByLabelText("Depth (mm)");
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
    expect(document.activeElement).toBe(screen.getByLabelText("Depth (mm)"));
  });

  it("a chip WITHOUT an axis placement passes no AXIS placement, but still avoids the arrow", () => {
    const { mountChip } = trackingEngine();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    // A centred chip is the WORST case for the arrow: with no axis offset it sits
    // exactly on the anchor the arrow reaches out from.
    expect(mountChip.mock.calls[0][3]).toEqual({
      axisFrom: undefined,
      offsetPx: undefined,
      avoidValueHandle: true,
      screenPosition: undefined,
      constrainToSafeRect: true,
      onPlacementStatus: expect.any(Function),
    });
  });

  it("moves the same focused draft node through dock and return without confirming", () => {
    const { mountChip } = trackingEngine();
    const onConfirm = vi.fn();
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showShell(2, WORLD, vi.fn(), {
        onConfirm,
        onCancel: vi.fn(),
      }),
    );
    const input = screen.getByLabelText("Thickness (mm)");
    fireEvent.change(input, { target: { value: "12abc" } });
    input.focus();

    fireEvent.click(screen.getByRole("button", { name: "Dock" }));
    expect(screen.getByLabelText("Thickness (mm)")).toBe(input);
    expect(input).toHaveValue("12abc");
    expect(document.activeElement).toBe(input);
    expect(screen.getByTestId("tool-chip-dock")).toContainElement(screen.getByTestId("model-tool-chip"));
    expect(screen.getByTestId("operation-hud")).toHaveClass("w-full", "max-w-full", "min-w-0");

    fireEvent.click(screen.getByRole("button", { name: "Return" }));
    expect(screen.getByLabelText("Thickness (mm)")).toBe(input);
    expect(document.activeElement).toBe(input);
    expect(mountChip).toHaveBeenCalledTimes(2);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("auto-docks when measured chrome leaves no safe footprint", () => {
    const { mountChip } = trackingEngine();
    viewportWorkAreaStore.getState().setViewport({ x: 0, y: 0, width: 320, height: 240 });
    viewportWorkAreaStore.getState().publishRegion("toolbar", { x: 0, y: 0, width: 320, height: 240 });
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    const placement = mountChip.mock.calls[0][3] as {
      onPlacementStatus: (status: { width: number; height: number; fits: boolean | null }) => void;
    };
    act(() => placement.onPlacementStatus({ width: 280, height: 80, fits: false }));
    expect(toolChipPlacementStore.getState().placement.mode).toBe("docked");
  });

  it("auto-docks when a measured content change has no valid final placement", () => {
    const { mountChip } = trackingEngine();
    viewportWorkAreaStore.getState().setViewport({ x: 0, y: 0, width: 800, height: 600 });
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    const placement = mountChip.mock.calls[0][3] as {
      onPlacementStatus: (status: { width: number; height: number; fits: boolean | null }) => void;
    };

    act(() => placement.onPlacementStatus({ width: 500, height: 80, fits: false }));

    expect(toolChipPlacementStore.getState().placement.mode).toBe("docked");
  });

  it("keeps the focused draft node when its anchor moves behind the camera", () => {
    const { mountChip } = trackingEngine();
    viewportWorkAreaStore.getState().setViewport({ x: 0, y: 0, width: 800, height: 600 });
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    const placement = mountChip.mock.calls[0][3] as {
      onPlacementStatus: (status: { width: number; height: number; fits: boolean | null }) => void;
    };
    const input = screen.getByLabelText("Thickness (mm)");
    fireEvent.change(input, { target: { value: "12abc" } });
    input.focus();

    act(() => placement.onPlacementStatus({ width: 280, height: 80, fits: true }));
    expect(toolChipPlacementStore.getState().placement.mode).toBe("anchored");
    act(() => placement.onPlacementStatus({ width: 280, height: 80, fits: false }));

    expect(toolChipPlacementStore.getState().placement.mode).toBe("docked");
    expect(screen.getByLabelText("Thickness (mm)")).toBe(input);
    expect(input).toHaveValue("12abc");
    expect(document.activeElement).toBe(input);
  });

  it("waits for safe bounds instead of docking an unknown placement", () => {
    const { mountChip } = trackingEngine();
    viewportWorkAreaStore.setState((state) => ({
      ...state,
      viewport: { x: 0, y: 0, width: 800, height: 600 },
      obstacleClearRect: null,
    }));
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    const placement = mountChip.mock.calls[0][3] as {
      onPlacementStatus: (status: { width: number; height: number; fits: boolean | null }) => void;
    };

    act(() => placement.onPlacementStatus({ width: 280, height: 80, fits: null }));

    expect(toolChipPlacementStore.getState().placement.mode).toBe("anchored");
  });

  it("temporarily returns a dock preference to its anchor when no dock host exists", () => {
    const { mountChip } = trackingEngine();
    toolChipPlacementStore.getState().dock();
    testingRender(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));

    expect(toolChipPlacementStore.getState().placement.mode).toBe("docked");
    expect(mountChip).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Return" })).toBeInTheDocument();
  });

  it("captures a physical drag, commits screen placement on release, and never confirms", () => {
    const { setChipScreenPosition } = trackingEngine();
    const onConfirm = vi.fn();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn(), { onConfirm, onCancel: vi.fn() }));
    const handle = screen.getByTestId("chip-drag-handle");
    const host = screen.getByTestId("model-tool-chip");
    vi.spyOn(host, "getBoundingClientRect").mockReturnValue({
      left: 60, top: 40, width: 100, height: 40, right: 160, bottom: 80,
    } as DOMRect);
    let captured = false;
    handle.setPointerCapture = vi.fn(() => { captured = true; });
    handle.hasPointerCapture = vi.fn(() => captured);
    handle.releasePointerCapture = vi.fn(() => { captured = false; });
    const escapedRelease = vi.fn();
    document.body.addEventListener("pointerup", escapedRelease);

    fireEvent.pointerDown(handle, { pointerId: 7, clientX: 100, clientY: 80 });
    fireEvent.pointerMove(handle, { pointerId: 7, clientX: 180, clientY: 140 });
    fireEvent.pointerUp(handle, { pointerId: 7, clientX: 180, clientY: 140 });

    expect(setChipScreenPosition).toHaveBeenLastCalledWith(MODEL_TOOL_CHIP_ID, { x: 190, y: 120 });
    expect(toolChipPlacementStore.getState().placement).toEqual({ mode: "floating", x: 190, y: 120 });
    expect(escapedRelease).not.toHaveBeenCalled();
    expect(onConfirm).not.toHaveBeenCalled();
    document.body.removeEventListener("pointerup", escapedRelease);
  });

  it("restores an anchored chip on pointer cancellation", () => {
    const { setChipScreenPosition } = trackingEngine();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    const handle = screen.getByTestId("chip-drag-handle");
    let captured = false;
    handle.setPointerCapture = vi.fn(() => { captured = true; });
    handle.hasPointerCapture = vi.fn(() => captured);
    handle.releasePointerCapture = vi.fn(() => { captured = false; });
    fireEvent.pointerDown(handle, { pointerId: 8, button: 0, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(handle, { pointerId: 8, clientX: 40, clientY: 40 });
    fireEvent.pointerCancel(handle, { pointerId: 8, clientX: 40, clientY: 40 });
    expect(setChipScreenPosition).toHaveBeenLastCalledWith(MODEL_TOOL_CHIP_ID, null);
    expect(toolChipPlacementStore.getState().placement).toEqual({ mode: "anchored" });
  });

  it("restores the original floating position on cancellation", () => {
    const { setChipScreenPosition } = trackingEngine();
    toolChipPlacementStore.getState().floatAt(300, 200);
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    const handle = screen.getByTestId("chip-drag-handle");
    let captured = false;
    handle.setPointerCapture = vi.fn(() => { captured = true; });
    handle.hasPointerCapture = vi.fn(() => captured);
    handle.releasePointerCapture = vi.fn(() => { captured = false; });
    fireEvent.pointerDown(handle, { pointerId: 9, button: 0, clientX: 300, clientY: 200 });
    fireEvent.pointerMove(handle, { pointerId: 9, clientX: 360, clientY: 260 });
    fireEvent.pointerCancel(handle, { pointerId: 9, clientX: 360, clientY: 260 });
    expect(setChipScreenPosition).toHaveBeenLastCalledWith(MODEL_TOOL_CHIP_ID, { x: 300, y: 200 });
    expect(toolChipPlacementStore.getState().placement).toEqual({ mode: "floating", x: 300, y: 200 });
  });

  it("ignores non-primary starts and unmatched releases", () => {
    const { setChipScreenPosition } = trackingEngine();
    render(<ModelToolChips />);
    act(() => toolChipStore.getState().showShell(2, WORLD, vi.fn()));
    const handle = screen.getByTestId("chip-drag-handle");
    handle.setPointerCapture = vi.fn();
    handle.hasPointerCapture = vi.fn(() => false);
    fireEvent.pointerDown(handle, { pointerId: 10, button: 2, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(handle, { pointerId: 11, clientX: 20, clientY: 20 });
    expect(handle.setPointerCapture).not.toHaveBeenCalled();
    expect(setChipScreenPosition).not.toHaveBeenCalled();
    expect(toolChipPlacementStore.getState().placement).toEqual({ mode: "anchored" });
  });

  /*
   * Numeric edit transaction (spec §9.2, TODO.md SESSION 37 H2): Escape in a
   * model-operation field restores the EDIT-START value and stays in the tool;
   * Enter/✓ confirm exactly once and only past a fresh store gate.
   */
  describe("numeric edit transaction (spec §9.2)", () => {
    beforeEach(() => {
      trackingEngine();
    });

    const liveShell = (value: number, handlers: { onConfirm?: () => void } = {}) => {
      const onValue = vi.fn((v: number) => toolChipStore.getState().setValue(v));
      act(() =>
        toolChipStore.getState().showShell(value, WORLD, onValue, {
          onConfirm: handlers.onConfirm ?? vi.fn(),
          onCancel: vi.fn(),
        }),
      );
      return onValue;
    };

    it("Escape restores the edit-start value after a live preview", () => {
      render(<ModelToolChips />);
      const onValue = liveShell(20);
      const input = screen.getByLabelText("Thickness (mm)");
      input.focus();
      fireEvent.change(input, { target: { value: "25" } });
      expect(onValue).toHaveBeenLastCalledWith(25);
      fireEvent.keyDown(input, { key: "Escape" });
      expect(onValue).toHaveBeenLastCalledWith(20);
      expect(screen.getByLabelText("Thickness (mm)")).toHaveValue("20");
      expect(toolChipStore.getState().rawInputErrors).toEqual({});
      expect(toolChipStore.getState().kind).toBe("shellThickness");
    });

    it("Escape over incomplete text clears its raw error and re-enables confirm", () => {
      render(<ModelToolChips />);
      act(() =>
        toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, {
          onAxis: vi.fn(),
          onCount: vi.fn(),
          onSpacing: vi.fn(),
          onConfirm: vi.fn(),
        }),
      );
      act(() => publishBodyContext("linearPattern"));
      const input = screen.getByLabelText("Spacing (mm)");
      input.focus();
      fireEvent.change(input, { target: { value: "20abc" } });
      expect(activeToolPresentation(toolChipStore.getState())?.canConfirm).toBe(false);
      fireEvent.keyDown(input, { key: "Escape" });
      expect(toolChipStore.getState().rawInputErrors).toEqual({});
      expect(activeToolPresentation(toolChipStore.getState())?.canConfirm).toBe(true);
      expect(input).toHaveValue("20");
    });

    it("Escape clears a controller range error through the fallback", () => {
      render(<ModelToolChips />);
      const onValue = liveShell(20);
      act(() =>
        toolChipStore.getState().setRangeValidation({ status: "invalid", draft: 25, message: "Too thick" }),
      );
      const input = screen.getByLabelText("Thickness (mm)");
      input.focus();
      fireEvent.keyDown(input, { key: "Escape" });
      expect(toolChipStore.getState().validation).toEqual({ status: "valid" });
      expect(onValue).toHaveBeenLastCalledWith(20);
    });

    it("a registered revert handler replaces the fallback", () => {
      render(<ModelToolChips />);
      const onValue = liveShell(20);
      const onRevert = vi.fn();
      act(() => toolChipStore.getState().setRevertHandler(onRevert));
      act(() =>
        toolChipStore.getState().setRangeValidation({ status: "invalid", draft: 25, message: "Too thick" }),
      );
      const input = screen.getByLabelText("Thickness (mm)");
      input.focus();
      fireEvent.change(input, { target: { value: "25" } });
      onValue.mockClear();
      fireEvent.keyDown(input, { key: "Escape" });
      expect(onRevert).toHaveBeenCalledTimes(1);
      expect(onRevert).toHaveBeenCalledWith(20);
      expect(onValue).not.toHaveBeenCalled();
      expect(toolChipStore.getState().rangeValidation.status).toBe("invalid");
    });

    it("show* resets the registered revert handler", () => {
      act(() => toolChipStore.getState().setRevertHandler(vi.fn()));
      liveShell(20);
      expect(toolChipStore.getState().onRevertValue).toBeNull();
    });

    it("Escape after a dock move reverts the original value on the same input node", () => {
      render(<ModelToolChips />);
      const onValue = liveShell(2);
      const input = screen.getByLabelText("Thickness (mm)");
      input.focus();
      fireEvent.change(input, { target: { value: "12abc" } });
      fireEvent.click(screen.getByRole("button", { name: "Dock" }));
      expect(screen.getByLabelText("Thickness (mm)")).toBe(input);
      expect(document.activeElement).toBe(input);
      fireEvent.keyDown(input, { key: "Escape" });
      expect(onValue).toHaveBeenLastCalledWith(2);
      expect(screen.getByLabelText("Thickness (mm)")).toBe(input);
      expect(input).toHaveValue("2");
      expect(toolChipStore.getState().rawInputErrors).toEqual({});
    });

    it("Enter while the range check is pending confirms once", () => {
      const onConfirm = vi.fn();
      render(<ModelToolChips />);
      liveShell(2, { onConfirm });
      act(() => toolChipStore.getState().setRangeValidation({ status: "pending", message: "Checking…" }));
      const input = screen.getByLabelText("Thickness (mm)");
      fireEvent.change(input, { target: { value: "3" } });
      fireEvent.keyDown(input, { key: "Enter" });
      expect(onConfirm).toHaveBeenCalledTimes(1);
    });

    it("Enter does not confirm while another field holds a settled raw error", () => {
      const onConfirm = vi.fn();
      render(<ModelToolChips />);
      liveShell(2, { onConfirm });
      act(() => toolChipStore.getState().setRawValueValidity("extrude-draft", false, "5x"));
      const input = screen.getByLabelText("Thickness (mm)");
      fireEvent.change(input, { target: { value: "3" } });
      fireEvent.keyDown(input, { key: "Enter" });
      expect(onConfirm).not.toHaveBeenCalled();
    });

    it("✓ does not confirm over a retained commit failure", () => {
      const onConfirm = vi.fn();
      render(<ModelToolChips />);
      liveShell(2, { onConfirm });
      act(() => toolChipStore.getState().setRetainedCommitFailure("Kernel refused"));
      fireEvent.click(screen.getByTestId("chip-confirm"));
      fireEvent.keyDown(screen.getByLabelText("Thickness (mm)"), { key: "Enter" });
      expect(onConfirm).not.toHaveBeenCalled();
    });

    it("hole diameter Enter applies the value and confirms once", () => {
      const onConfirm = vi.fn();
      const onValue = vi.fn();
      documentStore.setState({ bodies: { body1: { id: "body1", name: "Body 1", visible: true } } });
      render(<ModelToolChips />);
      act(() => {
        toolChipStore.getState().showHole(
          6.6,
          WORLD,
          {
            onValue,
            onHoleType: vi.fn(),
            onDepth: vi.fn(),
            onCbDiameter: vi.fn(),
            onCbDepth: vi.fn(),
            onCsDiameter: vi.fn(),
            onCsAngle: vi.fn(),
            onStandard: vi.fn(),
            onConfirm,
            onCancel: vi.fn(),
          },
          { holeType: "simple", depth: null },
        );
        toolChipStore.getState().setContext("hole", {
          tool: "hole",
          kind: "faces",
          affectedBodies: [{ bodyId: "body1" }],
          faces: [],
        });
      });
      const input = screen.getByLabelText("Hole diameter (mm)");
      fireEvent.change(input, { target: { value: "7" } });
      fireEvent.keyDown(input, { key: "Enter" });
      expect(onValue).toHaveBeenLastCalledWith(7);
      expect(onConfirm).toHaveBeenCalledTimes(1);
    });

    it("linear pattern spacing Enter confirms once", () => {
      const onConfirm = vi.fn();
      render(<ModelToolChips />);
      act(() =>
        toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, {
          onAxis: vi.fn(),
          onCount: vi.fn(),
          onSpacing: vi.fn(),
          onConfirm,
        }),
      );
      act(() => publishBodyContext("linearPattern"));
      const input = screen.getByLabelText("Spacing (mm)");
      fireEvent.change(input, { target: { value: "30" } });
      fireEvent.keyDown(input, { key: "Enter" });
      expect(onConfirm).toHaveBeenCalledTimes(1);
    });

    it("circular pattern angle Enter confirms once", () => {
      const onConfirm = vi.fn();
      render(<ModelToolChips />);
      act(() =>
        toolChipStore.getState().showCircularPattern("Z", 4, 360, WORLD, {
          onAxis: vi.fn(),
          onCount: vi.fn(),
          onAngle: vi.fn(),
          onConfirm,
        }),
      );
      act(() => publishBodyContext("circularPattern"));
      const input = screen.getByLabelText("Angle (°)");
      fireEvent.change(input, { target: { value: "180" } });
      fireEvent.keyDown(input, { key: "Enter" });
      expect(onConfirm).toHaveBeenCalledTimes(1);
    });
  });
});

describe("compact extrude chip and inspector", () => {
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
    act(() => publishProfileContext("extrudeDepth"));
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

  it("keeps value, mode and confirmation in the viewport; secondaries live in the inspector", () => {
    renderExtrudeUi();
    show();
    expect(screen.getByLabelText("Depth (mm)")).toBeInTheDocument();
    expect(screen.getByTestId("chip-mode-badge")).toHaveTextContent("New");
    expect(screen.getByTestId("chip-confirm")).toBeInTheDocument();
    expect(screen.getByTestId("chip-cancel")).toBeInTheDocument();
    expect(screen.queryByTestId("chip-overflow")).toBeNull();
    expect(screen.getByTestId("active-tool-inspector")).toBeInTheDocument();
    expect(screen.getByTestId("chip-end-blind")).toBeInTheDocument();
    expect(screen.getByTestId("chip-draft-input")).toBeInTheDocument();
    expect(screen.getByTestId("chip-symmetric")).toBeInTheDocument();
    expect(screen.getByTestId("chip-bool-cut")).toBeInTheDocument();
  });

  it("the compact badge follows the resolved boolean mode", () => {
    renderExtrudeUi();
    show({ booleanMode: "Add" });
    expect(screen.getByTestId("chip-mode-badge")).toHaveTextContent("Add");
    act(() => toolChipStore.getState().setBooleanMode("Cut"));
    expect(screen.getByTestId("chip-mode-badge")).toHaveTextContent("Cut");
  });

  it("inspector callbacks update the same armed operation and keep the primary input focused", () => {
    renderExtrudeUi();
    const h = show();
    const depth = screen.getByLabelText("Depth (mm)");
    depth.focus();
    fireEvent.click(screen.getByTestId("chip-bool-cut"));
    expect(h.onBooleanMode).toHaveBeenCalledWith("Cut");
    expect(document.activeElement).toBe(depth);
    fireEvent.click(screen.getByTestId("chip-confirm"));
    expect(h.onConfirm).toHaveBeenCalledTimes(1);
  });

  it("invalid inspector draft blocks shared confirmation without replacing the valid preview value", () => {
    renderExtrudeUi();
    const h = show();
    const draft = within(screen.getByTestId("chip-draft-input")).getByLabelText("Draft angle (°)");
    fireEvent.change(draft, { target: { value: "12abc" } });
    expect(screen.getByTestId("chip-confirm")).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Depth (mm)"), { target: { value: "22" } });
    expect(screen.getByTestId("chip-confirm")).toBeDisabled();
    expect(toolChipStore.getState().rawInputErrors).toHaveProperty("extrude-draft");
    fireEvent.click(screen.getByTestId("chip-confirm"));
    expect(h.onConfirm).not.toHaveBeenCalled();
    expect(h.onDraftAngle).not.toHaveBeenCalled();
    expect(toolChipStore.getState().draftAngleDeg).toBe(0);
    fireEvent.change(draft, { target: { value: "12" } });
    expect(screen.getByTestId("chip-confirm")).toBeEnabled();
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

  it("extrude inspector hides Intersect pending vertical proof", () => {
    renderExtrudeUi();
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
    expect(screen.getByTestId("chip-bool-newbody")).toBeInTheDocument();
    expect(screen.getByTestId("chip-bool-add")).toBeInTheDocument();
    expect(screen.getByTestId("chip-bool-cut")).toBeInTheDocument();
    expect(screen.queryByTestId("chip-bool-intersect")).toBeNull();
  });

  it("revolve overflow hides Intersect pending vertical proof", () => {
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
    expect(screen.queryByTestId("chip-bool-intersect")).toBeNull();
  });

  /*
   * WP6 reopened this closure for MIRROR ONLY, as a recorded user-visible change:
   * `MirrorBodyParams.fuseWithOriginal` now has an authoring surface, so the
   * assertion here flips from "absent" to "present and OFF by default". The two
   * pattern rows below are untouched — their `fuseResult` is still hard-coded.
   */
  it("mirror chip exposes the fuse toggle, OFF by default", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showMirror("XY", WORLD, { onPlane: vi.fn(), onConfirm: vi.fn() }),
    );
    expect(screen.getByRole("button", { name: "XY" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.getByTestId("chip-mirror-fuse")).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("button", { name: /Union/i })).toBeNull();
  });

  it("linear-pattern chip has no fuse/union toggle", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, {
        onAxis: vi.fn(),
        onCount: vi.fn(),
        onSpacing: vi.fn(),
        onConfirm: vi.fn(),
      }),
    );
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Fuse|Union/i })).toBeNull();
  });

  it("circular-pattern chip has no fuse/union toggle", () => {
    render(<ModelToolChips />);
    act(() =>
      toolChipStore.getState().showCircularPattern("Z", 4, 360, WORLD, {
        onAxis: vi.fn(),
        onCount: vi.fn(),
        onAngle: vi.fn(),
        onConfirm: vi.fn(),
      }),
    );
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Fuse|Union/i })).toBeNull();
  });
});
