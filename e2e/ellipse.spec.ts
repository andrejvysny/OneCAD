import { test, expect } from "./fixtures";
import {
  hideSeedSketches,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  sketchOptions,
  bodyOptions,
  getSketchEllipse,
  getSketchSnapshot,
  planePointToClient,
} from "./helpers";

/*
 * Ellipse tool (W3 P3) — mirrors e2e/circle.spec.ts, one click longer.
 *
 * A lone ellipse contributes exactly 5 free DOF (centre 2 + majorR + minorR +
 * rotation) with no autoconstraints, so the DOF pill lands on an exact,
 * deterministic value (mock `entityFreedom`, src/ipc/mockSketch.ts). It is a
 * SELF-CLOSED curve, so `detectRegions` publishes it as its own planar cell —
 * which is what makes the fill pickable and Extrude armable on it, exactly like
 * a circle.
 *
 * Region targeting reads the REAL ellipse back from the live session
 * (`getSketchEllipse`, before Enter clears it) and projects its centre through
 * the live camera — a canvas-relative pixel offset would not survive the
 * finish-sketch camera restore (see the multiregion spec's note).
 */

test("ellipse tool draws an ellipse and reports its degrees of freedom", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  const sketchCount = await sketchOptions(page).count();
  await selectSketchTool(page, "Ellipse");

  await expect(dofPill(page)).toHaveText("DOF: 0");

  // Centre → major-axis endpoint → minor extent (3 clicks).
  await clickAt(page, 0, 0);
  await clickAt(page, 160, 0);
  await clickAt(page, 0, 70);

  // 5 DOF exactly; no relationships inferred for a solitary ellipse.
  // U7: the centre click lands on the sketch origin, whose snap is now offered —
  // accepting it persists a `Fixed`, so the two translation DOF are already gone.
  await expect(dofPill(page)).toHaveText("DOF: 3");
  await expect(page.getByText("Under-constrained · DOF 3").first()).toBeVisible();
  // No RELATIONSHIP was inferred — a solitary ellipse has nothing to relate to.
  // The one constraint present is the origin anchor the centre click accepted (U7).
  await expect(page.getByText("No constraints yet.")).toHaveCount(0);

  // The committed entity is normalized: major ≥ minor, rotation in [0, 2π).
  const ell = await getSketchEllipse(page);
  expect(ell.majorR).toBeGreaterThanOrEqual(ell.minorR);
  expect(ell.minorR).toBeGreaterThan(0);
  expect(ell.rotation).toBeGreaterThanOrEqual(0);
  expect(ell.rotation).toBeLessThan(Math.PI * 2);

  // Finish cleanly back to model mode; the ellipse persists as a sketch.
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await expect(sketchOptions(page)).toHaveCount(sketchCount);
});

test("the O key arms the ellipse tool (and never shadows E = extrude)", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);

  const ellipseBtn = page.getByRole("button", { name: "Ellipse", exact: true });
  // Ellipse shares a toolbar slot with Circle (family `sketch.curve`), and the
  // slot opens on its default member — so before `o` there is no Ellipse button
  // at all, which is a STRONGER statement than "present but unpressed". Pressing
  // `o` has to both arm the tool and pull it into the slot.
  await expect(ellipseBtn).toHaveCount(0);
  await page.keyboard.press("o");
  await expect(ellipseBtn).toHaveAttribute("aria-pressed", "true");

  // Draw with the keyboard-armed tool to prove it is the real machine, not just
  // a highlighted button.
  await clickAt(page, 0, 0);
  await clickAt(page, 140, 0);
  await clickAt(page, 0, 60);
  // U7: the centre click lands on the sketch origin, whose snap is now offered —
  // accepting it persists a `Fixed`, so the two translation DOF are already gone.
  await expect(dofPill(page)).toHaveText("DOF: 3");

  // `E` in sketch mode is still the cross-mode extrude handoff: it finishes the
  // sketch and carries the Extrude intent into model mode (U2 — the full flow is
  // e2e/auto-mode.spec.ts's job; here it only has to prove `O` didn't steal the
  // letter). The ellipse is the sole extrudable region, so the tool arms outright.
  await page.keyboard.press("e");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
});

test("the ellipse publishes a pickable region fill that arms extrude", async ({ page }) => {
  await openEditorDebug(page);
  // Hide the mock's seed body + seed sketches so nothing else can win the raycast
  // (same setup as e2e/multiregion.spec.ts).
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await selectSketchTool(page, "Ellipse");
  await clickAt(page, 0, 0);
  await clickAt(page, 170, 0);
  await clickAt(page, 0, 80);
  // U7: the centre click lands on the sketch origin, whose snap is now offered —
  // accepting it persists a `Fixed`, so the two translation DOF are already gone.
  await expect(dofPill(page)).toHaveText("DOF: 3");

  // Snapshot BEFORE finishing — the session clears once the mode leaves "sketch".
  const ell = await getSketchEllipse(page);
  const { plane } = await getSketchSnapshot(page);

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);

  // The static layer publishes the ellipse's fill: a raycast at its centre hits.
  const centre = await planePointToClient(page, plane, { x: ell.center[0], y: ell.center[1] });
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          Boolean(
            (window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
              .__vpEngine?.sketchStaticHitTest(x, y),
          ),
        centre,
      ),
    )
    .toBe(true);

  // A point just OUTSIDE the minor axis but inside the major-axis bbox must NOT
  // be filled — the region follows the ellipse, not its bounding box.
  const cornerish = await planePointToClient(page, plane, {
    x: ell.center[0] + ell.majorR * 0.95 * Math.cos(ell.rotation) - ell.minorR * 0.95 * Math.sin(ell.rotation),
    y: ell.center[1] + ell.majorR * 0.95 * Math.sin(ell.rotation) + ell.minorR * 0.95 * Math.cos(ell.rotation),
  });
  expect(
    await page.evaluate(
      ({ x, y }) =>
        Boolean(
          (window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
            .__vpEngine?.sketchStaticHitTest(x, y),
        ),
      cornerish,
    ),
  ).toBe(false);

  // Picking the fill selects a sketchRegion, and Extrude arms on it.
  await clickAtClient(page, centre.x, centre.y);
  const selected = await page.evaluate(() => {
    const sel = (window as unknown as {
      __stores?: {
        selection: {
          getState(): { selected: Array<{ kind: string; sketchId?: string; regionId?: string }> };
        };
      };
    }).__stores?.selection.getState().selected;
    return sel?.[0] ?? null;
  });
  expect(selected?.kind).toBe("sketchRegion");
  expect(selected?.regionId).toBeTruthy();

  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
});
