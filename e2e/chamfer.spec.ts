import { test, expect, type Page } from "@playwright/test";
import { openEditorDebug, getFeatureLabels } from "./helpers";
import { seedSelection, toolPhases, dragInCanvas } from "./modelToolHelpers";

/*
 * MODEL-OPS W1 — Chamfer is a real tool.
 *
 * The worker has implemented it since W-WP6 (`FilletChamferOp.cpp
 * execute_chamfer`, sharing `FilletChamferParams` with Fillet and distinguished
 * by `mode`), SCHEMA §7.3 documents it in the Fillet block, and Rust has carried
 * `ChamferParams` all along — but `"Chamfer"` was absent from the `ModelTool`
 * union AND from the authorable `WireOperation` union, so no UI path could reach
 * it at all. The real-worker wire proof is `wire_contract.rs
 * chamfer_reaches_the_worker`; this spec pins the tool's presence and that it
 * shares the fillet's edge-selection gesture.
 */
test("the chamfer tool is selectable and prompts for edges like fillet", async ({ page }) => {
  await openEditorDebug(page);

  const chamfer = page.getByRole("button", { name: "Chamfer", exact: true });
  await expect(chamfer).toBeVisible();

  // No edge selected → the tool arms nothing and says what it needs, exactly as
  // Fillet does (armEdgeOpFromSelection's empty-selection guard).
  await chamfer.click();
  await expect(page.getByText("Select edges, then Chamfer")).toBeVisible();

  // H is the chamfer shortcut (F is fillet, ⇧F is zoom-fit).
  await page.keyboard.press("Escape");
  await page.keyboard.press("h");
  await expect(chamfer).toHaveAttribute("aria-pressed", "true");

  // It is a distinct tool, not a relabelled fillet.
  await page.keyboard.press("f");
  await expect(page.getByRole("button", { name: "Fillet", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(chamfer).not.toHaveAttribute("aria-pressed", "true");

  // The seeded document's features still read correctly (default_label now names
  // the real op rather than the coarse FeatureKind bucket).
  const labels = await getFeatureLabels(page);
  expect(labels.length).toBeGreaterThan(0);
});

/*
 * EDGE-OP PREVIEW wave — the armed-commit gesture.
 *
 * Chamfer (and Fillet, and Shell) used to commit on the pointer RELEASE with no
 * preview at all: drag a number, let go, find out from the history row whether
 * OCCT accepted it. They now behave like Extrude/Revolve — release keeps the tool
 * ARMED with a live kernel preview and an editable chip, and only Enter or the
 * visible ✓ writes to the timeline.
 *
 * GESTURE ONLY: the mock lane has no CSG, so it publishes no candidate geometry
 * (`localSolver` settles the epoch with an EMPTY result rather than faking a mesh
 * it cannot honestly synthesize). Geometry equality is pinned where the kernel
 * actually runs — `src-tauri/tests/preview_edge_shell.rs` and the worker ctest
 * `preview_op`. Here we assert the interaction contract and nothing else.
 */
const EDGE_REF = {
  kind: "edge",
  id: "body1#e:5",
  bodyId: "body1",
  topoKey: "e:5",
  elementId: "el-edge-5",
  anchor: { worldPoint: [0, 0, 0] },
};

async function armChamfer(page: Page): Promise<void> {
  await openEditorDebug(page);
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  // The armed cluster now carries the ✓/✕ pair (it used to be a bare mm input,
  // which was the only affordance — and it committed on release instead).
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
}

test("a released chamfer drag stays ARMED and commits nothing", async ({ page }) => {
  await armChamfer(page);
  const before = await getFeatureLabels(page);

  await dragInCanvas(page, 180, 120);

  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
  await expect(page.getByRole("button", { name: "Chamfer", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(await getFeatureLabels(page)).toEqual(before); // release NEVER commits
});

test("the visible ✓ commits the armed chamfer", async ({ page }) => {
  await armChamfer(page);
  const before = await getFeatureLabels(page);

  await dragInCanvas(page, 180, 120);
  await page.getByTestId("chip-confirm").click();

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Chamfer");
  // A committed tool hands the pointer back to Select and drops its chip.
  await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByTestId("chip-confirm")).toHaveCount(0);
});

test("Enter commits the armed chamfer; ✕ cancels it with no row", async ({ page }) => {
  await armChamfer(page);
  const before = await getFeatureLabels(page);

  await page.getByTestId("chip-cancel").click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("idle");
  expect(await getFeatureLabels(page)).toEqual(before); // ✕ writes nothing

  // Re-arm and finish with the keyboard instead.
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await page.keyboard.press("Enter");

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Chamfer");
});
