import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, getFeatureLabels } from "./helpers";
import { seedSelection, toolPhases, dragInCanvas } from "./modelToolHelpers";

/*
 * EDGE-OP PREVIEW wave — Shell joins the armed-commit gesture.
 *
 * Shell was the worst of the three blind-commit tools: hollowing is the op most
 * likely to be refused outright (a thickness thicker than the thinnest wall), and
 * the old flow committed it on the pointer RELEASE with no preview and no way to
 * back out — an OCCT refusal landed as an errored history row. It now arms with a
 * live kernel preview, stays armed through a release, and only commits on Enter or
 * the visible ✓ — with the ✓ BLOCKED while the newest candidate is a failure.
 *
 * GESTURE ONLY, same discipline as `filletChamfer.spec.ts`: the mock lane has no CSG and
 * publishes no candidate mesh, so there is nothing honest to assert about geometry
 * here. `src-tauri/tests/preview_edge_shell.rs shell_preview_matches_the_commit`
 * pins the real numbers (10000 → 2224, preview == commit) against the OCCT worker.
 */
const FACE_REF = {
  kind: "face",
  id: "body1#f:2",
  bodyId: "body1",
  topoKey: "f:2",
  elementId: "el-face-2",
  anchor: { worldPoint: [0, 0, 0] },
};

async function armShell(page: Page): Promise<void> {
  await seedSelection(page, [FACE_REF]);
  await page.getByRole("button", { name: "Shell", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.shellPhase).toBe("armed");
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
}

test("Shell asks for faces when nothing is selected", async ({ page }) => {
  await openEditorDebug(page);
  await page.getByRole("button", { name: "Shell", exact: true }).click();
  await expect(page.getByText("Select faces to remove, then Shell")).toBeVisible();
  expect((await toolPhases(page))?.shellPhase).toBe("idle");
});

test("a released shell drag stays ARMED and commits nothing", async ({ page }) => {
  await openEditorDebug(page);
  await armShell(page);
  const before = await getFeatureLabels(page);

  await dragInCanvas(page, 180, 120);

  await expect.poll(async () => (await toolPhases(page))?.shellPhase).toBe("armed");
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
  await expect(page.getByRole("button", { name: "Shell", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(await getFeatureLabels(page)).toEqual(before); // release NEVER commits
});

test("the visible ✓ commits the armed shell", async ({ page }) => {
  await openEditorDebug(page);
  await armShell(page);
  const before = await getFeatureLabels(page);

  await dragInCanvas(page, 180, 120);
  await page.getByTestId("chip-confirm").click();

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Shell");
  await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByTestId("chip-confirm")).toHaveCount(0);
});

test("Enter commits the armed shell; ✕ cancels it with no row", async ({ page }) => {
  await openEditorDebug(page);
  await armShell(page);
  const before = await getFeatureLabels(page);

  await page.getByTestId("chip-cancel").click();
  await expect.poll(async () => (await toolPhases(page))?.shellPhase).toBe("idle");
  expect(await getFeatureLabels(page)).toEqual(before);

  await armShell(page);
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Shell");
});
