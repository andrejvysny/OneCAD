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

/*
 * A revert that reverts nothing has to SAY so (WP-U1). Before this, ⌘Z on an
 * empty history was indistinguishable from a broken ⌘Z: the chord fired, the
 * client answered, and the screen did not move.
 */
test("⌘Z on an empty history reports 'Nothing to undo' instead of going silent", async ({
  page,
}) => {
  await openEditorDebug(page);
  await page.keyboard.press("Meta+z");
  await expect(page.getByTestId("status-hint")).toHaveText("Nothing to undo");

  await page.keyboard.press("Meta+Shift+z");
  await expect(page.getByTestId("status-hint")).toHaveText("Nothing to redo");
});

/*
 * …and the chord is not the only way in. The ⌘K palette carries Undo/Redo rows
 * enabled by history DEPTH and named by the step on top of the stack, so a user
 * who never learned the chord can still find it — and can see whether there is
 * anything to take back.
 */
test("the ⌘K palette offers Undo, named after the step it would revert", async ({ page }) => {
  await openEditorDebug(page);
  const openPalette = async () => {
    await page.keyboard.press(process.platform === "darwin" ? "Meta+k" : "Control+k");
    await expect(page.getByTestId("command-palette")).toBeVisible();
    await page.getByTestId("command-palette-input").fill("undo");
  };
  const undoRow = page.getByTestId("palette-item-onecad.modeling.command.undo");

  // Nothing edited yet: the row is there, dimmed, and says why.
  await openPalette();
  await expect(undoRow).toBeVisible();
  await expect(undoRow).toHaveText(/Undo/);
  await expect(undoRow).toHaveAttribute("aria-disabled", "true");
  await expect(page.getByTestId("palette-item-onecad.modeling.command.redo")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("command-palette")).toBeHidden();

  // One real edit (a suppress toggle) later, it names that step and is runnable.
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  await page.getByTestId("history-suppress-f3").click();
  await expect.poll(() => featureSuppressed(page, "f3")).toBe(true);

  await openPalette();
  await expect(undoRow).toHaveText(/Undo Suppress/);
  await expect(undoRow).toHaveAttribute("aria-disabled", "false");

  // Running it from the palette goes through the same router the chord does.
  await undoRow.click();
  await expect(page.getByTestId("command-palette")).toBeHidden();
  await expect.poll(() => featureSuppressed(page, "f3")).toBeFalsy();
});
