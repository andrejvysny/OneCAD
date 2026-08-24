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

/** The armed mirror's `fuseWithOriginal` toggle (WP6). */
const fuseToggle = (page: Page) => page.getByTestId("chip-mirror-fuse");

/** Select the body, press `M`, and wait for the armed mirror chip. */
async function armMirror(page: Page): Promise<void> {
  await seedSelection(page, [{ kind: "body", id: BODY }]);
  await page.keyboard.press("m");
  await expect(page.getByTestId("model-tool-chip")).toBeVisible();
  await expect(fuseToggle(page)).toBeVisible();
}

/**
 * Commit the armed chip and wait for the tool to come all the way back to rest.
 *
 * The chip vanishing is NOT that moment: `commitPattern` clears the chip before
 * it awaits the backend and only resets the tool afterwards, so a gesture issued
 * on "chip gone" races the trailing `setTool("select")` — whose `onToolChange`
 * bumps `armGen` and silently supersedes an in-flight re-edit arm. Measured: the
 * next re-edit's `getOperationParams` resolved against a bumped generation and
 * returned without arming anything.
 */
async function commitAndSettle(page: Page): Promise<void> {
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
  await page.getByTestId("chip-confirm").click();
  await expect(page.getByTestId("model-tool-chip")).toBeHidden();
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              __stores?: { tool: { getState(): { modelTool: string } } };
            }
          ).__stores?.tool.getState().modelTool,
      ),
    )
    .toBe("select");
}

/**
 * Re-open the committed Mirror row — the ONLY read-back the browser lane has of
 * what was actually committed: the re-edit seeds itself from
 * `getOperationParams`, i.e. the stored WIRE params, so the toggle's state on
 * re-arm IS the committed `fuseWithOriginal` and nothing else.
 *
 * The row is SELECTED through the store rather than clicked: selecting a feature
 * swaps the inspector from its three-row lineage view to the full timeline, and a
 * pointer gesture issued into that reflow is torn out from under itself. The
 * double-click — the gesture actually under test — still goes through the DOM.
 */
async function reopenMirrorRow(page: Page): Promise<void> {
  const id = await page.evaluate(
    () =>
      (
        window as unknown as {
          __stores?: {
            document: { getState(): { features: Array<{ id: string; label: string }> } };
          };
        }
      ).__stores?.document.getState().features.find((f) => f.label === "Mirror")?.id,
  );
  if (!id) throw new Error("the projection has no Mirror row");
  await seedSelection(page, [{ kind: "feature", id }]);
  const row = page.getByTestId(`history-row-${id}`);
  await expect(row).toBeVisible();
  await row.dblclick();
  await expect(fuseToggle(page)).toBeVisible();
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

/*
 * WP6 item 2 — `MirrorBodyParams.fuseWithOriginal` reaches the authoring UI.
 *
 * The flag was always on the wire and always hard-coded `false` for a fresh
 * mirror, so the fused mode had no browser flow at all. What is proven here is
 * the whole loop, not just the button: the toggle flips, the committed record
 * carries the flipped value (read back through the re-edit's
 * `getOperationParams` seed), and flipping it again on the re-edit writes the
 * other value back onto the SAME row.
 */
test("the mirror fuse toggle flips the committed fuseWithOriginal, both ways", async ({ page }) => {
  await openEditorDebug(page);

  // A fresh mirror is NON-fused — the product default, unchanged by WP6.
  await armMirror(page);
  await expect(fuseToggle(page)).toHaveAttribute("aria-pressed", "false");
  await expect(page.getByTestId("chip-result-summary")).toHaveCount(0);

  // Turn it on. The lifecycle line states what the flag will do BEFORE the ✓.
  await fuseToggle(page).click();
  await expect(fuseToggle(page)).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("chip-result-summary")).toHaveText("Mirror · fused into the source");

  await commitAndSettle(page);
  await expect.poll(async () => await getFeatureLabels(page)).toContain("Mirror");

  // The committed record carries `fuseWithOriginal: true` — the re-edit seeds
  // the toggle from the stored params, so a pressed toggle IS that assertion.
  await reopenMirrorRow(page);
  await expect(fuseToggle(page)).toHaveAttribute("aria-pressed", "true");

  // …and the re-edit can turn it back off, rewriting the same row.
  await fuseToggle(page).click();
  await expect(page.getByTestId("chip-result-summary")).toHaveText(
    "Mirror · 1 new body · source retained",
  );
  await commitAndSettle(page);

  await reopenMirrorRow(page);
  await expect(fuseToggle(page)).toHaveAttribute("aria-pressed", "false");
  // One record throughout: a re-edit rewrites, it never appends.
  expect((await getFeatureLabels(page)).filter((l) => l === "Mirror")).toHaveLength(1);
});
