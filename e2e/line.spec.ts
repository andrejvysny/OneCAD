import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  clickAt,
  clickAtAwaitingDofChange,
  dofPill,
} from "./helpers";

/*
 * Line tool — the REAL user path (plane picker) end to end.
 *
 * Draws a 3-segment polyline: the first click drops the anchor, each subsequent
 * click commits a segment. Every committed segment round-trips through
 * sketchUpsert → the mock solver, and autoConstrain infers a Coincident at every
 * shared endpoint (the chain reuses the exact snapped point). Between committing
 * clicks we settle on the DOF pill so the async commits can't clobber each other.
 * Assertions go through store-driven chrome: DOF pill + inspector Constraints.
 */
test("line tool draws a 3-segment chain with autoconstraint feedback", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);

  // Fresh empty sketch: fully constrained, DOF 0.
  await expect(dofPill(page)).toHaveText("DOF: 0");

  // Anchor + 3 chained segments (canvas-center-relative offsets, well spaced and
  // clear of the floating panels). Esc ends the chain.
  await clickAt(page, -160, -80);
  await clickAtAwaitingDofChange(page, -40, -80);
  await clickAtAwaitingDofChange(page, 60, 10);
  await clickAtAwaitingDofChange(page, 160, 90);
  await page.keyboard.press("Escape");

  // Entities exist → DOF is now non-zero (3 lines, minus autoconstraints).
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Chrome + inspector reflect the entities: under-constrained + Coincident rows
  // from autoconstrain (chained endpoints snapped exactly onto each other).
  // (chrome bar + inspector card both render the status; either is a valid signal)
  await expect(page.getByText(/^Under-constrained · DOF [1-9]/).first()).toBeVisible();
  // Per-row ConstraintList: a 3-segment chain infers one Coincident per shared
  // endpoint, so assert at least one row rather than exact-single text.
  await expect(page.getByText("Coincident", { exact: true }).first()).toBeVisible();
});
