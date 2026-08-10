import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, getFeatureLabels, bodyOptions } from "./helpers";
import {
  seedSelection,
  toolPhases,
  dragEdgeOpHandle,
  openFilletOverflow,
  closeFilletOverflow,
} from "./modelToolHelpers";

/*
 * FILLET-CHAMFER-UNIFY W2 — the `chamfer` TOOL ID and its `H` binding are dead;
 * one "Fillet / Chamfer" tool remains (id/icon/shortcut stay `fillet`/`fillet`/
 * `F`), and the drag DIRECTION — or an explicit chip segment — decides which op
 * gets authored. `H` now falls through to the GLOBAL `home` binding.
 *
 * Folds the two specs this wave deletes:
 *   - `chamfer.spec.ts` (the armed-commit gesture: released-stays-armed, ✓
 *     commits, Enter/✕) — armed here via the chip segment rather than a
 *     dedicated tool, since that tool id no longer exists.
 *   - `fillet-direction.spec.ts` (the bisector-tier direction drive + segment
 *     lock) — ported unchanged; the direction math itself did not move in W2.
 * Plus the W2 sweep cases: no separate Chamfer button, and `h` is inert as an
 * edge-op trigger.
 *
 * The seeded mock document publishes the box body's MESH1 through the ingest
 * path, so `body1`'s edges resolve on the bisector tier here — which is what
 * lets the direction-driven cases below drive the real gesture rather than
 * only the chip segments.
 */

/** `e:5` of the mock box (80×60×30, origin-centred): the +x/+z top edge. Its
 *  midpoint and bisector are fixed by the mesh, so both are written out here. */
const EDGE_MID: [number, number, number] = [40, 0, 15];
const OUTWARD: [number, number, number] = [Math.SQRT1_2, 0, Math.SQRT1_2];

const EDGE_REF = {
  kind: "edge",
  id: "body1#e:5",
  bodyId: "body1",
  topoKey: "e:5",
  elementId: "el-edge-5",
  // Anchors the chip (and the real 3D handle it's now leader-lined off of, since
  // this edge resolves the bisector tier — UNIFY-UX Phase 1) ON the edge, not at
  // the world origin, which real MESH1 tier math needs to be honest anyway.
  anchor: { worldPoint: EDGE_MID },
};

async function armFillet(page: Page): Promise<void> {
  await openEditorDebug(page);
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
}

/** Arm Fillet, then lock Chamfer via the chip segment — the unified-tool
 *  replacement for the old dedicated `chamfer` tool id / arm helper. */
async function armChamferViaSegment(page: Page): Promise<void> {
  await armFillet(page);
  await openFilletOverflow(page);
  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpKind).toBe("Chamfer");
  await closeFilletOverflow(page); // the popover can float over the handle
}

/** The screen direction one world step of "away from the body" points, via the
 *  engine's own `projectPoint` (exposed by `?vpdebug`). */
async function awayOnScreen(page: Page): Promise<{ x: number; y: number }> {
  const d = await page.evaluate(
    ([mid, out]) => {
      const engine = (
        window as unknown as {
          __vpEngine?: { projectPoint(w: number[]): { x: number; y: number } | null };
        }
      ).__vpEngine;
      if (!engine) throw new Error("__vpEngine unavailable (?vpdebug only)");
      const a = engine.projectPoint(mid);
      const b = engine.projectPoint([mid[0] + 10 * out[0], mid[1] + 10 * out[1], mid[2] + 10 * out[2]]);
      if (!a || !b) throw new Error("edge midpoint does not project");
      return { x: b.x - a.x, y: b.y - a.y };
    },
    [EDGE_MID, OUTWARD] as const,
  );
  const len = Math.hypot(d.x, d.y);
  expect(len).toBeGreaterThan(1); // a head-on edge would make this test meaningless
  return { x: d.x / len, y: d.y / len };
}

/** The armed chip's current value (the only readout of the dragged size). */
async function chipValue(page: Page): Promise<number> {
  return Number.parseFloat(await page.getByLabel("Dimension value").inputValue());
}

/** The projection's last feature row (`__stores.document`, dev-only). */
async function lastFeature(
  page: Page,
): Promise<{ id: string; label: string; valueText: string }> {
  const f = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        document: {
          getState(): { features: Array<{ id: string; label: string; valueText: string }> };
        };
      };
    };
    const all = w.__stores?.document.getState().features ?? [];
    return all[all.length - 1];
  });
  if (!f) throw new Error("the projection has no features");
  return f;
}

