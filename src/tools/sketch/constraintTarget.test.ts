import { describe, it, expect } from "vitest";
import { toConstraintTarget, resolveTargetPoint, type SketchConstraintTargetPoint } from "./constraintTarget";
import type { SketchEntity } from "@/ipc/types";

// Fixture mirrors the C++ prototype test's sketch (proto_sketch_constraint_
// applicability.cpp:26-41), INCLUDING the ellipse (W3 P3 added the "Ellipse"
// `SketchEntityType` variant).
const entities: SketchEntity[] = [
  { id: "p1", type: "Point", p0: [0, 0] },
  { id: "p2", type: "Point", p0: [10, 0] },
  { id: "line", type: "Line", p0: [0, 0], p1: [10, 0] },
  { id: "p3", type: "Point", p0: [0, 4] },
  { id: "p4", type: "Point", p0: [10, 4] },
  { id: "line2", type: "Line", p0: [0, 4], p1: [10, 4] },
  { id: "circleCenter", type: "Point", p0: [5, 5] },
  { id: "circle", type: "Circle", center: [5, 5], radius: 2.5 },
  { id: "arcCenter", type: "Point", p0: [12, 5] },
  { id: "arc", type: "Arc", center: [12, 5], radius: 2.5, start: [14.5, 5], end: [13.3508, 7.1035] },
  { id: "ellipseCenter", type: "Point", p0: [20, 5] },
  { id: "ellipse", type: "Ellipse", center: [20, 5], majorR: 5, minorR: 2, rotation: 0 },
];

describe("toConstraintTarget", () => {
  it("returns null for a missing entity id", () => {
    expect(toConstraintTarget({ entityId: "nope" }, entities)).toBeNull();
  });

  it("a bare Point entity ⇒ point target, isFreePoint=true, position normalized to Start", () => {
    expect(toConstraintTarget({ entityId: "p1" }, entities)).toEqual({
      kind: "point",
      entityId: "p1",
      position: "Start",
      isFreePoint: true,
    });
  });

  it("a Point entity selected with point:'Center' still normalizes to Start (Start/Center alias)", () => {
    expect(toConstraintTarget({ entityId: "p1", point: "Center" }, entities)).toEqual({
      kind: "point",
      entityId: "p1",
      position: "Start",
      isFreePoint: true,
    });
  });

  it("a Point entity selected with an invalid position (End) ⇒ null", () => {
    expect(toConstraintTarget({ entityId: "p1", point: "End" }, entities)).toBeNull();
  });

  it("a bare Line entity (no point) ⇒ line target", () => {
    expect(toConstraintTarget({ entityId: "line" }, entities)).toEqual({ kind: "line", entityId: "line" });
  });

  it.each(["Start", "End", "Midpoint"] as const)("a Line's %s point ⇒ point target, isFreePoint=false", (pos) => {
    expect(toConstraintTarget({ entityId: "line", point: pos }, entities)).toEqual({
      kind: "point",
      entityId: "line",
      position: pos,
      isFreePoint: false,
    });
  });

  it("a Line selected with 'Center' (invalid for Line) ⇒ null", () => {
    expect(toConstraintTarget({ entityId: "line", point: "Center" }, entities)).toBeNull();
  });

  it("a bare Circle entity (no point) ⇒ circle target", () => {
    expect(toConstraintTarget({ entityId: "circle" }, entities)).toEqual({ kind: "circle", entityId: "circle" });
  });

  it("a Circle's Center ⇒ point target", () => {
    expect(toConstraintTarget({ entityId: "circle", point: "Center" }, entities)).toEqual({
      kind: "point",
      entityId: "circle",
      position: "Center",
      isFreePoint: false,
    });
  });

  it("a Circle selected with 'Start' (invalid for Circle) ⇒ null", () => {
    expect(toConstraintTarget({ entityId: "circle", point: "Start" }, entities)).toBeNull();
  });

  it("a bare Arc entity (no point) ⇒ arc target", () => {
    expect(toConstraintTarget({ entityId: "arc" }, entities)).toEqual({ kind: "arc", entityId: "arc" });
  });

  it.each(["Center", "Start", "End"] as const)("an Arc's %s point ⇒ point target, isFreePoint=false", (pos) => {
    expect(toConstraintTarget({ entityId: "arc", point: pos }, entities)).toEqual({
      kind: "point",
      entityId: "arc",
      position: pos,
      isFreePoint: false,
    });
  });

  it("an Arc selected with 'Midpoint' (invalid for Arc) ⇒ null", () => {
    expect(toConstraintTarget({ entityId: "arc", point: "Midpoint" }, entities)).toBeNull();
  });
});

