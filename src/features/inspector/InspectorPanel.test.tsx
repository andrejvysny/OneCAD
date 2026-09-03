import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen, act, fireEvent } from "@testing-library/react";
import { InspectorPanel } from "./InspectorPanel";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { getMockLatency, mockClient } from "@/ipc/mockClient";
import { setModelToolController } from "@/tools/modelTools/modelToolBridge";
import type { ModelToolController } from "@/tools/modelTools/ModelToolController";
import { resetStores } from "@/test/resetStores";
import { renderWithPlatform } from "@/test/renderWithPlatform";
import { settleUntil } from "@/test/settle";
import { contributeInspectorSections } from "@/modules/modeling/inspectorSections";
import { flushSketchMutations } from "@/tools/sketch/sketchService";
import type { SketchConstraint, SketchSession } from "@/ipc/types";

/**
 * A live sketch session carrying the constraints the inspector summarizes.
 * `entities` carries one placeholder line — a session with any constraint
 * necessarily has geometry (constraints reference points on entities); a
 * genuinely EMPTY session (design item 12) is its own dedicated fixture below.
 */
function sessionWithConstraints(constraints: SketchConstraint[]): SketchSession {
  return {
    sketchId: "sketch2",
    plane: { kind: "XY", origin: [0, 0, 0], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
    entities: [{ id: "e0", type: "Line", p0: [0, 0], p1: [10, 0] }],
    constraints,
    dof: 3,
    status: "UnderConstrained",
  };
}

describe("InspectorPanel", () => {
  beforeEach(() => resetStores());

  it("shows the SELECTION state for the default sketch selection", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    expect(screen.getByText("Sketch 2")).toBeInTheDocument();
    expect(screen.getByText("Sketch")).toBeInTheDocument();
    expect(screen.getByText("Under-constrained · DOF 3")).toBeInTheDocument();
    expect(screen.getByText("History")).toBeInTheDocument();
    expect(screen.getByText("83.3 mm")).toBeInTheDocument();
  });

  /*
   * U0 red evidence. D14 fixed `constraintStatus.ts` — the pure function guards
   * `dof === 0` — but `InspectorPanel`'s SelectionState branch never consults it:
   * it hardcodes `Under-constrained · DOF {dof}` with no status check and a
   * `?? 0` default (InspectorPanel.tsx:119-123). So a fully-constrained sketch
   * selected in the tree still renders the impossible string the audit caught,
   * and an UNKNOWN sketch renders it too. RED until U7 unifies the authority.
   */
  it("never renders Under-constrained for a sketch the solver reports at DOF 0", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => documentStore.getState().setSketchSolve("sketch2", 0, "ok"));

    expect(screen.queryByText(/Under-constrained/)).toBeNull();
    expect(screen.getByText("Fully constrained · DOF 0")).toBeInTheDocument();
  });

  /*
   * LGU-1 WP-A / F1 red evidence. The audit caught "Fully constrained · DOF 0"
   * on screen in MODEL mode with no sketch open. The `dof === 0` fallback in
   * `constraintStatus.ts` is deliberate and stays (U7/D14: a real solve whose
   * status label lags its own DOF count must not be contradicted) — the defect
   * is one layer up, where `SelectionState` FABRICATES a solve state for a
   * sketch the registry has never heard of (`solve?.status ?? "under"`,
   * `solve?.dof ?? 0`) and so reports the strongest possible claim about a
   * sketch it knows nothing about. Absent evidence must render no placard, not
   * a confident one.
   */
  it("claims nothing about a sketch whose solve state is not loaded", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => {
      documentStore.setState({ sketches: {} });
      selectionStore.getState().set([{ kind: "sketch", id: "sketch2" }]);
    });

    expect(screen.getByText("Sketch")).toBeInTheDocument();
    expect(screen.queryByText(/Fully constrained/)).toBeNull();
    expect(screen.queryByText(/constrained · DOF/)).toBeNull();
  });

  it("shows body status + full history when a body is selected", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));

    expect(screen.getByText("Body 1")).toBeInTheDocument();
    expect(screen.getByText("Solid body")).toBeInTheDocument();
    expect(screen.getByText("Sketch 1")).toBeInTheDocument();
    expect(screen.getByText("Fillet")).toBeInTheDocument();
    // The row renders `primaryValue` through the DISPLAY formatter now (H3), which
    // trims trailing zeros exactly as every other length in the app does — the
    // backend's millimetre-fixed "2.0 mm" is still on `FeatureMeta.valueText`.
    expect(screen.getByText("2 mm")).toBeInTheDocument();
    // A body is fully defined — no DOF line.
    expect(screen.queryByText("Under-constrained · DOF 3")).toBeNull();
  });

  it("shows face status + appearance when a promoted face is selected", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() =>
      selectionStore.getState().set([
        {
          kind: "face",
          id: "body1#f:0",
          bodyId: "body1",
          topoKey: "f:0",
          elementId: "el_top",
        },
      ]),
    );

    expect(screen.getByText("Body 1")).toBeInTheDocument();
    expect(screen.getByText("Face")).toBeInTheDocument();
    expect(screen.getByLabelText("Face color")).toBeInTheDocument();
    expect(screen.getByLabelText("Face opacity")).toBeInTheDocument();
    expect(screen.queryByText("Under-constrained · DOF")).toBeNull();
  });

  it("shows the owning sketch when a filled region is selected", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() =>
      selectionStore.getState().set([
        {
          kind: "sketchRegion",
          id: '["sketch2","r_profile"]',
          sketchId: "sketch2",
          regionId: "r_profile",
        },
      ]),
    );

    expect(screen.getByText("Sketch 2")).toBeInTheDocument();
    expect(screen.getByText("Sketch profile")).toBeInTheDocument();
    expect(screen.getByText("Under-constrained · DOF 3")).toBeInTheDocument();
  });

  it("shows the EMPTY state when nothing is selected", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => selectionStore.getState().clear());
    expect(screen.getByText("Nothing selected")).toBeInTheDocument();
  });

  it("renders structured failure detail in the inspector and edits for retry", () => {
    const editEdgeOpFeature = vi.fn(() => Promise.resolve());
    setModelToolController({ editEdgeOpFeature } as unknown as ModelToolController);
    try {
      documentStore.setState({
        features: [{
          id: "f-failed", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm",
          status: "error", statusMessage: "kernel refused",
          diagnostics: [{ severity: "error", code: "FILLET_TOO_LARGE", stage: "build", message: "radius exceeds edge", evidence: { radius: 11 } }],
        }],
      });
      renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
      act(() => selectionStore.getState().set([{ kind: "feature", id: "f-failed" }]));

      expect(screen.getByText("FILLET_TOO_LARGE")).toBeInTheDocument();
      expect(screen.getByText("Stage: build")).toBeInTheDocument();
      expect(screen.getByTestId("operation-diagnostic-evidence-0")).toHaveTextContent('"radius": 11');
      fireEvent.click(screen.getByTestId("feature-edit-retry"));
      expect(editEdgeOpFeature).toHaveBeenCalledWith("f-failed", "Fillet");
    } finally {
      setModelToolController(null);
    }
  });

  it("renders a warning diagnostic for an ok feature, framed with warn tokens", () => {
    documentStore.setState({
      features: [{
        id: "f-warn", kind: "revolve", opType: "Revolve", label: "Revolve", valueText: "90°",
        status: "ok",
        diagnostics: [{ severity: "warning", code: "REGION_REBOUND_BY_ANCHOR", message: "profile re-bound via region anchor" }],
      }],
    });
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => selectionStore.getState().set([{ kind: "feature", id: "f-warn" }]));

    expect(screen.getByTestId("feature-diagnostic-section")).toBeInTheDocument();
    expect(screen.getByText("REGION_REBOUND_BY_ANCHOR")).toBeInTheDocument();
    expect(screen.getByText("profile re-bound via region anchor")).toBeInTheDocument();
    expect(screen.queryByTestId("feature-edit-retry")).toBeNull();
  });

  it("renders nothing extra for an ok feature with no diagnostics", () => {
    documentStore.setState({
      features: [{ id: "f-clean", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "5 mm", status: "ok" }],
    });
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => selectionStore.getState().set([{ kind: "feature", id: "f-clean" }]));

    expect(screen.queryByTestId("feature-diagnostic-section")).toBeNull();
    expect(screen.queryByTestId("operation-diagnostics")).toBeNull();
  });

  it("shows the SKETCH state (DOF card + one row per live constraint) in sketch mode", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => {
      toolStore.getState().setMode("sketch", "sketch2");
      // Live sketch session drives the CONSTRAINTS panel (no hardcoded demo data).
      sketchStore.getState().setSession(
        sessionWithConstraints([
          { id: "c1", type: "Coincident", entities: ["p1", "p2"] },
          { id: "c2", type: "Coincident", entities: ["p3", "p4"] },
          { id: "c3", type: "Coincident", entities: ["p5", "p6"] },
          { id: "c4", type: "Coincident", entities: ["p7", "p8"] },
          { id: "c5", type: "Horizontal", entities: ["l1"] },
          { id: "c6", type: "Distance", entities: ["p1", "p2"], value: 90 },
          { id: "c7", type: "Angle", entities: ["l1", "l2"], value: 30 },
        ]),
      );
    });

    expect(screen.getByText("Sketch 2")).toBeInTheDocument();
    expect(screen.getByText("Under-constrained · DOF 3")).toBeInTheDocument();
    // Constraint discovery lives here now too (moved off the top chrome bar).
    expect(screen.getByRole("button", { name: "Add constraint" })).toBeInTheDocument();
    expect(screen.getByText("Constraints")).toBeInTheDocument();
    // One row per constraint now (grouping/counting is gone) — 4 Coincident rows
    // plus Horizontal/Distance/Angle, each its own row with its own value column.
    expect(screen.getAllByTestId(/^constraint-row-/)).toHaveLength(7);
    expect(screen.getAllByText("Coincident")).toHaveLength(4);
    expect(screen.getByText("Horizontal")).toBeInTheDocument();
    expect(screen.getByText("Distance")).toBeInTheDocument();
    expect(screen.getByText("90 mm")).toBeInTheDocument();
    expect(screen.getByText("Angle")).toBeInTheDocument();
    expect(screen.getByText("30°")).toBeInTheDocument();
    expect(
      screen.getByText(/degrees of freedom remain/),
    ).toBeInTheDocument();
  });

  it("shows the empty-constraints hint when the sketch has none", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => {
      toolStore.getState().setMode("sketch", "sketch2");
      sketchStore.getState().setSession(sessionWithConstraints([]));
    });
    expect(screen.getByText("No constraints yet.")).toBeInTheDocument();
    expect(screen.queryByText("Coincident")).toBeNull();
  });

  /*
   * Design item 12 / audit A11a — a BLANK sketch (zero entities) must not
   * read as "Fully constrained · DOF 0 — Sketch is fully defined.", the false
   * completeness claim `sketches['sketch2']`'s registry dof/status (0/"ok")
   * would otherwise produce through the ordinary status path.
   */
  it("shows 'Empty sketch' / 'Draw geometry to begin.' for a session with zero entities", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => {
      documentStore.getState().setSketchSolve("sketch2", 0, "ok");
      toolStore.getState().setMode("sketch", "sketch2");
      sketchStore.getState().setSession({
        sketchId: "sketch2",
        plane: { kind: "XY", origin: [0, 0, 0], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
        entities: [],
        constraints: [],
        dof: 0,
        status: "FullyConstrained",
      });
    });

    expect(screen.getByText("Empty sketch")).toBeInTheDocument();
    expect(screen.getByText("Draw geometry to begin.")).toBeInTheDocument();
    expect(screen.queryByText(/Fully constrained/)).toBeNull();
    expect(screen.queryByText(/Sketch is fully defined/)).toBeNull();
  });

  /**
   * Re-edit routing runs off the authored `opType`, not `kind` — the backend folds
   * Chamfer into kind "fillet", so routing on `kind` alone opened a Chamfer in the
   * FILLET editor. Both rows below share kind "fillet" and must diverge here.
   */
  it("routes a history double-click on opType, so a Chamfer opens the chamfer editor", () => {
    const editEdgeOpFeature = vi.fn(() => Promise.resolve());
    setModelToolController({ editEdgeOpFeature } as unknown as ModelToolController);
    try {
      documentStore.setState({
        features: [
          { id: "f-ch", kind: "fillet", opType: "Chamfer", label: "Chamfer", valueText: "1.0 mm", status: "ok" },
          { id: "f-fi", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
        ],
      });
      renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
      act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));

      fireEvent.doubleClick(screen.getByTestId("history-row-f-ch"));
      expect(editEdgeOpFeature).toHaveBeenCalledWith("f-ch", "Chamfer");

      fireEvent.doubleClick(screen.getByTestId("history-row-f-fi"));
      expect(editEdgeOpFeature).toHaveBeenLastCalledWith("f-fi", "Fillet");
    } finally {
      setModelToolController(null);
    }
  });

  it("routes a Boolean row's double-click to the operation-swap re-edit", () => {
    const editBooleanFeature = vi.fn(() => Promise.resolve());
    setModelToolController({ editBooleanFeature } as unknown as ModelToolController);
    try {
      documentStore.setState({
        features: [{ id: "f-bool", kind: "boolean", opType: "Boolean", label: "Union", valueText: "", status: "ok" }],
      });
      renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
      act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));

      fireEvent.doubleClick(screen.getByTestId("history-row-f-bool"));
      expect(editBooleanFeature).toHaveBeenCalledWith("f-bool");
    } finally {
      setModelToolController(null);
    }
  });

  /**
   * An ImportStep row buckets under kind "boolean" in the projection (interim), so
   * routing on `kind` would have sent a double-click into the BOOLEAN editor — a
   * tool armed against inputs an import does not have. It must open no editor and
   * say why instead.
   */
  it("routes an ImportStep row's double-click to a no-op hint, not an editor", () => {
    const editBooleanFeature = vi.fn(() => Promise.resolve());
    setModelToolController({ editBooleanFeature } as unknown as ModelToolController);
    try {
      documentStore.setState({
        features: [
          { id: "f-imp", kind: "boolean", opType: "ImportStep", label: "Import", valueText: "", status: "ok" },
        ],
      });
      renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
      act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));

      fireEvent.doubleClick(screen.getByTestId("history-row-f-imp"));

      expect(editBooleanFeature).not.toHaveBeenCalled();
      expect(viewportStore.getState().statusHint?.message).toBe(
        "Imported feature — re-import to update",
      );
    } finally {
      setModelToolController(null);
    }
  });

  /**
   * The dim state rides `FeatureMeta.suppressed` (backend-authoritative), NOT a
   * frontend overlay. The retired overlay started empty on every document open, so
   * a persisted suppression rendered un-dimmed and its toggle computed
   * `!undefined === true` — re-suppressing an already-suppressed feature forever.
   */
  it("dims a suppressed row straight from the projection flag", () => {
    documentStore.setState({
      features: [
        { id: "f-a", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "5.0 mm", status: "ok" },
        {
          id: "f-b",
          kind: "fillet",
          opType: "Fillet",
          label: "Fillet",
          valueText: "1.0 mm",
          status: "dirty",
          suppressed: true,
        },
      ],
    });
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => selectionStore.getState().set([{ kind: "feature", id: "f-a" }]));

    expect(screen.getByTestId("history-row-f-b").className).toContain("opacity-60");
    expect(screen.getByText("Fillet").className).toContain("line-through");
    expect(screen.getByTestId("history-row-f-a").className).not.toContain("opacity-60");
  });

  it("un-suppresses a persisted suppression (toggle negates the feature's OWN flag)", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    try {
      documentStore.setState({
        features: [
          {
            id: "f-b",
            kind: "fillet",
            opType: "Fillet",
            label: "Fillet",
            valueText: "1.0 mm",
            status: "dirty",
            suppressed: true,
          },
        ],
      });
      renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
      act(() => selectionStore.getState().set([{ kind: "feature", id: "f-b" }]));

      await act(async () => {
        fireEvent.click(screen.getByTestId("history-suppress-f-b"));
      });

      // suppressed:false — the un-suppress. Cascade stays off when un-suppressing.
      expect(apply.mock.calls[0][0]).toEqual({
        cmd: "setOperationSuppression",
        record: "f-b",
        suppressed: false,
        cascade: false,
      });
    } finally {
      apply.mockRestore();
    }
  });

  /*
   * H3 — the inline value editor on a past feature's history row. The gates are
   * unit-tested in `featureValueEdit.test.ts`; these pin that the PANEL feeds them
   * live store values (a `getState()` read would leave a field editable after the
   * tool armed) and that a commit reaches the backend as one params patch.
   */
  async function timelineForInlineEdit(): Promise<void> {
    // Drain first, for LONGER than one mock command latency: a previous test's
    // edit command resolves on a timer and hydrates the store from ITS timeline,
    // which would otherwise land on top of this fixture mid-assertion (a real,
    // reproducible cross-test clobber).
    await act(async () => {
      await new Promise((r) => setTimeout(r, getMockLatency() + 60));
    });
    documentStore.setState({
      appliedOps: 2,
      features: [
        { id: "f-a", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "5.0 mm", primaryValue: 5, primaryValueKind: "length", status: "ok" },
        { id: "f-b", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "1.0 mm", primaryValue: 1, primaryValueKind: "length", status: "ok" },
        // Beyond the rollback bar (appliedOps = 2) ⇒ read-only.
        { id: "f-c", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "9.0 mm", primaryValue: 9, primaryValueKind: "length", status: "dirty" },
      ],
    });
  }

  // 15 s test budget so the two 4 s waits above cannot be guillotined by the 5 s
  // default before they have had their say.
  it("commits an inline value edit as ONE params patch on the stored op", async () => {
    await timelineForInlineEdit();
    const params = vi
      .spyOn(mockClient, "getOperationParams")
      .mockResolvedValue({ distance: { value: 5 }, profile: { sketchId: "s1", regionId: "r0" } });
    // Stubbed rather than called through: the real mock command resolves on a TIMER
    // and hydrates the store from ITS OWN timeline, which under parallel load lands
    // inside a later test and replaces that test's fixture (a real flake, seen).
    const apply = vi.spyOn(mockClient, "applyEditCommand").mockResolvedValue({
      revision: 99,
      changedBodies: [],
      removedBodies: [],
      features: documentStore.getState().features.map((f) =>
        f.id === "f-a" ? { ...f, valueText: "30.0 mm", primaryValue: 30 } : f,
      ) as never,
      // `terminal` is the verdict `commitFeatureValue` reads (`types.ts`: no
      // production result may omit it). This fixture changes no body, so the
      // honest stamp is `noop` — without it the body-count fallback reads a
      // successful value edit as a failure and the store is never hydrated.
      terminal: "noop",
    });
    try {
      renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
      act(() => selectionStore.getState().set([{ kind: "feature", id: "f-a" }]));

      fireEvent.click(screen.getByTestId("history-value-f-a"));
      // The field opens one double-click threshold later (HistoryList keeps the
      // row's dblclick re-edit alive), so await it rather than reading it back.
      const input = await screen.findByLabelText("Dimension value");
      // TWO act scopes, deliberately. `DimensionInput.commit()` reads the `text`
      // STATE and only calls `onCommit` when the formatted new value differs
      // from the current one — so Enter must run against a component that has
      // already re-rendered with "30". Firing both inside ONE act nests their
      // flushes into the outer scope, letting the keyDown handler close over the
      // pre-change text; `formatValue(n) === formatValue(value)` then holds and
      // the commit is correctly skipped, so `applyEditCommand` is never called.
      // That is a real race, not slow code: it went red in CI with a 4 s budget
      // on an assertion whose work is microtasks.
      await act(async () => {
        fireEvent.change(input, { target: { value: "30" } });
      });
      await act(async () => {
        fireEvent.keyDown(input, { key: "Enter" });
      });
      // The commit is fire-and-forget from the row, so wait for it to LAND rather
      // than assume one microtask flush drained both round-trips.
      //
      // Settled by TURNS, not by a deadline. Both round-trips are
      // `mockResolvedValue` stubs, so the commit is pure microtasks and lands in
      // well under a millisecond on an idle machine — but `vi.waitFor` budgets
      // WALL-CLOCK, and vitest runs files in parallel workers, so a starved event
      // loop blew the budget while the assertion itself was fine. This test went
      // red in CI at a 1 s budget, was raised to 4 s, and went red AGAIN. Raising
      // it further only makes the red rarer. `settleUntil` counts event-loop turns
      // instead, which is a property of the code rather than of the machine.
      // One REAL macrotask per turn, inside `act` so renders scheduled by the
      // resolved promises are flushed rather than warned about.
      const settle = {
        turn: () => act(async () => void (await new Promise((r) => setTimeout(r, 0)))),
      };
      await settleUntil(() => expect(apply).toHaveBeenCalled(), settle);
      await settleUntil(
        () =>
          expect(documentStore.getState().features.find((f) => f.id === "f-a")?.primaryValue).toBe(30),
        settle,
      );

      expect(apply.mock.calls[0][0]).toEqual({
        cmd: "updateOperationParams",
        record: "f-a",
        // The profile survives the patch — only `distance` moved.
        op: { opType: "Extrude", params: { distance: { value: 30 }, profile: { sketchId: "s1", regionId: "r0" } } },
      });
    } finally {
      params.mockRestore();
      apply.mockRestore();
    }
  }, 15_000);

  it("GUARD — arming a model tool turns every row's value read-only, live", async () => {
    await timelineForInlineEdit();
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => selectionStore.getState().set([{ kind: "feature", id: "f-a" }]));
    fireEvent.click(screen.getByTestId("history-value-f-a"));
    expect(await screen.findByLabelText("Dimension value")).toBeInTheDocument();

    // Arm Extrude WITHOUT re-selecting: the panel must re-render off the store.
    act(() => toolStore.getState().setTool("extrude"));
    expect(screen.queryByLabelText("Dimension value")).toBeNull();
    fireEvent.click(screen.getByTestId("history-value-f-a"));
    await new Promise((r) => setTimeout(r, 300));
    expect(screen.queryByLabelText("Dimension value")).toBeNull();
  });

  it("GUARD — a row past the rollback cursor is read-only", async () => {
    await timelineForInlineEdit();
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => selectionStore.getState().set([{ kind: "feature", id: "f-a" }]));
    fireEvent.click(screen.getByTestId("history-value-f-c"));
    await new Promise((r) => setTimeout(r, 300));
    expect(screen.queryByLabelText("Dimension value")).toBeNull();
  });

  it("GUARD — a suppressed row is read-only but still SHOWS its value", async () => {
    await timelineForInlineEdit();
    documentStore.setState({
      features: documentStore
        .getState()
        .features.map((f) => (f.id === "f-b" ? { ...f, suppressed: true } : f)),
    });
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => selectionStore.getState().set([{ kind: "feature", id: "f-a" }]));
    expect(screen.getByTestId("history-value-f-b")).toHaveTextContent("1 mm");
    fireEvent.click(screen.getByTestId("history-value-f-b"));
    await new Promise((r) => setTimeout(r, 300));
    expect(screen.queryByLabelText("Dimension value")).toBeNull();
  });

  /*
   * H3b — a SKETCH feature has no single primary dimension, so it gets a
   * "Dimensions" section instead: every dimensional constraint, editable, committed
   * through `setSketchDimension` (which the backend turns into a downstream regen).
   */
  it("lists a selected sketch feature's dimensions and commits an edit", async () => {
    documentStore.setState({
      appliedOps: 1,
      features: [
        { id: "f-s", kind: "sketch", opType: "Sketch", label: "Sketch 1", valueText: "", status: "ok" },
      ],
    });
    const params = vi
      .spyOn(mockClient, "getOperationParams")
      .mockResolvedValue({ sketchId: "sketch2" });
    const getSketch = vi.spyOn(mockClient, "getSketch").mockResolvedValue(
      sessionWithConstraints([
        { id: "c1", type: "Distance", entities: ["l1"], value: 60 },
        // Non-dimensional constraints have no value to offer — filtered out.
        { id: "c2", type: "Horizontal", entities: ["l1"] },
      ]),
    );
    const setDim = vi
      .spyOn(mockClient, "setSketchDimension")
      .mockResolvedValue({ sketchId: "sketch2", sketchRevision: 2, dof: 2, status: "UnderConstrained" });
    try {
      renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
      act(() => selectionStore.getState().set([{ kind: "feature", id: "f-s" }]));

      // The section loads over TWO round-trips (params → sketch), so await the
      // rendered result rather than a fixed number of microtask flushes.
      expect(await screen.findByText("Dimensions")).toBeInTheDocument();
      expect(await screen.findByTestId("sketch-dimension-c1")).toBeInTheDocument();
      expect(screen.queryByTestId("sketch-dimension-c2")).toBeNull();

      const input = screen.getByLabelText("Dimension value");
      await act(async () => {
        fireEvent.change(input, { target: { value: "75" } });
        fireEvent.keyDown(input, { key: "Enter" });
      });
      // The TYPE rides along: it is what decides the wire domain (deg→rad for an
      // Angle) inside the tauri client's `toWireDimensionValue` call.
      expect(setDim).toHaveBeenCalledWith("sketch2", "c1", "Distance", 75);
    } finally {
      params.mockRestore();
      getSketch.mockRestore();
      setDim.mockRestore();
    }
  });

  it("wires each row's delete button to deleteConstraints (re-solves + drops the row)", async () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    act(() => {
      toolStore.getState().setMode("sketch", "sketch2");
      sketchStore.getState().setSession(
        sessionWithConstraints([
          { id: "c1", type: "Coincident", entities: ["p1", "p2"] },
          { id: "c2", type: "Horizontal", entities: ["l1"] },
        ]),
      );
    });
    expect(screen.getByTestId("constraint-row-c2")).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByTestId("constraint-delete-c2"));
      // Drain the ACTUAL mutation queue rather than racing a fixed macrotask
      // (flaky under parallel worker-pool load — see useShortcuts.test.tsx).
      await flushSketchMutations();
    });

    expect(sketchStore.getState().session!.constraints.map((c) => c.id)).toEqual(["c1"]);
    expect(screen.queryByTestId("constraint-row-c2")).toBeNull();
  });

  /*
   * H10 — the inspector's "Depends on" / "Used by" section, read off
   * `client.featureDependencies` and mapped through the live `features` list for
   * labels. Also pins the delete/suppress dependent-count wiring at the panel
   * level (the row/menu mechanics themselves are covered in HistoryList.test.tsx).
   */
  describe("H10 — dependency view", () => {
    function timelineWithDependency(): void {
      documentStore.setState({
        features: [
          { id: "f-a", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "5.0 mm", status: "ok" },
          { id: "f-b", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "1.0 mm", status: "ok" },
        ],
      });
    }

    it("lists Depends on / Used by with feature labels, and a click selects that feature", async () => {
      timelineWithDependency();
      const deps = vi.spyOn(mockClient, "featureDependencies").mockImplementation(async (id) =>
        id === "f-b" ? { upstream: ["f-a"], downstream: [] } : { upstream: [], downstream: ["f-b"] },
      );
      try {
        renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
        act(() => selectionStore.getState().set([{ kind: "feature", id: "f-b" }]));

        expect(await screen.findByText("Depends on")).toBeInTheDocument();
        const row = screen.getByTestId("feature-dep-upstream-f-a");
        expect(row).toHaveTextContent("Extrude");
        expect(screen.queryByText("Used by")).toBeNull(); // f-b's OWN downstream is empty

        fireEvent.click(row);
        expect(selectionStore.getState().selected).toEqual([{ kind: "feature", id: "f-a" }]);
      } finally {
        deps.mockRestore();
      }
    });

    it("shows Used by (downstream) for an upstream feature and hides Depends on", async () => {
      timelineWithDependency();
      const deps = vi
        .spyOn(mockClient, "featureDependencies")
        .mockResolvedValue({ upstream: [], downstream: ["f-b"] });
      try {
        renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
        act(() => selectionStore.getState().set([{ kind: "feature", id: "f-a" }]));

        expect(await screen.findByText("Used by")).toBeInTheDocument();
        expect(screen.getByTestId("feature-dep-downstream-f-b")).toHaveTextContent("Fillet");
        expect(screen.queryByText("Depends on")).toBeNull();
      } finally {
        deps.mockRestore();
      }
    });

    it("renders nothing when both directions are empty", async () => {
      timelineWithDependency();
      const deps = vi
        .spyOn(mockClient, "featureDependencies")
        .mockResolvedValue({ upstream: [], downstream: [] });
      try {
        renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
        act(() => selectionStore.getState().set([{ kind: "feature", id: "f-a" }]));

        await vi.waitFor(() => expect(deps).toHaveBeenCalled());
        expect(screen.queryByText("Depends on")).toBeNull();
        expect(screen.queryByText("Used by")).toBeNull();
      } finally {
        deps.mockRestore();
      }
    });

    it("falls back to the bare id for a dependency the current features list no longer carries", async () => {
      timelineWithDependency();
      const deps = vi
        .spyOn(mockClient, "featureDependencies")
        .mockResolvedValue({ upstream: ["ghost-id"], downstream: [] });
      try {
        renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
        act(() => selectionStore.getState().set([{ kind: "feature", id: "f-b" }]));

        expect(await screen.findByTestId("feature-dep-upstream-ghost-id")).toHaveTextContent(
          "ghost-id",
        );
      } finally {
        deps.mockRestore();
      }
    });

    it("shows the dependent count on the delete confirm once the row cluster is hovered", async () => {
      timelineWithDependency();
      const deps = vi
        .spyOn(mockClient, "featureDependencies")
        .mockResolvedValue({ upstream: [], downstream: ["f-b"] });
      try {
        renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
        act(() => selectionStore.getState().set([{ kind: "feature", id: "f-a" }]));
        await screen.findByText("Used by"); // the panel's own fetch landed

        fireEvent.mouseEnter(screen.getByTestId("history-suppress-f-a").parentElement!);
        fireEvent.click(screen.getByTestId("history-delete-f-a"));
        await screen.findByTitle("Confirm delete — 1 dependent");
      } finally {
        deps.mockRestore();
      }
    });
  });
});
