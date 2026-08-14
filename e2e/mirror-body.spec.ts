import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, getFeatureLabels, bodyOptions } from "./helpers";
import { seedSelection } from "./modelToolHelpers";

const BODY = "body1";

async function documentBodyIds(page: Page): Promise<string[]> {
  return page.evaluate(
    () =>
      Object.keys(
        (
          window as unknown as {
            __stores?: { document: { getState(): { bodies: Record<string, unknown> } } };
          }
        ).__stores?.document.getState().bodies ?? {},
      ),
  );
}

async function clickConfirmButton(page: Page): Promise<void> {
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
  await page.getByTestId("chip-confirm").click();
}

test("mirror body: select body → M → YZ plane → Apply creates a mirrored copy", async ({
  page,
}) => {
  await openEditorDebug(page);
  const before = await documentBodyIds(page);
  expect(before).toContain(BODY);

  await seedSelection(page, [{ kind: "body", id: BODY }]);
  await page.keyboard.press("m");

  const chip = page.getByTestId("model-tool-chip");
  await expect(chip).toBeVisible();

  await page.getByTestId("chip-mirror-plane-yz").click();
  await clickConfirmButton(page);

  await expect.poll(async () => await getFeatureLabels(page)).toContain("Mirror");
  const after = await documentBodyIds(page);
  expect(after).toContain(BODY);
  await expect(page.getByTestId("model-tool-chip")).toBeHidden();
});
