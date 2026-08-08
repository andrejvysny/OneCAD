import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, waitForCameraSettled } from "./helpers";

async function cam(page: Page): Promise<{ dir: number[]; yaw: number; pitch: number }> {
  return page.evaluate(() => {
    const e = (window as unknown as { __vpEngine?: any }).__vpEngine;
    const d = e.getViewDirection();
    return { dir: [d.x, d.y, d.z], yaw: e.controls.yaw, pitch: e.controls.pitch };
  });
}

test.describe("ViewCube drag diagnostic", () => {
  test.beforeEach(async ({ page }) => {
    await openEditorDebug(page);
    await waitForCameraSettled(page);
  });

  test("drag orbits and does not snap", async ({ page }) => {
    const before = await cam(page);
    const face = page.getByRole("button", { name: "FRONT view" });
    await face.waitFor();
    const box = await face.boundingBox();
    const x = box!.x + box!.width / 2;
    const y = box!.y + box!.height / 2;

    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x + 20, y, { steps: 3 }); // mid-drag read
    const mid = await cam(page);
    await page.mouse.move(x + 60, y, { steps: 3 });
    await page.mouse.up();
    const after = await cam(page);

    const midMoved = Math.hypot(mid.dir[0] - before.dir[0], mid.dir[1] - before.dir[1], mid.dir[2] - before.dir[2]);
    const finalMoved = Math.hypot(after.dir[0] - before.dir[0], after.dir[1] - before.dir[1], after.dir[2] - before.dir[2]);
    // FRONT canonical: yaw=-π/2, pitch=0.
    const snapped = Math.abs(after.yaw + Math.PI / 2) < 1e-6 && Math.abs(after.pitch) < 1e-6;
    console.log("BEFORE", JSON.stringify(before));
    console.log("MID", JSON.stringify(mid), "midMoved", midMoved);
    console.log("AFTER", JSON.stringify(after), "finalMoved", finalMoved, "snapped", snapped);
    expect(midMoved).toBeGreaterThan(0.05);
    expect(finalMoved).toBeGreaterThan(0.05);
    expect(snapped).toBe(false);
  });
});
