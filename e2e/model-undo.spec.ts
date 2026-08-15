import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  hideSeedSketches,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  getFeatureLabels,
  getSketchSnapshot,
  planePointToClient,
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * TRUST wave — model-mode ⌘Z / ⇧⌘Z round-trip.
 *
 * Two things must follow the document history, not local UI state:
 *   (1) a committed OP — undo drops its body + timeline row, redo restores both;
 *   (2) a SUPPRESS toggle — the dim state is `FeatureDto.suppressed` off the
 *       projection (the optimistic overlay is gone), so undoing a suppression has
 *       to un-dim the row purely by re-hydrating the timeline. A frontend overlay
 *       could never have followed an undo at all: nothing tells it to flip back.
 *
 * Mock lane: `mockClient.undo/redo` restore whole-document snapshots, and the
 * suppression EditCommand pushes one — mirroring the real backend, where core
 * `session.apply` mints an inverse for `SetOperationSuppression` like any other
 * command.
 */

/** The `suppressed` flag of a projection feature (`__stores.document`, dev-only). */
async function featureSuppressed(page: Page, id: string): Promise<boolean | undefined> {
  return page.evaluate((featureId) => {
    const w = window as unknown as {
      __stores?: {
        document: { getState(): { features: Array<{ id: string; suppressed?: boolean }> } };
      };
    };
    return w.__stores?.document.getState().features.find((f) => f.id === featureId)?.suppressed;
  }, id);
}

/** Production profile-first arm: draw a rectangle → finish → pick its region → Extrude. */
async function armExtrude(page: Page): Promise<void> {
  await openEditorDebug(page);
  // Hide the seeded body + sketches so the region click hits the new profile and
  // not an occluding face (it stays listed in the tree, so body COUNTS are stable).
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  const snap = await getSketchSnapshot(page);
  const p0 = snap.lines[0]?.p0;
  const p2 = snap.lines[2]?.p0;
  if (!p0 || !p2) throw new Error("rectangle snapshot is incomplete");
  const centroid = { x: (p0[0] + p2[0]) / 2, y: (p0[1] + p2[1]) / 2 };

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  const client = await planePointToClient(page, snap.plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          Boolean(
            (window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
              .__vpEngine?.sketchStaticHitTest(x, y),
          ),
        client,
      ),
    )
    .toBe(true);
  await clickAtClient(page, client.x, client.y);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as {
            __stores?: { selection: { getState(): { selected: Array<{ kind: string }> } } };
          }).__stores?.selection.getState().selected[0]?.kind,
      ),
    )
    .toBe("sketchRegion");
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
}

test("model-mode ⌘Z / ⇧⌘Z round-trips a committed op and a suppress toggle", async ({ page }) => {
  await armExtrude(page);
  const bodiesBefore = await bodyOptions(page).count();
  const featuresBefore = (await getFeatureLabels(page)).length;

  // ── (1) a committed op ─────────────────────────────────────────────────────
  await commitExtrudeAtHandle(page);
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(featuresBefore + 1);

  await page.keyboard.press("Meta+z");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore);
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(featuresBefore);

  await page.keyboard.press("Meta+Shift+z");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(featuresBefore + 1);

  // ── (2) a suppress toggle ──────────────────────────────────────────────────
  // Select a body, then a history chip, to reach the per-row affordances.
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  const row = page.getByTestId("history-row-f3");
  await expect(page.getByTestId("history-suppress-f3")).toBeVisible();

  await page.getByTestId("history-suppress-f3").click();
  await expect(row).toHaveClass(/opacity-60/);
  expect(await featureSuppressed(page, "f3")).toBe(true);

  // Undo the suppression: the row un-dims because the restored PROJECTION says so.
  await page.keyboard.press("Meta+z");
  await expect(row).not.toHaveClass(/opacity-60/);
  await expect.poll(() => featureSuppressed(page, "f3")).toBeFalsy();

  await page.keyboard.press("Meta+Shift+z");
  await expect(row).toHaveClass(/opacity-60/);
  expect(await featureSuppressed(page, "f3")).toBe(true);
});