describe("resolveTargetPoint", () => {
  it("resolves a free Point's own coordinate", () => {
    const t: SketchConstraintTargetPoint = { kind: "point", entityId: "p1", position: "Start", isFreePoint: true };
    expect(resolveTargetPoint(t, entities)).toEqual([0, 0]);
  });

  it("resolves a Line's Start / End", () => {
    const start: SketchConstraintTargetPoint = { kind: "point", entityId: "line", position: "Start", isFreePoint: false };
    const end: SketchConstraintTargetPoint = { kind: "point", entityId: "line", position: "End", isFreePoint: false };
    expect(resolveTargetPoint(start, entities)).toEqual([0, 0]);
    expect(resolveTargetPoint(end, entities)).toEqual([10, 0]);
  });

  it("resolves a Line's Midpoint as the average of Start/End", () => {
    const mid: SketchConstraintTargetPoint = { kind: "point", entityId: "line2", position: "Midpoint", isFreePoint: false };
    expect(resolveTargetPoint(mid, entities)).toEqual([5, 4]);
  });

  it("resolves a Circle's Center", () => {
    const c: SketchConstraintTargetPoint = { kind: "point", entityId: "circle", position: "Center", isFreePoint: false };
    expect(resolveTargetPoint(c, entities)).toEqual([5, 5]);
  });

  it("resolves an Arc's Center / Start / End", () => {
    const center: SketchConstraintTargetPoint = { kind: "point", entityId: "arc", position: "Center", isFreePoint: false };
    const start: SketchConstraintTargetPoint = { kind: "point", entityId: "arc", position: "Start", isFreePoint: false };
    expect(resolveTargetPoint(center, entities)).toEqual([12, 5]);
    expect(resolveTargetPoint(start, entities)).toEqual([14.5, 5]);
  });

  it("returns null for a target whose entity no longer exists", () => {
    const stale: SketchConstraintTargetPoint = { kind: "point", entityId: "gone", position: "Start", isFreePoint: true };
    expect(resolveTargetPoint(stale, entities)).toBeNull();
  });

  it("returns null for a Circle's Start (not a valid field on a circle)", () => {
    const bad: SketchConstraintTargetPoint = { kind: "point", entityId: "circle", position: "Start", isFreePoint: false };
    expect(resolveTargetPoint(bad, entities)).toBeNull();
  });
});

// ── W3 P3: Ellipse target vocabulary ─────────────────────────────────────────

describe("toConstraintTarget — Ellipse", () => {
  it("a body pick resolves to its OWN `ellipse` kind (not `circle`, not dropped)", () => {
    expect(toConstraintTarget({ entityId: "ellipse" }, entities)).toEqual({
      kind: "ellipse",
      entityId: "ellipse",
    });
  });

  it("Center is its only valid named position", () => {
    expect(toConstraintTarget({ entityId: "ellipse", point: "Center" }, entities)).toEqual({
      kind: "point",
      entityId: "ellipse",
      position: "Center",
      isFreePoint: false,
    });
    expect(toConstraintTarget({ entityId: "ellipse", point: "Start" }, entities)).toBeNull();
    expect(toConstraintTarget({ entityId: "ellipse", point: "End" }, entities)).toBeNull();
    expect(toConstraintTarget({ entityId: "ellipse", point: "Midpoint" }, entities)).toBeNull();
  });
});

describe("resolveTargetPoint — Ellipse", () => {
  const center: SketchConstraintTargetPoint = {
    kind: "point",
    entityId: "ellipse",
    position: "Center",
    isFreePoint: false,
  };

  it("resolves the Center to the ellipse's centre coord", () => {
    expect(resolveTargetPoint(center, entities)).toEqual([20, 5]);
  });

  it("resolves nothing for any other position", () => {
    expect(resolveTargetPoint({ ...center, position: "Start" }, entities)).toBeNull();
  });
});
