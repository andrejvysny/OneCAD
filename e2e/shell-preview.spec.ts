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

/** A second face on the same body — the other half of a multi-face open set. */
const SECOND_FACE_REF = {
  kind: "face",
  id: "body1#f:3",
  bodyId: "body1",
  topoKey: "f:3",
  elementId: "el-face-3",
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
  // The toolbar button itself is disabled with nothing selected (below) — this
  // scenario is the reactive fallback for reaching the tool without it, e.g. the
  // keyboard shortcut, same pattern as filletChamfer.spec.ts's equivalent test.
  await page.keyboard.press("k");
  await expect(page.getByText("Select faces to remove, then Shell")).toBeVisible();
  expect((await toolPhases(page))?.shellPhase).toBe("idle");
});

test("the Shell toolbar button grays out and explains why when nothing is selected", async ({ page }) => {
  await openEditorDebug(page);
  const shellButton = page.getByRole("button", { name: "Shell", exact: true });
  await expect(shellButton).toHaveAttribute("aria-disabled", "true");
  await shellButton.hover();
  await expect(page.getByRole("tooltip")).toHaveText("Select faces to remove, then Shell");
  // A disabled button is truly inert — clicking it must not arm the tool
  // (no debug-surface tool switch ever fires, so assert on the DOM instead).
  await shellButton.click({ force: true });
  await expect(shellButton).toHaveAttribute("aria-pressed", "false");
  await expect(page.getByTestId("chip-confirm")).toHaveCount(0);
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

/*
 * MULTI-FACE. Every other shell assertion in this repo's browser lane seeds one
 * face, so nothing here distinguished a tool that hollows the whole selection from
 * one that quietly keeps only the first pick.
 *
 * What this lane can honestly see, and what it cannot: the armed hint is
 * `armShell`'s own readout of `this.shellFaces.length`, and `this.shellFaces` is
 * the very array `shellParams()` maps into `openFaces` — so "Shell 2 faces" is a
 * direct observation that BOTH picks reached the tool and will both reach the
 * record. The committed `openFaces` array itself has no browser surface (there is
 * no client handle on `window`, and a shell re-edit is thickness-only), and the
 * mock lane has no CSG, so the geometry of a two-face shell is pinned where it is
 * measurable: `worker/tests/test_m6a_ops.cpp test_shell_two_open_faces` and
 * `src-tauri/tests/preview_edge_shell.rs shell_removes_two_open_faces_in_one_op`
 * both take the 20×20×25 box to the exact multi-face volume.
 */
test("a two-face selection arms ONE shell over both and commits a single row", async ({ page }) => {
  await openEditorDebug(page);
  await seedSelection(page, [FACE_REF, SECOND_FACE_REF]);
  const before = await getFeatureLabels(page);

  await page.getByRole("button", { name: "Shell", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.shellPhase).toBe("armed");
  // Plural, and counted: a one-pick regression reads "Shell 1 face".
  await expect(page.getByText(/Shell 2 faces/)).toBeVisible();
  await expect(page.getByTestId("chip-confirm")).toBeVisible();

  await page.getByTestId("chip-confirm").click();
  await expect.poll(async () => (await toolPhases(page))?.shellPhase).toBe("idle");
  // TWO open faces are still ONE Shell feature, not one row per face.
  const after = await getFeatureLabels(page);
  expect(after.length).toBe(before.length + 1);
  expect(after.at(-1)).toBe("Shell");
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
