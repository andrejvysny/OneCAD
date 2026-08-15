/*
 * Solve reconciliation (SNAP P1).
 *
 * Two things are pinned here:
 *   1. the helper itself applies BOTH channels, positions before curves;
 *   2. no PRODUCTION file outside this module composes the two halves by hand —
 *      which is the structural guard, because the defect this closes was four
 *      publication paths that each remembered only `solvedPositions`.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import { applySketchSolveResult } from "./solveResult";

const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({
  id,
  type: "Line",
  p0,
  p1,
});

describe("applySketchSolveResult", () => {
  it("returns the same reference for a null/empty result (no React churn)", () => {
    const entities = [line("l1", [0, 0], [10, 0])];
    expect(applySketchSolveResult(entities, null)).toBe(entities);
    expect(applySketchSolveResult(entities, undefined)).toBe(entities);
    expect(applySketchSolveResult(entities, {})).toBe(entities);
    expect(applySketchSolveResult(entities, { solvedPositions: {}, solvedCurves: {} })).toBe(
      entities,
    );
  });

  it("applies positions", () => {
    const entities = [line("l1", [0, 0], [10, 0])];
    const out = applySketchSolveResult(entities, { solvedPositions: { "l1.End": [12, 3] } });
    expect(out[0].p1).toEqual([12, 3]);
    expect(out).not.toBe(entities);
  });

  it("applies curve parameters even when NO point moved", () => {
    // The exact defect: a `Radius` dimension reports only `solvedCurves`, so a
    // positions-only publication rendered the old radius indefinitely.
    const entities: SketchEntity[] = [{ id: "c1", type: "Circle", center: [0, 0], radius: 5 }];
    const out = applySketchSolveResult(entities, { solvedCurves: { c1: { radius: 9 } } });
    expect(out[0].radius).toBe(9);
  });

  it("applies positions BEFORE curves, so an arc's endpoints follow the moved centre", () => {
    const entities: SketchEntity[] = [
      { id: "a1", type: "Arc", center: [0, 0], radius: 10, start: [10, 0], end: [0, 10] },
    ];
    const out = applySketchSolveResult(entities, {
      solvedPositions: { "a1.Center": [5, 5] },
      solvedCurves: { a1: { radius: 4 } },
    });
    const arc = out[0];
    expect(arc.center).toEqual([5, 5]);
    expect(arc.radius).toBe(4);
    // THE invariant: the endpoints are rebuilt off the SOLVED centre at the
    // SOLVED radius. Reverse the order and they would be re-derived around the
    // pre-solve centre and then be `radius` away from the wrong point.
    const d = (p: [number, number] | undefined): number =>
      p ? Math.hypot(p[0] - 5, p[1] - 5) : NaN;
    expect(d(arc.start)).toBeCloseTo(4, 9);
    expect(d(arc.end)).toBeCloseTo(4, 9);
  });

  it("rebuilds an arc endpoint off the SOLVED centre, not the pre-solve one", () => {
    // `applySolvedPositions` moves the centre WITHOUT translating the stored
    // endpoints, so the reshape has to see the new centre to re-derive the
    // endpoint angles. Running curves first would measure the angle against the
    // old centre and land the endpoint somewhere the arc does not pass.
    const entities: SketchEntity[] = [
      { id: "a1", type: "Arc", center: [0, 0], radius: 10, start: [10, 0], end: [0, 10] },
    ];
    const right = applySketchSolveResult(entities, {
      solvedPositions: { "a1.Center": [5, 5] },
      solvedCurves: { a1: { radius: 4 } },
    })[0];
    const wrong = applySketchSolveResult(
      applySketchSolveResult(entities, { solvedCurves: { a1: { radius: 4 } } }),
      { solvedPositions: { "a1.Center": [5, 5] } },
    )[0];
    expect(right.start).not.toEqual(wrong.start);
  });

  it("applies both channels in one call", () => {
    const entities: SketchEntity[] = [
      line("l1", [0, 0], [10, 0]),
      { id: "c1", type: "Circle", center: [0, 0], radius: 5 },
    ];
    const out = applySketchSolveResult(entities, {
      solvedPositions: { "l1.End": [20, 0] },
      solvedCurves: { c1: { radius: 7 } },
    });
    expect(out[0].p1).toEqual([20, 0]);
    expect(out[1].radius).toBe(7);
  });
});

// ── the structural guard ─────────────────────────────────────────────────────

const SRC = join(process.cwd(), "src");

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (/\.tsx?$/.test(name)) out.push(full);
  }
  return out;
}

describe("solve publication has one owner", () => {
  it("no production file outside solveResult.ts composes both solve channels", () => {
    const offenders: string[] = [];
    for (const file of walk(SRC)) {
      const rel = relative(SRC, file);
      // Tests may exercise the lower-level halves directly; `sketchWireMap`
      // DEFINES them; `solveResult` is the sanctioned composition.
      if (/\.test\.tsx?$/.test(rel)) continue;
      if (rel === join("ipc", "sketchWireMap.ts")) continue;
      if (rel === join("tools", "sketch", "solveResult.ts")) continue;
      const text = readFileSync(file, "utf8");
      // Only IMPORTS count — a doc comment naming the function is fine.
      const imports = text.match(/^import[\s\S]*?from\s+["'][^"']+["'];/gm) ?? [];
      const joined = imports.join("\n");
      if (joined.includes("applySolvedPositions") && joined.includes("applySolvedCurves")) {
        offenders.push(rel);
      }
    }
    expect(offenders).toEqual([]);
  });

  it("every SketchController / sketchService publication routes through the helper", () => {
    for (const rel of [
      join("tools", "sketch", "SketchController.ts"),
      join("tools", "sketch", "sketchService.ts"),
    ]) {
      const text = readFileSync(join(SRC, rel), "utf8");
      const calls = text.match(/apply(SolvedPositions|SolvedCurves)\(/g) ?? [];
      expect(calls, rel).toEqual([]);
      expect(text).toContain("applySketchSolveResult");
    }
  });
});
