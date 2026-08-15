import { test, expect } from "./fixtures";

/*
 * COMPONENT-LIBRARY WP-B2/B5 — "Save as Component" through the real UI chain
 * on the MOCK lane: a body row's context menu → the contributed item → the
 * dialog → the library panel listing what was saved.
 *
 * WHAT THIS OWNS, and what it deliberately does not: the wiring. The menu item
 * is registered by `onecad.library` onto a row `onecad.modeling` provides
 * (`platform.menus`, WP-B1), the dialog is a `Slots.ShellOverlay` occupant, and
 * the panel reads the catalog back — four pieces from three modules that only
 * meet at runtime. The GEOMETRY half (freeze, bake, single-solid refusal, the
 * package that places back as the same solid) is a real-worker concern and is
 * covered there: `src-tauri/tests/component_ops.rs::
 * a_body_saved_as_a_component_places_back_as_the_same_solid`. The mock lane has
 * no worker to bake with, and faking one here would prove nothing.
 *
 * `retries: 0`, matching the rest of this suite.
 */

test.beforeEach(async ({ page }) => {
  // `vpdemo` boots the editor with a real BOX body — the thing being authored.
  // `mocklibrary=1` opts the fixture catalog in so the panel has a baseline.
  await page.goto("/?vpdebug&vpdemo&mocklibrary=1");
  await expect(page.locator('[data-testid="viewport-canvas"] canvas')).toBeVisible();
});

test("a body's context menu offers Save as Component, and the saved component appears in the library", async ({
  page,
}) => {
  // The model tree is the sidebar's default tab; the demo body is its one row.
  const bodyRow = page.getByRole("listbox", { name: "Bodies" }).getByRole("option").first();
  await expect(bodyRow).toBeVisible();
  await bodyRow.click({ button: "right" });

  const item = page.getByText("Save as Component…");
  await expect(item).toBeVisible();
  await item.click();

  const dialog = page.getByTestId("save-as-component-dialog");
  await expect(dialog).toBeVisible();

  // The name seeds from the row and the id follows it — a user who types
  // nothing still gets a valid, namespaced identity.
  const idField = page.getByTestId("save-as-component-id");
  await expect(idField).not.toHaveValue("");

  await page.getByTestId("save-as-component-name").fill("Bracket Plate");
  await page.getByTestId("save-as-component-tags").fill("printed");
  await page.getByTestId("save-as-component-commit").click();

  await expect(dialog).toBeHidden();

  // The Library modal lists it: the catalog read is the same one the placement
  // gesture arms from, so this is the whole round trip the UI can see.
  await page.getByRole("button", { name: "Library" }).click();
  await expect(
    page.getByTestId("library-modal-card").filter({ hasText: "Bracket Plate" }),
  ).toBeVisible();
});

test("an invalid id blocks the save instead of failing at the backend", async ({ page }) => {
  const bodyRow = page.getByRole("listbox", { name: "Bodies" }).getByRole("option").first();
  await bodyRow.click({ button: "right" });
  await page.getByText("Save as Component…").click();

  const id = page.getByTestId("save-as-component-id");
  await id.fill("bracket"); // not namespaced

  await expect(page.getByTestId("save-as-component-id-error")).toBeVisible();
  await expect(page.getByTestId("save-as-component-commit")).toBeDisabled();
  // Still open, still holding what was typed — a refusal here is something the
  // author fixes in place.
  await expect(page.getByTestId("save-as-component-dialog")).toBeVisible();
});
