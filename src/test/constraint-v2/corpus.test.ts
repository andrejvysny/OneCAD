/*
 * Constraint V2 corpus runner (SKETCH-V2 P0).
 *
 * Asserts every expectation whose owning phase is in `LANDED_PHASES`, and
 * REPORTS the rest. The point is that nothing in `fixtures.ts` can quietly go
 * unverified: `pending expectations are accounted for` fails if a case carries
 * an expectation field this runner does not know how to route.
 *
 * Adding a phase to `LANDED_PHASES` without wiring its assertion block is a
 * hard error (see `every landed phase has a wired assertion block`), so the
 * gate cannot be passed by editing one constant.
 */
import { describe, it, expect } from "vitest";
import { solveSketch } from "@/ipc/mockSketch";
import { CONSTRAINT_V2_CORPUS, type ConstraintCase, type Phase } from "./fixtures";

/** Phases whose implementation has landed. Extend ONLY together with the
 *  matching assertion block below and a recorded gate in `TODO.md`. */
const LANDED_PHASES: ReadonlySet<Phase> = new Set<Phase>(["P0"]);

/** Which phase owns each optional expectation field on a case. */
const FIELD_PHASE = {
  targetDof: "P2",
  vertices: "P4",
  connectedComponent: "P4",
  entityStates: "P10",
  drags: "P7",
  edits: "P2",
} as const satisfies Record<string, Phase>;

/** Phases this file actually knows how to assert. */
const WIRED_PHASES: ReadonlySet<Phase> = new Set<Phase>(["P0"]);

const landed = (p: Phase): boolean => LANDED_PHASES.has(p);

/**
 * Placeholder body for a phase-gated block that has no real assertion yet.
 * These blocks are skipped while their phase is unlanded, so this throws only
 * once someone claims the phase — which is exactly when the assertion must be
 * written. Never replace this with a vacuous `toBeDefined()`: a gated test that
 * passes without checking anything is worse than no test.
 */
function notWiredYet(phase: Phase, what: string): never {
  throw new Error(
    `${phase} is in LANDED_PHASES but "${what}" still has a placeholder body. ` +
      `Write the real assertion against the fixture before passing the ${phase} gate.`,
  );
}

describe("constraint-v2 corpus — integrity", () => {
  it("every landed phase has a wired assertion block", () => {
    const unwired = [...LANDED_PHASES].filter((p) => !WIRED_PHASES.has(p));
    expect(unwired, `LANDED_PHASES claims ${unwired.join(", ")} with no assertions`).toEqual([]);
  });

  it("pending expectations are accounted for", () => {
    // Guards against a field being added to a fixture with no route here.
    const known = new Set([
      "name",
      "why",
      "entities",
      "constraints",
      "currentMockDof",
      ...Object.keys(FIELD_PHASE),
    ]);
    const stray: string[] = [];
    for (const kase of CONSTRAINT_V2_CORPUS) {
      for (const k of Object.keys(kase)) if (!known.has(k)) stray.push(`${kase.name}.${k}`);
    }
    expect(stray).toEqual([]);
  });

  it("case names are unique", () => {
    const names = CONSTRAINT_V2_CORPUS.map((k) => k.name);
    expect(new Set(names).size).toBe(names.length);
  });
});

describe.each(CONSTRAINT_V2_CORPUS.map((k): [string, ConstraintCase] => [k.name, k]))(
  "constraint-v2 · %s",
  (_name, kase) => {
    it("is well formed", () => {
      const entityIds = kase.entities.map((e) => e.id);
      expect(new Set(entityIds).size, "duplicate entity id").toBe(entityIds.length);

      const constraintIds = kase.constraints.map((c) => c.id);
      expect(new Set(constraintIds).size, "duplicate constraint id").toBe(constraintIds.length);

      const live = new Set(entityIds);
      for (const c of kase.constraints) {
        for (const ref of c.entities) {
          expect(live.has(ref), `${c.id} references missing entity ${ref}`).toBe(true);
        }
        if (c.positions) {
          expect(c.positions.length, `${c.id} positions/entities arity`).toBe(c.entities.length);
        }
      }
    });

    it("every vertex expectation names live entities", () => {
      if (!kase.vertices) return;
      const live = new Set(kase.entities.map((e) => e.id));
      for (const [vid, v] of Object.entries(kase.vertices)) {
        expect(v.members.length, `${vid} has no members`).toBeGreaterThan(0);
        for (const m of v.members) {
          expect(live.has(m.split(".")[0]), `${vid} member ${m} names a missing entity`).toBe(true);
        }
      }
    });

    // ── P0: the only lane computable today ───────────────────────────────────
    it("mock DOF matches the recorded current value", () => {
      const { dof } = solveSketch(kase.entities, kase.constraints);
      expect(dof).toBe(kase.currentMockDof);
    });

    // ── P2 ───────────────────────────────────────────────────────────────────
    it.skipIf(!landed(FIELD_PHASE.targetDof))("global DOF matches the target", () => {
      const { dof } = solveSketch(kase.entities, kase.constraints);
      expect(dof).toBe(kase.targetDof);
    });

    it.skipIf(!landed(FIELD_PHASE.edits))("atomic edits accept/reject as specified", () => {
      notWiredYet("P2", "atomic edits");
    });

    // ── P4 ───────────────────────────────────────────────────────────────────
    it.skipIf(!landed(FIELD_PHASE.vertices))("shared vertices match", () => {
      notWiredYet("P4", "shared vertices");
    });

    it.skipIf(!landed(FIELD_PHASE.connectedComponent))("connected component matches", () => {
      notWiredYet("P4", "connected component");
    });

    // ── P7 ───────────────────────────────────────────────────────────────────
    it.skipIf(!landed(FIELD_PHASE.drags))("drag outcomes match", () => {
      notWiredYet("P7", "drag outcomes");
    });

    // ── P10 ──────────────────────────────────────────────────────────────────
    it.skipIf(!landed(FIELD_PHASE.entityStates))("per-entity states match", () => {
      notWiredYet("P10", "per-entity states");
    });
  },
);

/**
 * Cases where today's mock DOF is knowingly NOT the spec target, as
 * `"<case>: mock <n> vs target <n>"`. Empty today — the naive mock count
 * happens to agree with the spec on all 13 cases, which is why DOF is the one
 * lane P0 can already assert. A new entry here is a deliberate, recorded gap;
 * an UNLISTED divergence is a regression.
 */
const KNOWN_DOF_GAPS: readonly string[] = [];

describe("constraint-v2 corpus — the gap", () => {
  it("mock/target DOF divergences are exactly the recorded ones", () => {
    const diverging = CONSTRAINT_V2_CORPUS.filter((k) => k.currentMockDof !== k.targetDof).map(
      (k) => `${k.name}: mock ${k.currentMockDof} vs target ${k.targetDof}`,
    );
    expect(diverging.slice().sort()).toEqual(KNOWN_DOF_GAPS.slice().sort());
  });
});
