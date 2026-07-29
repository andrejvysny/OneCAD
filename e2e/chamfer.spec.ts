import { test, expect } from "@playwright/test";
import { openEditorDebug, getFeatureLabels } from "./helpers";

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
