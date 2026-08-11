import { beforeEach, describe, it, expect, vi } from "vitest";
import { screen, fireEvent, act, waitFor } from "@testing-library/react";
import { InspectorPanel } from "@/features/inspector/InspectorPanel";
import { repairStore } from "@/stores/repairStore";
import { mockClient } from "@/ipc/mockClient";
import { documentStore } from "@/stores/documentStore";
import { resetStores } from "@/test/resetStores";
import { renderWithPlatform } from "@/test/renderWithPlatform";
import { contributeInspectorSections } from "@/modules/modeling/inspectorSections";
import type { NeedsRepairEvent } from "@/ipc/types";

const oneItem = (opId: string, refId: string): NeedsRepairEvent => ({
  revision: 7,
  snapshotId: 700,
  items: [{ opId, refId, reason: "ambiguous", scoringVersion: 1, candidateCount: 2 }],
});

function openRepair(opId: string, refId: string): void {
  act(() => {
    repairStore.getState().applyEvent(oneItem(opId, refId));
    repairStore.getState().openPanel();
  });
}

beforeEach(() => resetStores());

describe("RepairPanel (inspector repair state)", () => {
  it("renders the repair panel with the feature label from the projection", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    openRepair("f3", "f3.input0"); // f3 = the seeded Fillet feature
    expect(screen.getByText("Repair references")).toBeInTheDocument();
    expect(screen.getByText("Fillet")).toBeInTheDocument();
    expect(screen.getByText(/2 candidates/)).toBeInTheDocument();
  });

  it("falls back to an opId prefix when the feature is not in the projection", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    openRepair("op_deadbeef99", "op_deadbeef99.input0");
    expect(screen.getByText(/Feature op_dea/)).toBeInTheDocument();
  });

  it("expanding an item calls resolveRefs and renders candidates sorted by score", async () => {
    const spy = vi.spyOn(mockClient, "resolveRefs");
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    openRepair("f3", "f3.input0");

    fireEvent.click(screen.getByTestId("repair-item-head-f3.input0"));
    // H9: the slot's InputPath is resolved FIRST (a slot with no path never
    // fetches candidates), so the resolveRefs call is one tick behind the click.
    await waitFor(() =>
      expect(spy).toHaveBeenCalledWith([{ refId: "f3.input0", snapshotId: 700, revision: 7 }]),
    );

    // The canned candidates arrive (mock latency) — highest score first.
    await screen.findByText("91%");
    expect(screen.getByText("89%")).toBeInTheDocument();
    const pcts = screen.getAllByText(/%$/).map((n) => n.textContent);
    expect(pcts).toEqual(["91%", "89%"]);
    spy.mockRestore();
  });

  it("choosing a candidate promotes it then sends an EditOperationInput rebind", async () => {
    const promote = vi.spyOn(mockClient, "promoteSelection");
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    openRepair("f3", "f3.input0");

    fireEvent.click(screen.getByTestId("repair-item-head-f3.input0"));
    const candidates = await screen.findAllByTestId(/^repair-candidate-f3\.input0-/);
    fireEvent.click(candidates[0]); // highest-score candidate

    await waitFor(() => expect(promote).toHaveBeenCalled());
    await waitFor(() => expect(apply).toHaveBeenCalled());
    // Promotion carries the candidate topoKey + worldPos anchor, on the seed body.
    expect(promote.mock.calls[0][0]).toBe("body1");
    expect(promote.mock.calls[0][1][0]).toMatchObject({ anchor: { worldPoint: [12, 3.5, 0] } });
    // The rebind targets the fillet op + the slot index parsed from the refId (0).
    expect(apply.mock.calls[0][0]).toMatchObject({
      cmd: "editOperationInput",
      record: "f3",
      path: { path: "filletEdges", index: 0 },
    });
    promote.mockRestore();
    apply.mockRestore();
  });

  it("drops a late candidate load after a newer repair event", async () => {
    let finish: ((value: Awaited<ReturnType<typeof mockClient.resolveRefs>>) => void) | undefined;
    const resolve = vi.spyOn(mockClient, "resolveRefs").mockImplementation(
      () =>
        new Promise((done: (value: Awaited<ReturnType<typeof mockClient.resolveRefs>>) => void) => {
          finish = done;
        }),
    );
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    openRepair("f3", "f3.input0");
    fireEvent.click(screen.getByTestId("repair-item-head-f3.input0"));
    await waitFor(() => expect(resolve).toHaveBeenCalled());

    act(() =>
      repairStore.getState().applyEvent({
        revision: 8,
        snapshotId: 800,
        items: [{ opId: "f3", refId: "f3.input0", reason: "ambiguous", candidateCount: 1 }],
      }),
    );
    finish?.([
      {
        snapshotId: 700,
        revision: 7,
        refId: "f3.input0",
        bodyId: "body1",
        outcome: "needsRepair",
        candidates: [
          { topoKey: "e:7", score: 0.91, margin: 0.02, worldPos: [12, 3.5, 0], summary: "edge" },
        ],
      },
    ]);

    await waitFor(() => expect(screen.queryByTestId("repair-candidate-f3.input0-e:7")).toBeNull());
    resolve.mockRestore();
  });

  it("does not promote a row retained from an older repair event", async () => {
    const promote = vi.spyOn(mockClient, "promoteSelection");
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    openRepair("f3", "f3.input0");
    fireEvent.click(screen.getByTestId("repair-item-head-f3.input0"));
    const candidate = (await screen.findAllByTestId(/^repair-candidate-f3\.input0-/))[0];

    act(() =>
      repairStore.getState().applyEvent({
        revision: 8,
        snapshotId: 800,
        items: [{ opId: "f3", refId: "f3.input0", reason: "ambiguous", candidateCount: 1 }],
      }),
    );
    fireEvent.click(candidate);

    expect(promote).not.toHaveBeenCalled();
    promote.mockRestore();
  });

  it("the close affordance dismisses the panel", () => {
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    openRepair("f3", "f3.input0");
    fireEvent.click(screen.getByTestId("repair-close"));
    expect(repairStore.getState().panelOpen).toBe(false);
  });
});

