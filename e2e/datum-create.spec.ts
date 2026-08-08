import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  waitForCameraSettled,
  findPlaneQuad,
  datumOptions,
  getDatumNames,
  extrudeDebug,
} from "./helpers";

/*
 * DATUM W1 — creating an offset datum plane through the real user path.
 *
 * The tool is a two-phase gesture with no drag: arm (the plane picker appears) →
 * click a base quad (ghost + offset chip) → ✓ (or Enter) commits. There is
 * deliberately no click-away commit, so a stray canvas click can never author a
 * datum — the spec relies on that when it clicks around during the offset phase.
 *
 * The base quad is targeted by scanning the engine's OWN `planePickerHitTest`
 * (`findPlaneQuad`): all three origin quads meet at the world origin, so a
 * center click cannot name a plane.
 */

test.describe("datum plane — create", () => {
  test("tool → base plane → offset → ✓ creates 'Datum 1'; undo removes it", async ({ page }) => {
    await openEditorDebug(page);
    await expect(datumOptions(page)).toHaveCount(0);

    // ── arm: the picker opens and the tool waits for a base plane ──
    await page.getByRole("button", { name: "Datum plane", exact: true }).click();
    await expect(page.getByText(/Select a base plane for the datum/)).toBeVisible();
    // Arming re-homes the camera when the view is axis-aligned; let it settle
    // before raycasting (the quads ARE the hit geometry).
    await waitForCameraSettled(page);
    await expect.poll(async () => (await extrudeDebug(page))?.datumPhase).toBe("basePick");

    // ── base pick: click the XY quad ──
    const quad = await findPlaneQuad(page, "XY");
    await page.mouse.move(quad.x, quad.y);
    await page.mouse.down();
    await page.mouse.up();

    // ── offset phase: ghost + chip, seeded at 10mm ──
    await expect.poll(async () => (await extrudeDebug(page))?.datumPhase).toBe("offset");
    const chip = page.getByTestId("model-tool-chip");
    await expect(chip.getByTestId("chip-datum-base")).toHaveText("XY");
    const input = chip.getByLabel("Dimension value");
    await expect(input).toHaveValue("10");
    // The ghost is on screen — a datum that will exist, not one that does.
    expect(
      await page.evaluate(
        () =>
          (window as unknown as { __datumVisuals?: { ghostVisible: boolean } }).__datumVisuals
            ?.ghostVisible ?? false,
      ),
    ).toBe(true);
    // Still nothing in the tree: the offset phase commits NOTHING on its own.
    await expect(datumOptions(page)).toHaveCount(0);

    // ── set an offset, then confirm with the chip ✓ ──
    await input.fill("24");
    await input.blur();
    await expect.poll(async () => (await extrudeDebug(page))?.datumOffset).toBe(24);

    await chip.getByTestId("chip-confirm").click();

    await expect(datumOptions(page)).toHaveCount(1);
    await expect(datumOptions(page).first()).toContainText("Datum 1");
    await expect(page.getByText("Datum 1 created")).toBeVisible();
    // The tool returned to Select, and every trace of the arm is gone.
    await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.getByTestId("model-tool-chip")).toHaveCount(0);
    await expect.poll(async () => (await extrudeDebug(page))?.datumPhase).toBe("idle");

    // The BACKEND-resolved frame is what landed (XY offset 24 ⇒ world +Z 24).
    expect(
      await page.evaluate(() => {
        const w = window as unknown as {
          __stores?: {
            document: {
              getState(): { datums: Record<string, { plane: { origin: number[] }; offset: number }> };
            };
          };
        };
        const d = Object.values(w.__stores?.document.getState().datums ?? {})[0];
        return { origin: d?.plane.origin, offset: d?.offset };
      }),
    ).toEqual({ origin: [0, 0, 24], offset: 24 });

    // ── undo removes it (datums ride the document history, not local UI state) ──
    await page.keyboard.press("Meta+z");
    await expect(datumOptions(page)).toHaveCount(0);
    expect(await getDatumNames(page)).toEqual([]);

    // …and redo brings it back.
    await page.keyboard.press("Meta+Shift+z");
    await expect(datumOptions(page)).toHaveCount(1);
  });

  test("Esc during the BASE PICK cancels with nothing created", async ({ page }) => {
    await openEditorDebug(page);
    await page.getByRole("button", { name: "Datum plane", exact: true }).click();
    await expect(page.getByText(/Select a base plane for the datum/)).toBeVisible();

    await page.keyboard.press("Escape");

    await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(datumOptions(page)).toHaveCount(0);
    await expect.poll(async () => (await extrudeDebug(page))?.datumPhase).toBe("idle");
  });

  test("Esc during the OFFSET phase drops the ghost + chip and creates nothing", async ({ page }) => {
    await openEditorDebug(page);
    await page.getByRole("button", { name: "Datum plane", exact: true }).click();
    await waitForCameraSettled(page);
    const quad = await findPlaneQuad(page, "XY");
    await page.mouse.move(quad.x, quad.y);
    await page.mouse.down();
    await page.mouse.up();
    await expect(page.getByTestId("model-tool-chip")).toBeVisible();

    await page.keyboard.press("Escape");

    await expect(page.getByTestId("model-tool-chip")).toHaveCount(0);
    await expect(datumOptions(page)).toHaveCount(0);
    expect(
      await page.evaluate(
        () =>
          (window as unknown as { __datumVisuals?: { ghostVisible: boolean } }).__datumVisuals
            ?.ghostVisible ?? false,
      ),
    ).toBe(false);
  });

  test("the D shortcut arms the same tool", async ({ page }) => {
    await openEditorDebug(page);
    await page.keyboard.press("d");
    await expect(page.getByRole("button", { name: "Datum plane", exact: true })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.getByText(/Select a base plane for the datum/)).toBeVisible();
  });
});
