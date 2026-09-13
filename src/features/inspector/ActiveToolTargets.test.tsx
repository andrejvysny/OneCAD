import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ActiveToolTargets } from "./ActiveToolTargets";
import { ActiveToolInspector } from "./ActiveToolInspector";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { operationAttemptStore } from "@/stores/operationAttemptStore";
import { activeToolPresentation } from "@/tools/modelTools/activeToolPresentation";
import type {
  ActiveToolBodyReference,
  ActiveToolElementReference,
  ActiveToolPresentedContextFor,
} from "@/tools/modelTools/activeToolPresentation";

const body = (bodyId: string, label: string): ActiveToolBodyReference => ({
  kind: "body",
  bodyId,
  label,
  resolution: "current",
});

const edge = (bodyId: string, elementId: string): ActiveToolElementReference => ({
  kind: "edge",
  bodyId,
  elementId,
  label: `Body · edge ${elementId}`,
  resolution: "identity-only",
});

describe("ActiveToolTargets", () => {
  beforeEach(() => {
    act(() => documentStore.setState(seedMockDocument()));
    toolChipStore.getState().clear();
  });

  it("renders named profile fields and direction without callbacks", () => {
    const targets: ActiveToolPresentedContextFor<"extrudeDepth"> = {
      tool: "extrudeDepth",
      kind: "profile",
      sketch: { kind: "sketch", sketchId: "sketch-1", label: "Base sketch", resolution: "current" },
      regionIds: ["r1", "r2"],
      hostBodies: [body("body-1", "Housing")],
      direction: { kind: "normal", vector: [0, 0, 1] },
    };
    render(<ActiveToolTargets targets={targets} />);
    expect(screen.getByText("Base sketch")).toBeInTheDocument();
    expect(screen.getByText("Regions: 2")).toBeInTheDocument();
    expect(screen.getByText("Direction: Normal [0, 0, 1]")).toBeInTheDocument();
  });

  it("keeps Boolean Target and Tool distinct while identifiers stay collapsed", () => {
    const targets: ActiveToolPresentedContextFor<"booleanOp"> = {
      tool: "booleanOp",
      kind: "boolean",
      target: body("target-with-a-very-long-authoritative-id", "Target body"),
      toolBody: body("tool-with-a-very-long-authoritative-id", "Tool body"),
    };
    render(<ActiveToolTargets targets={targets} />);
    expect(screen.getByText("Boolean Target")).toBeInTheDocument();
    expect(screen.getByText("Boolean Tool")).toBeInTheDocument();
    const details = screen.getByLabelText("Boolean Target reference details").closest("details");
    expect(details).not.toHaveAttribute("open");
    fireEvent.click(screen.getByLabelText("Boolean Target reference details"));
    expect(details).toHaveAttribute("open");
    expect(within(details as HTMLElement).getByText(/target-with-a-very-long-authoritative-id/)).toBeInTheDocument();
  });

  it("shows compact distinct references with collapsed identity details", () => {
    act(() => documentStore.setState({
      bodies: { ...documentStore.getState().bodies, body1: { ...documentStore.getState().bodies.body1, name: "Housing" } },
    }));
    const targets: ActiveToolPresentedContextFor<"filletRadius"> = {
      tool: "filletRadius",
      kind: "edgeOperation",
      affectedBodies: [body("body1", "Housing")],
      edges: [edge("body1", "edge-main")],
      referenceFaces: [{ a: { ...edge("body1", "face-a"), kind: "face" }, b: { ...edge("body1", "face-b"), kind: "face" } }],
    };
    render(<ActiveToolTargets targets={targets} />);
    expect(screen.getByText("Housing · Edge")).toBeInTheDocument();
    expect(screen.getAllByText("Housing · Face")).toHaveLength(2);
    expect(screen.getAllByText("Reference details")).toHaveLength(4);
    const referenceA = screen.getByLabelText("Reference A reference details");
    const details = referenceA.closest("details");
    expect(details).not.toHaveAttribute("open");
    fireEvent.click(referenceA);
    expect(details).toHaveAttribute("open");
    expect(within(details as HTMLElement).getByText("Identity only; topology not verified")).toBeInTheDocument();
  });

  it("keeps a missing element reference visible without its presentation label", () => {
    const targets: ActiveToolPresentedContextFor<"gear"> = {
      tool: "gear",
      kind: "gear",
      support: {
        ...edge("missing-body", "edge-uuid"),
        label: "Leaked body · edge-uuid",
        resolution: "missing",
      },
    };
    render(<ActiveToolTargets targets={targets} />);
    expect(screen.getByText("Missing body (missing-body) · Edge")).toBeInTheDocument();
    expect(screen.getByText("(missing)")).toBeInTheDocument();
    expect(screen.queryByText("Leaked body · edge-uuid")).toBeNull();
  });

  it("states missing context explicitly", () => {
    render(<ActiveToolTargets targets={null} />);
    expect(screen.getByTestId("inspector-targets-missing")).toHaveTextContent(
      "Tool targets unavailable — cancel and reopen the tool.",
    );
  });
});

