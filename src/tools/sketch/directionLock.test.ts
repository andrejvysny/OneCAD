/*
 * directionLock — the parity-graph admission rule (WP-2 D6 / B3).
 *
 * The four decisions, a chain deep enough that transitivity is the only way to
 * reach the verdict, the review's A-182 shape (`Horizontal(a)` + `Vertical(b)`
 * makes `Perpendicular(a, b)` redundant and `Parallel(a, b)` contradictory),
 * and order-independence of both the verdict and the witness.
 */
import { describe, it, expect } from "vitest";
import {
  admitDirectionCandidates,
  classifyDirectionCandidate,
  isDirectionConstraintType,
  orderDirectionCandidates,
  type DirectionConstraint,
} from "./directionLock";

const H = (id: string, a: string): DirectionConstraint => ({ id, type: "Horizontal", entities: [a] });
const V = (id: string, a: string): DirectionConstraint => ({ id, type: "Vertical", entities: [a] });
const P = (id: string, a: string, b: string): DirectionConstraint => ({
  id,
  type: "Parallel",
  entities: [a, b],
});
const X = (id: string, a: string, b: string): DirectionConstraint => ({
  id,
  type: "Perpendicular",
  entities: [a, b],
});

describe("classifyDirectionCandidate", () => {
  it("an unrelated candidate is independent", () => {
    expect(classifyDirectionCandidate([H("c1", "l1")], X("c9", "l2", "l3"))).toEqual({
      kind: "independent",
    });
  });

  it("an empty graph makes every candidate independent", () => {
    expect(classifyDirectionCandidate([], H("c1", "l1"))).toEqual({ kind: "independent" });
    expect(classifyDirectionCandidate([], P("c1", "l1", "l2"))).toEqual({ kind: "independent" });
  });

  it("a candidate already implied by the graph is redundant, with the implying chain", () => {
    // Horizontal(l1) ⇒ l1 ⊕ X = 0. A second Horizontal(l1) says the same thing.
    expect(classifyDirectionCandidate([H("c1", "l1")], H("c2", "l1"))).toEqual({
      kind: "redundant",
      witness: ["c1"],
    });
    // Horizontal(l1) + Vertical(l2) ⇒ l1 ⊕ l2 = 1, which IS Perpendicular.
    expect(
      classifyDirectionCandidate([H("c1", "l1"), V("c2", "l2")], X("c3", "l1", "l2")),
    ).toEqual({ kind: "redundant", witness: ["c1", "c2"] });
  });

  it("a candidate the graph contradicts is contradictory, and the witness closes the cycle", () => {
    // The review's A-182 shape: the two legs are axis-locked in OPPOSITE
    // directions, so Parallel between them cannot hold. Three ids — the two
    // existing constraints plus the candidate — are the unsatisfiable set.
    const decision = classifyDirectionCandidate([H("c1", "l1"), V("c2", "l2")], P("c3", "l1", "l2"));
    expect(decision).toEqual({ kind: "contradictory", witness: ["c1", "c2", "c3"] });
  });

  it("a Vertical candidate on an already-Horizontal line is contradictory", () => {
    expect(classifyDirectionCandidate([H("c1", "l1")], V("c2", "l1"))).toEqual({
      kind: "contradictory",
      witness: ["c1", "c2"],
    });
  });

  it("resolves through a 3-deep chain, not just the direct pair", () => {
    // l1 ∥ l2 ⟂ l3 ∥ l4  ⇒  l1 ⊕ l4 = 1, so Perpendicular(l1, l4) is redundant
    // and Parallel(l1, l4) contradicts.
    const chain = [P("c1", "l1", "l2"), X("c2", "l2", "l3"), P("c3", "l3", "l4")];
    expect(classifyDirectionCandidate(chain, X("c4", "l1", "l4"))).toEqual({
      kind: "redundant",
      witness: ["c1", "c2", "c3"],
    });
    expect(classifyDirectionCandidate(chain, P("c4", "l1", "l4"))).toEqual({
      kind: "contradictory",
      witness: ["c1", "c2", "c3", "c4"],
    });
  });

  it("refuses to rule on a component that already contradicts itself", () => {
    // Horizontal(l1) + Vertical(l1): the component carries a parity cycle
    // before the candidate exists, so no verdict about the candidate is honest.
    const decision = classifyDirectionCandidate([H("c1", "l1"), V("c2", "l1")], P("c3", "l1", "l2"));
    expect(decision.kind).toBe("invalid");
    if (decision.kind !== "invalid") throw new Error("unreachable");
    expect(decision.reason).toContain("c1");
    expect(decision.reason).toContain("c2");
  });

  it("an inconsistent axis chain anywhere poisons every axis-locked line's verdict", () => {
    // Every Horizontal/Vertical joins the SAME axis node, so `l9` being both at
    // once puts an odd cycle in the component `l1` also belongs to — from there
    // BOTH parities are derivable for any pair, and no redundancy claim is
    // sound. `admitDirectionCandidates` therefore authors such a candidate
    // unchanged rather than dropping it on an unsound verdict.
    const decision = classifyDirectionCandidate(
      [H("c1", "l9"), V("c2", "l9"), H("c3", "l1")],
      H("c4", "l1"),
    );
    expect(decision.kind).toBe("invalid");
  });

  it("an inconsistent component with no axis node does not reach an unrelated pair", () => {
    const decision = classifyDirectionCandidate(
      [P("c1", "l8", "l9"), X("c2", "l8", "l9"), P("c3", "l1", "l2")],
      X("c4", "l1", "l2"),
    );
    expect(decision).toEqual({ kind: "contradictory", witness: ["c3", "c4"] });
  });

  it("a pair relation naming one line twice is invalid", () => {
    const decision = classifyDirectionCandidate([], P("c1", "l1", "l1"));
    expect(decision.kind).toBe("invalid");
  });

  it("excludes the candidate from the graph even when the caller passes the full set", () => {
    const full = [H("c1", "l1"), X("c2", "l1", "l2")];
    expect(classifyDirectionCandidate(full, X("c2", "l1", "l2"))).toEqual({ kind: "independent" });
  });

  it("verdict and witness do not depend on the graph's array order", () => {
    const chain = [P("c1", "l1", "l2"), X("c2", "l2", "l3"), P("c3", "l3", "l4")];
    const permutations = [
      [chain[0], chain[1], chain[2]],
      [chain[2], chain[1], chain[0]],
      [chain[1], chain[2], chain[0]],
    ];
    for (const graph of permutations) {
      expect(classifyDirectionCandidate(graph, X("c4", "l1", "l4"))).toEqual({
        kind: "redundant",
        witness: ["c1", "c2", "c3"],
      });
    }
  });

  it("knows which kinds take part", () => {
    expect(isDirectionConstraintType("Perpendicular")).toBe(true);
    expect(isDirectionConstraintType("Coincident")).toBe(false);
  });
});

