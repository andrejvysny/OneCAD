import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  waitForCameraSettled,
  findPlaneQuad,
  datumOptions,
  extrudeDebug,
} from "./helpers";

/*
 * WP-C2 — the document display unit, end to end in the mock lane.
 *
 * The load-bearing assertion is the one a screenshot cannot make: the DOCUMENT
 * never leaves millimetres. Every test below pairs a DISPLAY assertion (what
 * the chip / status bar reads) with a WIRE assertion (what the FSM and the
 * document actually hold), because a conversion leaking into the marshalling
 * path is the failure mode that would silently corrupt a model — the
 * `angleUnits.ts` bug class, in a new domain.
 *
 * The datum-offset chip is the vehicle: it is the shortest real path from a
 * typed number to a stored document value (seeded 10 mm → `datumOffset` →
 * `datums[].plane.origin`), with no solver in between to muddy the arithmetic.
 */

const displayButton = (page: Page) => page.getByRole("button", { name: /^Display mode/ });

/**
 * Pick a display unit through the REAL control.
 *
 * The camera settle is not optional (theme.spec.ts documents why): the ViewCube
 * re-renders on every camera change and the popover lives in the same subtree,
 * so clicking a tab while the intro framing animates detaches the node
 * mid-click.
 */
async function pickUnit(
  page: Page,
  unit: "mm" | "cm" | "m" | "in",
  close: "escape" | "toggle" = "escape",
): Promise<void> {
  await waitForCameraSettled(page);
  const button = displayButton(page);
  await button.click();
  const tab = page.getByRole("tablist", { name: "Units" }).getByRole("tab", { name: unit });
  await expect(tab).toBeVisible();
  await tab.click();
  // Escape is the natural dismiss, but it ALSO cancels an armed model tool
  // (datum-create.spec.ts pins that) — so a test holding a live chip closes the
  // popover by toggling its own anchor button instead. The popover's
  // outside-click handler deliberately ignores the anchor, so the toggle is a
  // single clean close rather than a close-then-reopen race.
  if (close === "escape") await page.keyboard.press("Escape");
  else await button.click();
  await expect(page.getByRole("tablist", { name: "Units" })).toHaveCount(0);
}

const storedUnit = (page: Page) =>
  page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { settings: { getState(): { displayUnit: string } } };
    };
    return w.__stores?.settings.getState().displayUnit ?? "?";
  });

/** The status-bar mono cursor read-out. */
const cursorRow = (page: Page) => page.locator("div.h-\\[34px\\] span.font-mono").last();

/** Arm the datum tool and pick the XY base quad — leaves the offset chip open. */
async function armDatumOffsetChip(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Datum plane", exact: true }).click();
  await waitForCameraSettled(page);
  await expect.poll(async () => (await extrudeDebug(page))?.datumPhase).toBe("basePick");
  const quad = await findPlaneQuad(page, "XY");
  await page.mouse.move(quad.x, quad.y);
  await page.mouse.down();
  await page.mouse.up();
  await expect.poll(async () => (await extrudeDebug(page))?.datumPhase).toBe("offset");
}

test("the unit picker re-renders the status-bar cursor read-out", async ({ page }) => {
  await openEditorDebug(page);

  await expect(cursorRow(page)).toContainText("mm");
  const inMm = (await cursorRow(page).textContent()) ?? "";

  await pickUnit(page, "in");
  await expect(cursorRow(page)).toContainText("in");
  expect(await cursorRow(page).textContent()).not.toBe(inMm);

  await pickUnit(page, "mm");
  await expect(cursorRow(page)).toContainText("mm");
});

test("a chip shows the stored millimetres in the picked unit, geometry untouched", async ({
  page,
}) => {
  await openEditorDebug(page);
  await pickUnit(page, "in");

  await armDatumOffsetChip(page);
  const chip = page.getByTestId("model-tool-chip");
  const input = chip.getByLabel("Dimension value");

  // The FSM still holds the 10 mm seed; only the FIELD moved. 10 / 25.4 =
  // 0.3937007…, four decimals.
  await expect(input).toHaveValue("0.3937");
  await expect(chip).toContainText("in");
  expect((await extrudeDebug(page))?.datumOffset).toBe(10);
});

test("a BARE number is read in the display unit and stored as MILLIMETRES", async ({ page }) => {
  await openEditorDebug(page);
  await pickUnit(page, "in");
  await armDatumOffsetChip(page);

  const chip = page.getByTestId("model-tool-chip");
  const input = chip.getByLabel("Dimension value");

  // Displaying inches, "1" means one INCH — the professional convention.
  await input.fill("1");
  await input.blur();
  await expect.poll(async () => (await extrudeDebug(page))?.datumOffset).toBe(25.4);

  await chip.getByTestId("chip-confirm").click();
  await expect(datumOptions(page)).toHaveCount(1);

  // What LANDED in the document is millimetres, exactly — 25.4, not 1, and not
  // a float that merely rounds to 25.4.
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
  ).toEqual({ origin: [0, 0, 25.4], offset: 25.4 });
});

test("an explicit suffix overrides the display unit", async ({ page }) => {
  await openEditorDebug(page);
  await pickUnit(page, "in");
  await armDatumOffsetChip(page);

  const input = page.getByTestId("model-tool-chip").getByLabel("Dimension value");
  await input.fill("2 mm");
  await input.blur();
  await expect.poll(async () => (await extrudeDebug(page))?.datumOffset).toBe(2);
});

test("switching back to mm re-labels the same value — the document does not move", async ({
  page,
}) => {
  await openEditorDebug(page);
  await armDatumOffsetChip(page);

  const chip = page.getByTestId("model-tool-chip");
  const input = chip.getByLabel("Dimension value");
  await input.fill("50.8");
  await input.blur();
  await expect.poll(async () => (await extrudeDebug(page))?.datumOffset).toBe(50.8);

  // The picker is reachable with the chip open (it is viewport chrome, not a
  // modal), so this is a live re-display of an armed value.
  await pickUnit(page, "in", "toggle");
  await expect(input).toHaveValue("2");
  await expect(chip).toContainText("in");
  expect((await extrudeDebug(page))?.datumOffset).toBe(50.8);

  await pickUnit(page, "mm", "toggle");
  await expect(input).toHaveValue("50.8");
  await expect(chip).toContainText("mm");
  expect((await extrudeDebug(page))?.datumOffset).toBe(50.8);

  // …and the value that was armed all along is the one that lands.
  await chip.getByTestId("chip-confirm").click();
  await expect(datumOptions(page)).toHaveCount(1);
});

test("the unit preference survives a fresh page load", async ({ page }) => {
  await openEditorDebug(page);
  await pickUnit(page, "in");
  await expect.poll(() => storedUnit(page)).toBe("in");

  // A full re-navigation, not a soft reset: this is the returning-user path,
  // where the ONLY carrier is localStorage (the theme-boot precedent).
  await openEditorDebug(page);

  await expect.poll(() => storedUnit(page)).toBe("in");
  await expect(cursorRow(page)).toContainText("in");
});

test("a pre-v6 settings blob boots in millimetres", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      "onecad.settings",
      JSON.stringify({ state: { theme: "light", displayMode: "wireframe" }, version: 5 }),
    );
  });

  await openEditorDebug(page);

  expect(await storedUnit(page)).toBe("mm");
  await expect(cursorRow(page)).toContainText("mm");
});