describe("ActiveToolInspector target projection subscription", () => {
  it("refreshes a mounted Boolean target when the body is renamed or removed", () => {
    documentStore.setState(seedMockDocument());
    toolChipStore.getState().clear();
    toolChipStore.getState().showBoolean("Cut", [0, 0, 0], {
      onOp: () => undefined,
      onConfirm: () => undefined,
    });
    toolChipStore.getState().setContext("booleanOp", {
      tool: "booleanOp",
      kind: "boolean",
      target: { bodyId: "body1" },
      toolBody: { bodyId: "body2" },
    });

    const { unmount } = render(<ActiveToolInspector />);
    expect(screen.getByText("Body 1")).toBeInTheDocument();
    expect(screen.getAllByText("Body 1")).toHaveLength(1);
    expect(screen.getByTestId("chip-bool-target")).toHaveTextContent("Boolean TargetBody 1");
    expect(screen.getByTestId("chip-bool-tool")).toHaveTextContent("Boolean ToolMissing body (body2)");

    act(() => documentStore.setState({
      bodies: { ...documentStore.getState().bodies, body1: { ...documentStore.getState().bodies.body1, name: "Renamed target" } },
    }));
    expect(screen.getByText("Renamed target")).toBeInTheDocument();

    act(() => documentStore.setState({
      bodies: Object.fromEntries(Object.entries(documentStore.getState().bodies).filter(([id]) => id !== "body1")),
    }));
    expect(screen.getByText(/Missing body \(body1\)/)).toBeInTheDocument();
    unmount();
    toolChipStore.getState().clear();
  });
});

/*
 * The attempt lifecycle vs an ARMED tool.
 *
 * `operationAttemptStore` has no authority over arming — `begin` only refuses a
 * second attempt while one is still `applying`, and `authoringEntryBlocked` /
 * `toolStore.setTool` gate on `applying` alone. `completed` is a DISPLAY state,
 * so it must never suppress live controls: the inspector already renders the
 * armed section under `applying` and `failed`, and `completed` was the one
 * asymmetric case.
 *
 * What that cost: every re-edit entered from a history row (`editFeature` →
 * `editXxxFeature`) arms without going through `activateTool`, which is the only
 * caller of `clear()` — so after any successful commit the inspector kept showing
 * that commit's read-only summary and the re-edit rendered NO secondary controls.
 */
describe("ActiveToolInspector attempt lifecycle", () => {
  const armEdgeOp = () =>
    act(() =>
      toolChipStore.getState().showFillet(
        2,
        [0, 0, 0],
        vi.fn(),
        { onConfirm: vi.fn(), onCancel: vi.fn() },
        { showEdgeOpSegments: true, edgeOp: "Chamfer", onEdgeOp: vi.fn() },
      ),
    );

  /** Settle a `completed` attempt carrying the armed tool's own presentation,
   *  exactly as `ModelToolController` does when a commit lands. */
  const settleCompleted = () =>
    act(() => {
      const token = operationAttemptStore
        .getState()
        .begin("mock-document", activeToolPresentation(toolChipStore.getState()));
      if (token === null) throw new Error("an attempt was already applying");
      operationAttemptStore.getState().settle(token, "completed");
    });

  beforeEach(() => {
    act(() => documentStore.setState(seedMockDocument()));
    toolChipStore.getState().clear();
    operationAttemptStore.getState().clear();
  });

  afterEach(() => {
    toolChipStore.getState().clear();
    operationAttemptStore.getState().clear();
  });

  it("reports the last operation once the commit disarmed the tool", () => {
    armEdgeOp();
    settleCompleted();
    act(() => toolChipStore.getState().clear());

    render(<ActiveToolInspector />);
    expect(screen.getByText(/^Completed/)).toBeInTheDocument();
    // No armed tool ⇒ no chip dock and no live controls, only the read-only recap.
    expect(screen.queryByTestId("tool-chip-dock")).toBeNull();
    expect(screen.queryByTestId("chip-edgeop-fillet")).toBeNull();
  });

  it("gives an armed re-edit its own controls even while the last commit reads completed", () => {
    armEdgeOp();
    settleCompleted();
    // The re-edit arms WITHOUT `activateTool`, so the completed attempt survives.
    armEdgeOp();

    render(<ActiveToolInspector />);
    expect(screen.getByTestId("tool-chip-dock")).toBeInTheDocument();
    expect(screen.getByTestId("chip-edgeop-fillet")).toBeInTheDocument();
    expect(screen.getByTestId("chip-chamfer-d2")).toBeInTheDocument();
    expect(screen.queryByText(/^Completed/)).toBeNull();
  });
});