describe("admitDirectionCandidates", () => {
  it("passes non-direction candidates through untouched", () => {
    const coincident = { id: "c1", type: "Coincident", entities: ["l1", "l2"] };
    const { admitted, dropped } = admitDirectionCandidates([], [coincident]);
    expect(admitted).toEqual([coincident]);
    expect(dropped).toEqual([]);
  });

  it("drops the redundant Perpendicular a rectangle leg's snap intent proposes (A-182)", () => {
    // One leg of the review's rectangle: leg 1 is already committed Horizontal,
    // this commit infers Vertical on leg 2 from its coordinates AND persists a
    // Perpendicular from the polar snap. Precedence order puts the axis lock
    // first, so the Perpendicular is the one that goes.
    const ordered = orderDirectionCandidates(
      [X("i2", "l2", "l1"), V("a2", "l2")],
      new Set(["i2"]),
    );
    expect(ordered.map((c) => c.id)).toEqual(["a2", "i2"]);
    const { admitted, dropped } = admitDirectionCandidates([H("a1", "l1")], ordered);
    expect(admitted.map((c) => c.id)).toEqual(["a2"]);
    expect(dropped.map((d) => d.constraint.id)).toEqual(["i2"]);
    // Witness reads from the candidate's FIRST operand (l2) to its second (l1):
    // "l2 is vertical (a2), and horizontal l1 (a1) is already at right angles".
    expect(dropped[0].decision).toEqual({ kind: "redundant", witness: ["a2", "a1"] });
  });

  it("a snap intent outranks an incidental inference of the SAME rank", () => {
    // Two pair relations, nothing axis-locked: the persisted intent wins.
    const ordered = orderDirectionCandidates(
      [P("a1", "l2", "l1"), X("i1", "l2", "l1")],
      new Set(["i1"]),
    );
    expect(ordered.map((c) => c.id)).toEqual(["i1", "a1"]);
    const { admitted, dropped } = admitDirectionCandidates([], ordered);
    expect(admitted.map((c) => c.id)).toEqual(["i1"]);
    expect(dropped.map((d) => d.decision.kind)).toEqual(["contradictory"]);
  });

  it("with the axis locks committed first, every Perpendicular intent is refused", () => {
    const committed = [H("a1", "l1"), V("a2", "l2"), H("a3", "l3"), V("a4", "l4")];
    const { admitted, dropped } = admitDirectionCandidates(committed, [
      X("i2", "l2", "l1"),
      X("i3", "l3", "l2"),
      X("i4", "l4", "l3"),
    ]);
    expect(admitted).toEqual([]);
    expect(dropped.map((d) => d.decision.kind)).toEqual(["redundant", "redundant", "redundant"]);
  });

  it("a candidate accepted earlier in the batch constrains the ones after it", () => {
    const { admitted, dropped } = admitDirectionCandidates([], [H("c1", "l1"), V("c2", "l1")]);
    expect(admitted.map((c) => c.id)).toEqual(["c1"]);
    expect(dropped.map((d) => d.decision.kind)).toEqual(["contradictory"]);
  });
});
