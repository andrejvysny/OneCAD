import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  dofPill,
  constraintToolbar,
  getSketchSnapshot,
  planePointToClient,
} from "./helpers";

/*
 * Dimension reject-on-conflict e2e (mock lane) — proves the mock's deterministic
 * conflict detection (mockConflicts.ts R1: duplicate-incompatible dimension) makes
 * the Conflicting UX reachable outside the C++ worker: authoring a SECOND Distance
 * on the same two points with a different value is dropped, the sketch reverts,
 * and the status hint NAMES the pre-existing Distance constraint.
 *
 * Value note: the chip seeds the CURRENT measured distance between the clicked
 * points (`buildAppliedDimension`), which varies with the camera's zoom/framing
 * (a real perspective raycast — same caveat as acceptance.spec.ts). Typed values
 * are derived as `seeded + N` rather than hardcoded so a coincidental match never
 * makes `DimensionInput.commit()`'s `n !== value` no-op gate silently skip the
 * author call.
 */
test("a second Distance on the same points with a different value is rejected + reverted", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // A single non-axis-aligned line (~31° — safe from the 5° H/V autoConstrain
  // tolerance), so DOF arithmetic stays exact: 4 free dof, nothing auto-removed.
  await selectSketchTool(page, "Line");
  await clickAt(page, -180, -140);
  await clickAtAwaitingDofChange(page, -80, -80);
  await page.keyboard.press("Escape");
  await expect(dofPill(page)).toHaveText("DOF: 4");

  await selectSketchTool(page, "Select");
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(1);
  const plane = snap.plane;
  let line = snap.lines[0];

  const clickPlanePoint = async (p: { x: number; y: number }) => {
    const c = await planePointToClient(page, plane, p);
    await clickAtClient(page, c.x, c.y);
  };
  // A 2-point selection stays live after a commit (nothing clears it), which
  // leaves `ConstraintContextChips`'s floating button row parked at the pair's
  // centroid — close enough to the line's own endpoints on screen to intercept
  // the NEXT round's plain click before it ever reaches the canvas. A miss-click
  // on empty canvas (`clickSelection`'s "plain + miss → clear") dismisses it and
  // gives each round a clean slate, exactly like a user clicking away would.
  const deselect = () => clickAt(page, 250, 220);
  const selectBothEndpoints = async () => {
    await clickPlanePoint({ x: line.p0[0], y: line.p0[1] }); // Start (plain replaces selection)
    await page.keyboard.down("Shift");
    await clickPlanePoint({ x: line.p1[0], y: line.p1[1] }); // + End
    await page.keyboard.up("Shift");
  };

  // A plain selector chain, not a snapshot — the menu is a Popover that
  // closes on any outside click (Sketcher UX cleanup, Track A3), so
  // `constraintToolbar(page)` must be called again to re-open it right
  // before EACH round below, not cached across the intervening
  // deselect/reselect canvas clicks.
  const distanceBtn = page
    .getByRole("toolbar", { name: "Constraints" })
    .getByRole("button", { name: "Distance", exact: true });
  const rows = () => page.locator('[data-testid^="constraint-row-"]');
  // Scoped to the toolChipStore-driven chip host (ModelToolChips.tsx — the
  // shared renderer for EVERY `toolChipStore` chip, model-tool AND sketch
  // dimension alike). An unscoped `getByLabel` collides with the Inspector's
  // OWN per-constraint editable value field once a Distance row exists.
  const dimInput = page.getByTestId("model-tool-chip").getByLabel("Dimension value");

  // ── First Distance: accepted, a new row lands, DOF drops by 1 ────────────────
  await selectBothEndpoints();
  await constraintToolbar(page);
  await expect(distanceBtn).toBeEnabled();
  const rowsBefore = await rows().count();
  await distanceBtn.click();

  await expect(dimInput).toBeVisible();
  const seeded = Number.parseFloat(await dimInput.inputValue());
  const firstValue = seeded + 50;
  await dimInput.fill(firstValue.toFixed(1));
  await dimInput.press("Enter");

  await expect(rows()).toHaveCount(rowsBefore + 1);
  const distanceRow = rows().last();
  await expect(distanceRow).toContainText("Distance");
  await expect(dofPill(page)).toHaveText("DOF: 3");
  const rowsAfterFirst = await rows().count();
  const dofAfterFirst = (await dofPill(page).textContent()) ?? "";

  // ── Second Distance on the SAME two points, a different value: rejected ─────
  await deselect();
  // Re-snapshot: the first Distance is DRIVING in the mock lane since the A2
  // fix (`mockEnforce.ts` scales the line to the authored value), so the
  // endpoints moved and the pre-commit coordinates no longer hit them. The old
  // pass relied on the exact identity-solve defect this program removed.
  line = (await getSketchSnapshot(page)).lines[0];
  await selectBothEndpoints();
  await constraintToolbar(page);
  await expect(distanceBtn).toBeEnabled();
  await distanceBtn.click();

  await expect(dimInput).toBeVisible();
  // The chip re-seeds from the actual geometry, which the first Distance DROVE
  // to `firstValue` — so offsetting from the FIRST authored value (not the
  // original seed) is what guarantees a provable R1 clash (|Δvalue| far past
  // the 1e-9 tolerance).
  const secondValue = firstValue + 70;
  await dimInput.fill(secondValue.toFixed(1));
  await dimInput.press("Enter");

  // No new row: the clashing dimension was dropped, not appended.
  await expect(rows()).toHaveCount(rowsAfterFirst);
  // The status hint names the pre-existing Distance constraint it clashed with.
  // (Scoped to the StatusBar: the near-action pulse repeats the same text.)
  await expect(page.getByTestId("status-hint")).toContainText(/conflicts with Distance/);
  // The sketch reverted to exactly its post-first-dimension state.
  await expect(dofPill(page)).toHaveText(dofAfterFirst);
});