// ── HISTORY-HARDEN H9 ────────────────────────────────────────────────────────

describe("RepairPanel — a slot with no rebind path (H9)", () => {
  it("says so instead of offering candidates that can only be rejected", async () => {
    // A Hole's `inputs[0]` is the HOST BODY — a repair candidate is an element
    // TopoKey, so nothing could be written there. Pre-H9 the panel showed
    // clickable candidates and sent `filletEdges{0}`, which the core rejected.
    documentStore.setState({
      features: [
        { id: "h1", kind: "extrude", opType: "Hole", label: "Hole", valueText: "", status: "needsRepair" },
      ],
    });
    const spy = vi.spyOn(mockClient, "resolveRefs");
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    openRepair("h1", "h1.input0");

    fireEvent.click(screen.getByTestId("repair-item-head-h1.input0"));
    await screen.findByTestId("repair-not-rebindable-h1.input0");
    expect(
      screen.getByText("This reference must be re-picked in the feature editor"),
    ).toBeInTheDocument();
    // …and it never even asked for candidates.
    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });

  it("still offers candidates for a slot that IS rebindable (the hole's host FACE)", async () => {
    documentStore.setState({
      features: [
        { id: "h1", kind: "extrude", opType: "Hole", label: "Hole", valueText: "", status: "needsRepair" },
      ],
    });
    renderWithPlatform(<InspectorPanel />, { contribute: contributeInspectorSections });
    openRepair("h1", "h1.input1"); // slot 1 = host face
    fireEvent.click(screen.getByTestId("repair-item-head-h1.input1"));
    await screen.findByText("91%");
    expect(screen.queryByTestId("repair-not-rebindable-h1.input1")).not.toBeInTheDocument();
  });
});
