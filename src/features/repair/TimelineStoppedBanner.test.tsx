/*
 * H7b — the "Timeline stopped" banner and its derivation.
 *
 * The mock lane cannot produce an ERRORED feature (every mock op commits as "ok"
 * — the mock has no kernel to fail in), so the halt surface is exercised HERE,
 * against the store the real projection hydrates, instead of in an e2e spec. What
 * an e2e would add over this is only the mount point, which `EditorScreen` pins.
 */
import { beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { TimelineStoppedBanner, timelineHalt } from "./TimelineStoppedBanner";
import { documentStore, seedMockDocument, type FeatureMeta } from "@/stores/documentStore";
import { repairStore } from "@/stores/repairStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";

const clearWorkerCircuit = vi.fn(() => Promise.resolve(3));
const applyEditCommand = vi.fn(() =>
  Promise.resolve({ revision: 9, changedBodies: [], removedBodies: [], features: [] }),
);
vi.mock("@/ipc/client", () => ({
  createClient: () => ({ clearWorkerCircuit, applyEditCommand }),
}));

const rows = (...statuses: FeatureMeta["status"][]): FeatureMeta[] =>
  statuses.map((status, i) => ({
    id: `f${i + 1}`,
    kind: "extrude" as const,
    opType: "Extrude",
    label: `Step ${i + 1}`,
    valueText: "",
    status,
  }));

beforeEach(() => {
  clearWorkerCircuit.mockClear();
  applyEditCommand.mockClear();
  repairStore.getState().reset();
  selectionStore.getState().clear();
  documentStore.getState().applySnapshot(seedMockDocument());
});

describe("timelineHalt", () => {
  it("is null for a healthy timeline", () => {
    expect(timelineHalt(rows("ok", "ok", "ok"))).toBeNull();
  });

  it("finds the FIRST error and counts the dirty tail behind it", () => {
    const halt = timelineHalt(rows("ok", "error", "dirty", "dirty"));
    expect(halt?.index).toBe(1);
    expect(halt?.feature.id).toBe("f2");
    expect(halt?.notRebuilt).toBe(2);
  });

  it("counts only DIRTY rows behind the halt — a rebuilt row is not damage", () => {
    expect(timelineHalt(rows("ok", "error", "ok", "dirty"))?.notRebuilt).toBe(1);
  });

  it("treats needsRepair as a halt too, and takes whichever comes first", () => {
    expect(timelineHalt(rows("ok", "needsRepair", "dirty"))?.index).toBe(1);
    expect(timelineHalt(rows("needsRepair", "error"))?.index).toBe(0);
  });
});

describe("TimelineStoppedBanner", () => {
  it("renders nothing while nothing has halted", () => {
    render(<TimelineStoppedBanner />);
    expect(screen.queryByTestId("timeline-stopped-banner")).toBeNull();
  });

  it("names the halt row and the blocked count", () => {
    render(<TimelineStoppedBanner />);
    act(() => documentStore.getState().applyChange({ features: rows("ok", "error", "dirty", "dirty") }));
    expect(screen.getByTestId("timeline-stopped-banner")).toHaveTextContent(
      "Timeline stopped at Step 2 — 2 features not rebuilt",
    );
  });

  it("drops the blocked clause when nothing behind it is dirty", () => {
    render(<TimelineStoppedBanner />);
    act(() => documentStore.getState().applyChange({ features: rows("ok", "error") }));
    expect(screen.getByTestId("timeline-stopped-banner")).toHaveTextContent(
      "Timeline stopped at Step 2",
    );
    expect(screen.getByTestId("timeline-stopped-banner")).not.toHaveTextContent("not rebuilt");
  });

  it("clicking the label selects the halt feature (the history row scrolls to it)", () => {
    render(<TimelineStoppedBanner />);
    act(() => documentStore.getState().applyChange({ features: rows("ok", "error", "dirty") }));
    fireEvent.click(screen.getByTestId("timeline-stopped-goto"));
    expect(selectionStore.getState().selected).toEqual([{ kind: "feature", id: "f2" }]);
  });

  it("the primary action suppresses the halting feature (with cascade)", () => {
    render(<TimelineStoppedBanner />);
    act(() => documentStore.getState().applyChange({ features: rows("ok", "error", "dirty") }));
    fireEvent.click(screen.getByTestId("timeline-stopped-suppress"));
    expect(applyEditCommand).toHaveBeenCalledWith({
      cmd: "setOperationSuppression",
      record: "f2",
      suppressed: true,
      cascade: true,
    });
  });

  it("the secondary action clears the worker circuit and reports the count", async () => {
    render(<TimelineStoppedBanner />);
    act(() => documentStore.getState().applyChange({ features: rows("error") }));
    fireEvent.click(screen.getByTestId("timeline-stopped-reset-breaker"));
    expect(clearWorkerCircuit).toHaveBeenCalled();
    await vi.waitFor(() =>
      expect(viewportStore.getState().statusHint?.message).toBe(
        "Failure breaker reset (3 blocked steps)",
      ),
    );
  });

  it("stacks below the repair pill when both are up", () => {
    render(<TimelineStoppedBanner />);
    act(() => documentStore.getState().applyChange({ features: rows("error") }));
    expect(screen.getByTestId("timeline-stopped-banner").className).toContain("top-3");
    act(() =>
      repairStore.getState().applyEvent({
        revision: 7,
        snapshotId: 700,
        items: [{ opId: "op_1", refId: "op_1.input0", reason: "ambiguous", candidateCount: 2 }],
      }),
    );
    expect(screen.getByTestId("timeline-stopped-banner").className).toContain("top-[52px]");
  });
});
