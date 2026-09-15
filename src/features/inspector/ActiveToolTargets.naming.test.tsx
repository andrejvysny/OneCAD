/*
 * UX review 2026-09-14 — N3, N4, N11, T5: the Targets block has to name what is
 * actually selected.
 *
 * N3 the recap titled a committed Chamfer "Edge operation"; N4 `Reference A:
 * Not selected` read as an error in the middle of a working pick; N11 "Body 1 ·
 * Face" never said WHICH face; T5 `Direction: Normal [0, 0, 1]` never moved.
 */
import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ActiveToolTargets } from "./ActiveToolTargets";
import { ActiveToolInspector } from "./ActiveToolInspector";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { operationAttemptStore } from "@/stores/operationAttemptStore";
import { activeToolPresentation } from "@/tools/modelTools/activeToolPresentation";
import type { ActiveToolPresentedContextFor } from "@/tools/modelTools/activeToolPresentation";

const FACE_ID = "el_9c1d4f0ab7e24d5e8b6a3f19c2d70e41";

function edgeOpTargets(
  overrides: Partial<ActiveToolPresentedContextFor<"filletRadius">> = {},
): ActiveToolPresentedContextFor<"filletRadius"> {
  return {
    tool: "filletRadius",
    kind: "edgeOperation",
    affectedBodies: [{ kind: "body", bodyId: "body1", label: "Body 1", resolution: "current" }],
    edges: [],
    referenceFaces: [{ a: null, b: null }],
    ...overrides,
  };
}

describe("ActiveToolTargets naming", () => {
  beforeEach(() => {
    act(() => documentStore.setState(seedMockDocument()));
    toolChipStore.getState().clear();
    operationAttemptStore.getState().clear();
  });

  it("names the two chamfer reference slots and says a pick is awaited", () => {
    render(<ActiveToolTargets targets={edgeOpTargets()} />);
    expect(screen.getByText("First face")).toBeInTheDocument();
    expect(screen.getByText("Second face")).toBeInTheDocument();
    expect(screen.getAllByText("Awaiting pick")).toHaveLength(2);
    expect(screen.queryByText("Not selected")).toBeNull();
    expect(screen.queryByText(/Reference [AB]/)).toBeNull();
  });

  it("identifies a picked face by its short element id", () => {
    render(
      <ActiveToolTargets
        targets={edgeOpTargets({
          referenceFaces: [
            {
              a: {
                kind: "face",
                bodyId: "body1",
                elementId: FACE_ID,
                label: `Body 1 · face ${FACE_ID}`,
                resolution: "identity-only",
              },
              b: null,
            },
          ],
        })}
      />,
    );
    expect(screen.getByText("Body 1 · Face …d70e41")).toBeInTheDocument();
    // The full id stays in the collapsed reference details, never in the label.
    expect(screen.queryByText(`Body 1 · Face ${FACE_ID}`)).toBeNull();
  });

  it("adds the measured area and normal when the presentation carries them", () => {
    render(
      <ActiveToolTargets
        targets={edgeOpTargets({
          referenceFaces: [
            {
              a: {
                kind: "face",
                bodyId: "body1",
                elementId: FACE_ID,
                label: "Body 1 · face",
                resolution: "identity-only",
                area: 800,
                normal: [0, 0, 1],
              },
              b: null,
            },
          ],
        })}
      />,
    );
    expect(screen.getByText("Body 1 · Face …d70e41 · 800 mm² · n [0, 0, 1]")).toBeInTheDocument();
  });

  it("renders the live extrude direction instead of the frozen arm-time vector", () => {
    const fn = vi.fn();
    act(() => {
      toolChipStore
        .getState()
        .showExtrude(-12, [0, 0, 0], { onValue: fn, onSymmetric: fn, onConfirm: fn, onCancel: fn });
      toolChipStore.getState().setContext("extrudeDepth", {
        tool: "extrudeDepth",
        kind: "profile",
        sketch: { sketchId: "sketch2" },
        regionIds: ["r1"],
        hostBodies: [],
        direction: { kind: "normal", vector: [0, 0, 1] },
      });
    });
    const presentation = activeToolPresentation(toolChipStore.getState());
    if (presentation?.tool !== "extrudeDepth") throw new Error("wrong presentation tool");
    render(<ActiveToolTargets targets={presentation.targets} />);
    expect(screen.getByText("Direction: −Normal [0, 0, 1]")).toBeInTheDocument();
  });

  it("titles the read-only recap with the edge operation that ran", () => {
    act(() => {
      toolChipStore
        .getState()
        .showFillet(2, [0, 0, 0], vi.fn(), { onConfirm: vi.fn() }, { edgeOp: "Chamfer" });
      const token = operationAttemptStore
        .getState()
        .begin("mock-document", activeToolPresentation(toolChipStore.getState()));
      if (token === null) throw new Error("an attempt was already applying");
      operationAttemptStore.getState().settle(token, "completed");
      toolChipStore.getState().clear();
    });

    render(<ActiveToolInspector />);
    expect(screen.getByText("Chamfer")).toBeInTheDocument();
    expect(screen.queryByText("Edge operation")).toBeNull();
  });
});
