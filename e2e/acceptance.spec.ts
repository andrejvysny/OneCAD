import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  constraintToolbar,
  getSketchSnapshot,
  planePointToClient,
  commitExtrudeAtHandle,
  type SketchLineSnapshot,
  type SketchPlaneSnapshot,
} from "./helpers";

const mid = (l: SketchLineSnapshot): { x: number; y: number } => ({
  x: (l.p0[0] + l.p1[0]) / 2,
  y: (l.p0[1] + l.p1[1]) / 2,
});

/*
 * Full acceptance ribbon in one deterministic flow (mock lane):
 *   AC1 — enter sketch via the real plane picker.
 *   (b) — draw a closed rectangle; select tool click-selects a line (constraint
 *         toolbar feedback) and a vertex (apply Fixed → new ConstraintList row).
 *   (c) — dimension: two points → Distance → the seeded chip → typed value →
 *         a new dimensional row carrying that value.
 *   (d) — delete round-trip: select the rectangle's entities → Delete → DOF +
 *         constraint rows change.
 *   (e) — redraw the closed region → Enter → AC3: extrude auto-arms, and a
 *         real drag-commit on the depth handle produces a Body.
 *
 * Targeting note: every select-tool click below targets a point read back from
 * the LIVE sketch entities (`getSketchSnapshot`) and projected through the real
 * camera (`planePointToClient`), not a guessed canvas-relative pixel offset —
 * `screenToPlane` is a real perspective raycast, so pixel offsets from the two
 * draw clicks cannot be linearly combined to predict a third point's screen
 * position (a corner never clicked, an edge midpoint, …). See helpers.ts.
 *
 * Constraint-button caveat (verified against autoConstrain.ts + rect.spec.ts):
 * drawing an axis-aligned rectangle already auto-infers Horizontal/Vertical on
 * EVERY side. Re-applying Horizontal/Vertical on a side is therefore redundant,
 * and `applyConstraint`'s reject-on-conflict path (sketchService.ts) would drop
 * it silently (no new row) if the mock's DOF bookkeeping calls it
 * over-constraining. So step (b) proves the "selection → toolbar enables"
 * feedback via a LINE (Horizontal/Vertical light up), but proves the actual
 * "apply → new row" round trip via a VERTEX + Fixed instead — single-target,
 * never auto-applied, so it is guaranteed novel (confirmed against
 * `evaluateApplicability`/`constraintApplicability.ts`).
 *
 * Extrude commit caveat: ModelToolController only calls `commitExtrude` from a
 * real drag-release on the 3D depth handle (see helpers.ts `commitExtrudeAtHandle`
 * doc) — typing a value into the chip only updates the live preview depth.
 */
test("full acceptance ribbon: plane picker → constrain → dimension → delete → extrude", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page); // AC1
  // Entering the plane picker kicks off an animated camera re-home
  // (CadOrbitControls, 250ms) — settle before any click/projection races it
  // (see helpers.ts waitForCameraSettled for the empirically-confirmed root cause).
  await waitForCameraSettled(page);

  // ── (b) draw a closed rectangle, then read its real geometry back ───────
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4);
  const plane: SketchPlaneSnapshot = snap.plane;
  const clickPlanePoint = async (p: { x: number; y: number }) => {
    const c = await planePointToClient(page, plane, p);
    await clickAtClient(page, c.x, c.y);
  };

  // Select tool: click one line's MIDPOINT → selects the LINE body (hitTestSketch
  // prioritizes vertex handles within tolerance, so a midpoint always hits the body).
  await selectSketchTool(page, "Select");
  const toolbar = constraintToolbar(page);
  await clickPlanePoint(mid(snap.lines[0]));
  const horizontalBtn = toolbar.getByRole("button", { name: "Horizontal", exact: true });
  await expect(horizontalBtn).toBeEnabled(); // selection feedback: constraint toolbar lit up

  const rows = () => page.locator('[data-testid^="constraint-row-"]');
  const rowsBeforeFixed = await rows().count();

  // Click that same line's Start vertex → selects the point handle → Fixed applies.
  await clickPlanePoint({ x: snap.lines[0].p0[0], y: snap.lines[0].p0[1] });
  const fixedBtn = toolbar.getByRole("button", { name: "Fixed", exact: true });
  await expect(fixedBtn).toBeEnabled();
  await fixedBtn.click();
  await expect(rows()).toHaveCount(rowsBeforeFixed + 1);
  await expect(rows().last()).toContainText("Fixed");

  // ── (c) dimension: two points on the SAME line (guaranteed adjacent, joined)
  //        → Distance → chip → typed value → new row ──────────────────────
  await clickPlanePoint({ x: snap.lines[0].p0[0], y: snap.lines[0].p0[1] }); // Start (plain replaces selection)
  await page.keyboard.down("Shift");
  await clickPlanePoint({ x: snap.lines[0].p1[0], y: snap.lines[0].p1[1] }); // + End
  await page.keyboard.up("Shift");

  const distanceBtn = toolbar.getByRole("button", { name: "Distance", exact: true });
  await expect(distanceBtn).toBeEnabled();
  const rowsBeforeDistance = await rows().count();
  await distanceBtn.click();

  const dimInput = page.getByLabel("Dimension value");
  await expect(dimInput).toBeVisible();
  // The chip seeds the CURRENT measured distance (buildAppliedDimension); the
  // plane-space size of our rectangle varies with the camera's zoom/framing
  // (each run's screen→plane raycast differs), so a hardcoded literal risks
  // coincidentally matching the seed — DimensionInput.commit() no-ops when the
  // typed value equals the current one (`n !== value` gate), silently skipping
  // the author call. Derive a value guaranteed to differ instead.
  const seeded = Number.parseFloat(await dimInput.inputValue());
  const typedValue = seeded + 50;
  await dimInput.fill(typedValue.toFixed(1));
  await dimInput.press("Enter");

  await expect(rows()).toHaveCount(rowsBeforeDistance + 1);
  const distanceRow = rows().last();
  await expect(distanceRow).toContainText("Distance");
  await expect(distanceRow).toContainText(typedValue.toFixed(1));

  // ── (d) delete round-trip ────────────────────────────────────────────────
  const dofBeforeDelete = (await dofPill(page).textContent()) ?? "";
  // Select all four sides (shift-click each line's midpoint) and delete in one shot.
  await clickPlanePoint(mid(snap.lines[0])); // plain → replaces selection
  await page.keyboard.down("Shift");
  await clickPlanePoint(mid(snap.lines[1]));
  await clickPlanePoint(mid(snap.lines[2]));
  await clickPlanePoint(mid(snap.lines[3]));
  await page.keyboard.up("Shift");
  await page.keyboard.press("Delete");

  await expect(dofPill(page)).not.toHaveText(dofBeforeDelete);
  await expect(dofPill(page)).toHaveText("DOF: 0"); // sketch is empty again
  await expect(page.getByText("No constraints yet.")).toBeVisible();

  // ── (e) redraw the closed region, finish → AC3 auto-arms extrude ────────
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  const bodiesBefore = await bodyOptions(page).count();
  await page.keyboard.press("Enter"); // finish → single region → auto-arms extrude directly
  await expect(page.getByText("Drag the arrow to set depth, or type a value")).toBeVisible();

  await commitExtrudeAtHandle(page);
  // "Extruded" is a transient statusHint (ModelToolController.finishExtrude) that
  // can already be gone by the time we poll — the durable proof of a successful
  // commit is the new Body row, which finishExtrude also selects.
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect(bodyOptions(page).last()).toHaveAttribute("aria-selected", "true");
});
