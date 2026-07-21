import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ConstraintList } from "./ConstraintList";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import type { SketchConstraint } from "@/ipc/types";

const constraints: SketchConstraint[] = [
  { id: "c1", type: "Coincident", entities: ["p1", "p2"] },
  { id: "c2", type: "Horizontal", entities: ["l1"] },
  { id: "c3", type: "Distance", entities: ["p1", "p2"], value: 90 },
  { id: "c4", type: "Angle", entities: ["l1", "l2"], value: 45 },
  { id: "c5", type: "Coincident", entities: ["p1", "p2", "p3", "p4"] },
];

describe("ConstraintList", () => {
  beforeEach(() => {
    sketchSelectionStore.setState({ selected: [], hover: null, constraintHover: null });
  });

  it("renders one row per constraint with its glyph, type label, and entity summary", () => {
    render(<ConstraintList constraints={constraints} onDelete={() => {}} />);
    expect(screen.getByTestId("constraint-row-c1")).toBeInTheDocument();
    expect(screen.getByTestId("constraint-row-c2")).toBeInTheDocument();
    expect(screen.getAllByText("Coincident")).toHaveLength(2);
    expect(screen.getByText("Horizontal")).toBeInTheDocument();
    expect(screen.getAllByText("p1, p2")).toHaveLength(2); // c1 (Coincident) + c3 (Distance)
    expect(screen.getByText("l1")).toBeInTheDocument();
  });

  it("compacts entity summaries beyond 2 refs to first-id + count", () => {
    render(<ConstraintList constraints={constraints} onDelete={() => {}} />);
    expect(screen.getByText("p1 +3")).toBeInTheDocument();
  });

  it("shows a value column for dimensional kinds, Angle in degrees with °", () => {
    render(<ConstraintList constraints={constraints} onDelete={() => {}} />);
    expect(screen.getByText("90.0")).toBeInTheDocument();
    expect(screen.getByText("45.0°")).toBeInTheDocument();
  });

  it("does not render a value for non-dimensional kinds", () => {
    render(<ConstraintList constraints={[constraints[0]]} onDelete={() => {}} />);
    expect(screen.queryByText(/^\d+\.\d$/)).toBeNull();
  });

  it("row mouseenter/mouseleave drive sketchSelectionStore.constraintHover", () => {
    render(<ConstraintList constraints={constraints} onDelete={() => {}} />);
    fireEvent.mouseEnter(screen.getByTestId("constraint-row-c2"));
    expect(sketchSelectionStore.getState().constraintHover).toBe("c2");
    fireEvent.mouseLeave(screen.getByTestId("constraint-row-c2"));
    expect(sketchSelectionStore.getState().constraintHover).toBeNull();
  });

  it("delete button calls onDelete with that row's constraint id", () => {
    const onDelete = vi.fn();
    render(<ConstraintList constraints={constraints} onDelete={onDelete} />);
    fireEvent.click(screen.getByTestId("constraint-delete-c3"));
    expect(onDelete).toHaveBeenCalledTimes(1);
    expect(onDelete).toHaveBeenCalledWith("c3");
  });

  it("renders no rows for an empty constraint set", () => {
    render(<ConstraintList constraints={[]} onDelete={() => {}} />);
    expect(screen.queryByTestId(/constraint-row-/)).toBeNull();
  });

  it("tints a conflicting row with the traffic-close token; clean rows are untinted", () => {
    render(<ConstraintList constraints={constraints} onDelete={() => {}} conflictingIds={["c1"]} />);
    const conflicting = screen.getByTestId("constraint-row-c1");
    expect(conflicting).toHaveAttribute("data-conflicting", "true");
    // The glyph + type label carry the traffic-close class (no raw hex).
    expect(conflicting.querySelector(".text-traffic-close")).not.toBeNull();
    // An unrelated row is not tinted.
    const clean = screen.getByTestId("constraint-row-c2");
    expect(clean).not.toHaveAttribute("data-conflicting");
    expect(clean.querySelector(".text-traffic-close")).toBeNull();
  });

  it("no rows are tinted when conflictingIds is empty / omitted", () => {
    render(<ConstraintList constraints={constraints} onDelete={() => {}} />);
    for (const c of constraints) {
      expect(screen.getByTestId(`constraint-row-${c.id}`)).not.toHaveAttribute("data-conflicting");
    }
  });
});
