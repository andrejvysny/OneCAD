import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  dofPill,
  bodyOptions,
  findExtrudeHandle,
  extrudeDebug,
  CANVAS,
} from "./helpers";

/*
 * MODEL-HARDEN Wave 1 — the professional extrude commit gesture (mock lane).
 *
 * The old flow committed on the drag RELEASE. The Wave 1 flow keeps the tool
 * ARMED after release (live preview + editable chip cluster) and commits only on
 * an EXPLICIT gesture: Enter, the chip ✓, or a click away. Esc cancels and never
 * leaves a body behind. Each test re-arms a fresh single-region extrude (draw a
 * rectangle → Enter finishes → the single region auto-arms extrude directly).
 */
async function armExtrude(page: import("@playwright/test").Page): Promise<void> {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  // The plane-picker entry animates the camera (CadOrbitControls, 250ms) — settle
  // before any draw click races it (see helpers.ts waitForCameraSettled).
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  await page.keyboard.press("Enter"); // finish → single region → auto-arms extrude
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
}

/**
 * Drag the depth handle and release (which now KEEPS the tool armed). The grab must
 * actually land — the handle scan and the real pointerdown are a frame apart, so a
 * miss would leave the tool armed the WHOLE time and make "release keeps armed" pass
 * vacuously (finding 15). Assert the phase became "dragging" mid-drag; a single-frame
 * miss re-scans + re-grabs, a persistent miss fails loudly.
 */
async function dragReleaseHandle(page: import("@playwright/test").Page): Promise<void> {
  await expect(async () => {
    const { x, y } = await findExtrudeHandle(page);
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x, y + 12, { steps: 4 });
    const phase = (await extrudeDebug(page))?.phase;
    if (phase !== "dragging") {
      await page.mouse.up(); // release the missed grab before retrying
      throw new Error(`extrude grab missed — phase was ${String(phase)}`);
    }
  }).toPass({ timeout: 10_000, intervals: [200, 400, 800] });
  await page.mouse.up();
}

test("release keeps the tool armed with a chip cluster; Enter commits a body", async ({ page }) => {
  await armExtrude(page);
  const bodiesBefore = await bodyOptions(page).count();

  await dragReleaseHandle(page);

  // Release did NOT commit — the tool is still armed (debug surface) with the chip.
  expect((await extrudeDebug(page))?.phase).toBe("armed");
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
  await expect(page.getByRole("button", { name: "Extrude", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore); // nothing committed yet

  // Explicit confirm.
  await page.keyboard.press("Enter");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect(bodyOptions(page).last()).toHaveAttribute("aria-selected", "true");
});

test("Esc after release cancels the tool and creates no body", async ({ page }) => {
  await armExtrude(page);
  const bodiesBefore = await bodyOptions(page).count();

  await dragReleaseHandle(page);
  expect((await extrudeDebug(page))?.phase).toBe("armed");

  await page.keyboard.press("Escape");
  // Esc ladder tail: the tool returns to Select and the preview is discarded.
  await expect(page.getByRole("button", { name: "Extrude", exact: true })).not.toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore); // no body
});

test("clicking empty canvas away from the handle commits (click-away)", async ({ page }) => {
  await armExtrude(page);
  const bodiesBefore = await bodyOptions(page).count();

  // A true click on empty canvas, off the handle, biased toward centre so it clears
  // the floating side panels (tree left / inspector right / toolbar top).
  const box = await page.locator(CANVAS).boundingBox();
  const h = await findExtrudeHandle(page);
  const cx = (box?.x ?? 0) + (box?.width ?? 0) / 2;
  const tx = h.x + (h.x < cx ? 120 : -120);
  await page.mouse.move(tx, h.y);
  await page.mouse.down();
  await page.mouse.up();

  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
});