/** The armed chamfer's second-distance field (`=` when equal-leg). */
function d2Field(page: Page) {
  return page.getByLabel("Second distance");
}

/** Open the timeline and double-click the row `id` (the parametric re-edit entry).
 *  A re-edit has no picks of its own — `filletOutward` never resolves, so it stays
 *  DEGRADED (no 3D handle) — but its [Fillet|Chamfer] segments still live behind
 *  the overflow, so every caller needs it open regardless. */
async function reopenRow(page: Page, id: string): Promise<void> {
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  const row = page.getByTestId(`history-row-${id}`);
  await expect(row).toBeVisible();
  await row.dblclick();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await openFilletOverflow(page);
}

// ── the unified tool + the dead `chamfer` id / `H` binding ───────────────────

test("the toolbar shows ONE Fillet / Chamfer button, no separate Chamfer button", async ({ page }) => {
  await openEditorDebug(page);
  await expect(page.getByRole("button", { name: "Fillet / Chamfer", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Chamfer", exact: true })).toHaveCount(0);
});

test("`h` in model mode does not arm an edge op (H is Home now, not Chamfer)", async ({ page }) => {
  await openEditorDebug(page);
  // Touch a real tool switch once first so the debug surface actually exists —
  // `window.__extrudePreview` is first published by `onToolChange`'s own cancel
  // sweep (`cancelFillet` calls `updateDebug` unconditionally), and a fresh boot
  // has never fired one, so polling for "idle" straight off `h` would otherwise
  // hang against `undefined` forever rather than genuinely observe idle.
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("idle");

  await page.keyboard.press("h");
  // No edge op armed — the minimum bar for a key that used to be a tool trigger
  // and is now a plain (engine-side, storeless) camera action.
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("idle");
  await expect(page.getByRole("button", { name: "Fillet / Chamfer", exact: true })).not.toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("F arms the unified tool with an edge seeded; an empty selection hints for Fillet", async ({ page }) => {
  await openEditorDebug(page);
  // No edge selected → the tool arms nothing and names the CURRENT (default) type,
  // exactly as before unification (armEdgeOpFromSelection's empty-selection guard).
  await page.keyboard.press("f");
  await expect(page.getByText("Select edges, then Fillet")).toBeVisible();

  await page.keyboard.press("Escape");
  await seedSelection(page, [EDGE_REF]);
  await page.keyboard.press("f");
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await expect(page.getByRole("button", { name: "Fillet / Chamfer", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

// ── bisector-tier direction drive (ported from fillet-direction.spec.ts) ─────

test("a bisector-tier arm is direction-driven: drag INTO the body types a Chamfer, back OUT types a Fillet", async ({
  page,
}) => {
  await armFillet(page);

  const armed = await toolPhases(page);
  expect(armed?.edgeOpAxisSource).toBe("bisector"); // the only tier auto may trust
  expect(armed?.edgeOpAuto).toBe(true);
  expect(armed?.edgeOpKind).toBe("Fillet");

  const away = await awayOnScreen(page);
  // UNIFY-UX Phase 1: the bisector tier resolved, so a real handle exists and the
  // drag has to start ON it — the delta math is press-relative, so the direction
  // vector below still drives the same type flip regardless of where the press
  // itself lands.
  await dragEdgeOpHandle(page, -away.x * 140, -away.y * 140); // INTO the body

  await expect.poll(async () => (await toolPhases(page))?.edgeOpKind).toBe("Chamfer");
  await openFilletOverflow(page);
  await expect(page.getByTestId("chip-edgeop-chamfer")).toHaveAttribute("aria-pressed", "true");
  expect(await chipValue(page)).toBeGreaterThan(1); // a MAGNITUDE, never negative
  // A flip is not a commit — the tool is still armed on the same edges.
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await closeFilletOverflow(page); // the popover can float over the handle

  // …and the gesture is reversible: dragging back out re-types to Fillet.
  await dragEdgeOpHandle(page, away.x * 200, away.y * 200);
  await expect.poll(async () => (await toolPhases(page))?.edgeOpKind).toBe("Fillet");
});

test("an explicit segment pick LOCKS the type against any later drag", async ({ page }) => {
  await armFillet(page);
  expect((await toolPhases(page))?.edgeOpAuto).toBe(true);

  // Picking the ACTIVE segment changes no op — it ends the direction-driven lane.
  await openFilletOverflow(page);
  await page.getByTestId("chip-edgeop-fillet").click();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpAuto).toBe(false);
  expect((await toolPhases(page))?.edgeOpKind).toBe("Fillet");
  await closeFilletOverflow(page); // the popover can float over the handle

  const away = await awayOnScreen(page);
  await dragEdgeOpHandle(page, -away.x * 200, -away.y * 200); // hard INTO the body
  expect((await toolPhases(page))?.edgeOpKind).toBe("Fillet"); // no re-type
  // The locked type projects the drag onto its own half-line: bottomed at the
  // minimum rather than bouncing back up as a magnitude would.
  expect(await chipValue(page)).toBeCloseTo(0.1, 5);

  await dragEdgeOpHandle(page, away.x * 200, away.y * 200);
  expect(await chipValue(page)).toBeGreaterThan(1); // …and it recovers, still a Fillet
  expect((await toolPhases(page))?.edgeOpKind).toBe("Fillet");
});

// ── armed-commit gesture (ported from chamfer.spec.ts, armed via segment) ───

test("a released drag stays ARMED and commits nothing", async ({ page }) => {
  await armChamferViaSegment(page);
  const before = await getFeatureLabels(page);

  await dragEdgeOpHandle(page, 0, -40);

  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
  await expect(page.getByRole("button", { name: "Fillet / Chamfer", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(await getFeatureLabels(page)).toEqual(before); // release NEVER commits
});

test("the visible ✓ commits Chamfer after a flip", async ({ page }) => {
  await armChamferViaSegment(page);
  const before = await getFeatureLabels(page);

  await dragEdgeOpHandle(page, 0, -40);
  await page.getByTestId("chip-confirm").click();

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Chamfer");
  // A committed tool hands the pointer back to Select and drops its chip.
  await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByTestId("chip-confirm")).toHaveCount(0);
});

test("the visible ✓ commits Fillet with no flip", async ({ page }) => {
  await armFillet(page);
  const before = await getFeatureLabels(page);

  await page.getByTestId("chip-confirm").click();

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Fillet");
});

test("Enter commits the armed op; ✕ cancels it with no row", async ({ page }) => {
  await armChamferViaSegment(page);
  const before = await getFeatureLabels(page);

  await page.getByTestId("chip-cancel").click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("idle");
  expect(await getFeatureLabels(page)).toEqual(before); // ✕ writes nothing

  // Re-arm (plain Fillet this time) and finish with the keyboard instead.
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await page.keyboard.press("Enter");

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Fillet");
});

// ── re-edit TYPE flip (W3) ──────────────────────────────────────────────────

test("a committed Fillet row re-edits its TYPE: dblclick → Chamfer segment → Enter swaps the SAME row, ⌘Z restores", async ({
  page,
}) => {
  // (1) Commit a real Fillet — the seeded mock rows carry no stored params, so a
  // re-edit has to run against a feature this spec authored.
  await armFillet(page);
  await page.getByTestId("chip-confirm").click();
  await expect.poll(async () => (await getFeatureLabels(page)).at(-1)).toBe("Fillet");
  const rowCount = (await getFeatureLabels(page)).length;
  const { id } = await lastFeature(page);

  // (2) Reach the FULL timeline: the body state shows only the first rows, the
  // feature state shows every one (InspectorPanel BodyState vs FeatureState).
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  const row = page.getByTestId(`history-row-${id}`);
  await expect(row).toBeVisible();

  // (3) Double-click re-opens it armed on its COMMITTED type, with the segments.
  await row.dblclick();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await openFilletOverflow(page);
  await expect(page.getByTestId("chip-edgeop-fillet")).toHaveAttribute("aria-pressed", "true");
  // The re-edit opens the committed type with the drag lane OFF — only an explicit
  // segment may re-type a committed op.
  expect((await toolPhases(page))?.edgeOpAuto).toBe(false);

  // (4) Flip + Enter. The size is untouched, so this commits a PURE opType swap.
  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpKind).toBe("Chamfer");
  await page.getByLabel("Dimension value").click();
  await page.keyboard.press("Enter");

  // (5) The SAME row swapped — no new feature was authored.
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Chamfer");
  expect((await getFeatureLabels(page)).length).toBe(rowCount);
  expect((await lastFeature(page)).id).toBe(id);

  // (6) …and the swap is a normal undoable edit.
  await page.keyboard.press("Meta+z");
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Fillet");
  expect((await getFeatureLabels(page)).length).toBe(rowCount);
});

// ── two-distance chamfer (SCHEMA §7.3, 2026-08-03 — WP-C T2a) ────────────────

test("a Chamfer arm gains a second-distance field; a Fillet arm does not", async ({ page }) => {
  await armFillet(page);
  await openFilletOverflow(page);
  // SCHEMA §7.3 forbids a Fillet from carrying `distance2`, so the field that
  // authors it must not even be reachable there.
  await expect(d2Field(page)).toHaveCount(0);

  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect(d2Field(page)).toHaveValue("="); // equal-leg, not a blank
  expect((await toolPhases(page))?.edgeOpDistance2).toBeNull();

  // …and flipping back hides it again.
  await page.getByTestId("chip-edgeop-fillet").click();
  await expect(d2Field(page)).toHaveCount(0);
});

test("arm Chamfer → type a second distance → commit: the row reads `d1×d2`", async ({ page }) => {
  await armChamferViaSegment(page);
  await openFilletOverflow(page);
  const before = await getFeatureLabels(page);

  await d2Field(page).fill("2.5");
  // Enter in the second-distance field applies the value THEN confirms the op —
  // the same single-fire contract the first distance's input has.
  await d2Field(page).press("Enter");

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  const row = await lastFeature(page);
  expect(row.label).toBe("Chamfer");
  // Mirrors Rust `dto.rs feature_value_text` (pinned there); the LEADING number
  // is what a re-edit seeds d1 from.
  expect(row.valueText).toBe("1.0×2.5 mm");
});

test("a two-distance chamfer BLOCKS the type flip, and allows it once d2 is cleared", async ({
  page,
}) => {
  // (1) Commit a two-distance chamfer this spec owns (the seeded mock rows carry
  // no stored params, so a re-edit has to run against one authored here).
  await armChamferViaSegment(page);
  await openFilletOverflow(page);
  await d2Field(page).fill("2.5");
  await d2Field(page).press("Enter");
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Chamfer");
  const { id } = await lastFeature(page);
  const rowCount = (await getFeatureLabels(page)).length;

  // (2) Re-open it: BOTH legs are seeded from the stored params.
  await reopenRow(page, id);
  await expect(page.getByTestId("chip-edgeop-chamfer")).toHaveAttribute("aria-pressed", "true");
  await expect(d2Field(page)).toHaveValue("2.5");
  expect((await toolPhases(page))?.edgeOpDistance2).toBe(2.5);

  // (3) Flipping to Fillet would DROP the second leg, so the backend refuses the
  // edit with the standard allow-list reason — and the hint names the field.
  await page.getByTestId("chip-edgeop-fillet").click();
  await page.getByLabel("Dimension value").click();
  await page.keyboard.press("Enter");
  await expect(page.getByText(/clear distance2 first/)).toBeVisible();
  // The rejected flip wrote nothing: the row is still the same Chamfer.
  expect((await lastFeature(page)).label).toBe("Chamfer");
  expect((await lastFeature(page)).valueText).toBe("1.0×2.5 mm");
  expect((await getFeatureLabels(page)).length).toBe(rowCount);

  // (4) Clear the second leg back to `=` — an ordinary params edit.
  await reopenRow(page, id);
  await d2Field(page).fill("=");
  await d2Field(page).press("Enter");
  await expect.poll(async () => (await lastFeature(page)).valueText).toBe("1.0 mm");
  expect((await lastFeature(page)).label).toBe("Chamfer");

  // (5) …and NOW the plain W3 swap goes through on the SAME row.
  await reopenRow(page, id);
  await expect(d2Field(page)).toHaveValue("=");
  await page.getByTestId("chip-edgeop-fillet").click();
  await page.getByLabel("Dimension value").click();
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Fillet");
  expect((await lastFeature(page)).id).toBe(id);
  expect((await getFeatureLabels(page)).length).toBe(rowCount);
});
