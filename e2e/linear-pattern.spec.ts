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

test("linear pattern: select body → P → Apply commits a Linear Pattern row", async ({
  page,
}) => {
  await openEditorDebug(page);
  const before = await documentBodyIds(page);
  expect(before).toContain(BODY);

  await seedSelection(page, [{ kind: "body", id: BODY }]);
  await page.keyboard.press("p");

  const chip = page.getByTestId("model-tool-chip");
  await expect(chip).toBeVisible();
  await expect(page.getByTestId("chip-pattern-axis-x")).toBeVisible();
  // U6: Total is an editable field now (typing reaches the worker maximum), so
  // the count reads off `value` rather than text content.
  await expect(page.getByTestId("pattern-count")).toHaveValue(/^[0-9]+$/);

  await page.getByTestId("chip-confirm").click();

  await expect.poll(async () => await getFeatureLabels(page)).toContain("Linear Pattern");
  const after = await documentBodyIds(page);
  expect(after).toContain(BODY);
  await expect(chip).toBeHidden();
});
