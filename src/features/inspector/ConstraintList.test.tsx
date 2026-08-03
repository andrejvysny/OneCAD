import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ConstraintList, visibleConstraints } from "./ConstraintList";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import type { SketchConstraint, SketchEntity } from "@/ipc/types";

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

  it("shows a value column for dimensional kinds, lengths with the display unit, Angle in degrees with °", () => {
    render(<ConstraintList constraints={constraints} onDelete={() => {}} />);
    expect(screen.getByText("90 mm")).toBeInTheDocument();
    expect(screen.getByText("45°")).toBeInTheDocument();
  });

  it("does not render a value for non-dimensional kinds", () => {
    render(<ConstraintList constraints={[constraints[0]]} onDelete={() => {}} />);
    expect(screen.queryByText(/^-?\d+(\.\d+)?°?$/)).toBeNull();
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

  // ── Machine `Fixed` rows from a host-face projection (SKETCH-ON-FACE W2) ──

  describe("locked reference geometry", () => {
    const locked: SketchEntity[] = [
      { id: "ref1", type: "Line", p0: [0, 0], p1: [40, 0], referenceLocked: true },
      { id: "ref2", type: "Line", p0: [40, 0], p1: [40, 30], referenceLocked: true },
    ];
    const user: SketchEntity[] = [{ id: "l1", type: "Line", p0: [5, 5], p1: [20, 5] }];
    const rows: SketchConstraint[] = [
      { id: "fx1", type: "Fixed", entities: ["ref1"], positions: ["Start"] },
      { id: "fx2", type: "Fixed", entities: ["ref2"], positions: ["Start"] },
      { id: "u1", type: "Fixed", entities: ["l1"], positions: ["Start"] }, // the USER pinned their own point
      { id: "u2", type: "Horizontal", entities: ["l1"] },
    ];

    it("hides the machine Fixed rows pinning locked geometry, keeps everything else", () => {
      render(<ConstraintList constraints={rows} entities={[...locked, ...user]} onDelete={() => {}} />);
      expect(screen.queryByTestId("constraint-row-fx1")).toBeNull();
      expect(screen.queryByTestId("constraint-row-fx2")).toBeNull();
      // A Fixed the USER authored on their OWN geometry is not machine noise.
      expect(screen.getByTestId("constraint-row-u1")).toBeInTheDocument();
      expect(screen.getByTestId("constraint-row-u2")).toBeInTheDocument();
    });

    it("is a VIEW filter only — the caller's array is untouched", () => {
      // session.constraints must keep the pins: `marshalUpsert` reads a seeded
      // constraint missing from the live array as a removal and emits
      // `removeConstraint` for it, deleting the pins on the first edit.
      const before = [...rows];
      render(<ConstraintList constraints={rows} entities={[...locked, ...user]} onDelete={() => {}} />);
      expect(rows).toEqual(before);
      expect(visibleConstraints(rows, [...locked, ...user])).toHaveLength(2);
      expect(rows).toHaveLength(4);
    });

    it("with no locked entities every row renders (the pre-W2 behaviour)", () => {
      render(<ConstraintList constraints={rows} entities={user} onDelete={() => {}} />);
      for (const c of rows) expect(screen.getByTestId(`constraint-row-${c.id}`)).toBeInTheDocument();
      expect(visibleConstraints(rows, [])).toBe(rows); // identity: no copy, no filter
    });
  });
});
