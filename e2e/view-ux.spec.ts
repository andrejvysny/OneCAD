import { test, expect, type Page } from "@playwright/test";
import { openEditorDebug, waitForCameraSettled, bodyOptions } from "./helpers";

/*
 * W3 view UX, end to end in the mock lane:
 *   1. the display-mode button really changes what the renderer draws,
 *   2. ⇧F frames the SELECTION rather than always the whole scene,
 *   3. ⇧I isolates the selected bodies transiently, and Esc restores them.
 *
 * Everything is asserted against the LIVE scene graph through `?vpdebug`'s
 * `window.__vpEngine` — there is no per-body DOM, and asserting on the store
 * would prove only that a boolean flipped, which is exactly the defect W3 fixes
 * (the display toggle was store-only and the renderer never read it).
 */

/** Per-body child visibility in bodiesRoot: `{ bodyId: [face, edge] }`. */
function childVisibility(page: Page): Promise<Record<string, boolean[]>> {
  return page.evaluate(() => {
    const engine = (
      window as unknown as {
        __vpEngine?: {
          bodiesRoot: {
            children: Array<{
              visible: boolean;
              userData: { bodyId?: string };
              children: Array<{ visible: boolean; userData: { kind?: string } }>;
            }>;
          };
        };
      }
    ).__vpEngine;
    const out: Record<string, boolean[]> = {};
    for (const group of engine?.bodiesRoot.children ?? []) {
      out[String(group.userData.bodyId)] = ["face", "edge"].map(
        (kind) => group.children.find((c) => c.userData.kind === kind)?.visible ?? false,
      );
    }
    return out;
  });
}

/** Whether each body GROUP is visible: `{ bodyId: visible }`. */
function groupVisibility(page: Page): Promise<Record<string, boolean>> {
  return page.evaluate(() => {
    const engine = (
      window as unknown as {
        __vpEngine?: {
          bodiesRoot: { children: Array<{ visible: boolean; userData: { bodyId?: string } }> };
        };
      }
    ).__vpEngine;
    const out: Record<string, boolean> = {};
    for (const group of engine?.bodiesRoot.children ?? []) {
      out[String(group.userData.bodyId)] = group.visible;
    }
    return out;
  });
}

/** Live camera distance (the fit result). */
function cameraDistance(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      (
        window as unknown as { __vpEngine?: { debugSnapshot(): { distance: number } } }
      ).__vpEngine!.debugSnapshot().distance,
  );
}

/**
 * Add a SECOND body to the projection. The mock backend already serves a
 * distinct mesh for `body2` (a cylinder — `meshForBody`), and MeshIngest's
 * document-store subscription lazy-loads a body the first time it is visible, so
 * this drives the real ingestion path. Two bodies of different size is the
 * minimum that can distinguish "framed the selection" from "framed everything".
 */
async function seedSecondBody(page: Page): Promise<void> {
  await page.evaluate(() => {
    (
      window as unknown as {
        __stores: {
          document: {
            getState(): { applyChange(c: { bodies: Record<string, unknown> }): void };
          };
        };
      }
    ).__stores.document.getState().applyChange({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true },
        body2: { id: "body2", name: "Body 2", visible: true },
      },
    });
  });
  await expect.poll(async () => Object.keys(await groupVisibility(page)).length).toBe(2);
}

test("the display-mode button cycles what the renderer actually draws", async ({ page }) => {
  await openEditorDebug(page);
  await expect.poll(async () => Object.keys(await childVisibility(page)).length).toBe(1);

  // Default is shadedEdges — the look the renderer has always produced.
  expect((await childVisibility(page)).body1).toEqual([true, true]);

  const button = page.getByRole("button", { name: /^Display mode/ });
  await expect(button).toHaveAccessibleName("Display mode: Shaded + edges");

  await button.click(); // → wireframe: edges only
  await expect(button).toHaveAccessibleName("Display mode: Wireframe");
  await expect.poll(async () => (await childVisibility(page)).body1).toEqual([false, true]);

  await button.click(); // → shaded: faces only
  await expect(button).toHaveAccessibleName("Display mode: Shaded");
  await expect.poll(async () => (await childVisibility(page)).body1).toEqual([true, false]);

  await button.click(); // → back to shadedEdges
  await expect.poll(async () => (await childVisibility(page)).body1).toEqual([true, true]);
});

test("⇧F frames the selected body, and falls back to the whole scene with none", async ({
  page,
}) => {
  await openEditorDebug(page);
  await seedSecondBody(page);

  // Frame ONLY the cylinder (the smaller of the two bodies).
  await bodyOptions(page).filter({ hasText: "Body 2" }).click();
  await page.keyboard.press("Shift+F");
  await waitForCameraSettled(page);
  const selectionDistance = await cameraDistance(page);

  // Esc clears the selection (no tool armed, no isolation) → ⇧F frames both.
  await page.keyboard.press("Escape");
  await page.keyboard.press("Shift+F");
  await waitForCameraSettled(page);
  const sceneDistance = await cameraDistance(page);

  // The union of box + cylinder is materially larger than the cylinder alone.
  expect(sceneDistance).toBeGreaterThan(selectionDistance * 1.15);
});

test("⇧I isolates the selection transiently; Esc restores it without touching the document", async ({
  page,
}) => {
  await openEditorDebug(page);
  await seedSecondBody(page);

  await bodyOptions(page).filter({ hasText: "Body 1" }).click();
  await page.keyboard.press("Shift+I");

  await expect.poll(() => groupVisibility(page)).toEqual({ body1: true, body2: false });
  await expect(page.getByText("Isolation on — Esc or ⇧I to exit")).toBeVisible();
  // The document is untouched: body2's eye still reports it as visible.
  await expect(
    bodyOptions(page).filter({ hasText: "Body 2" }).getByRole("switch"),
  ).toHaveAttribute("aria-checked", "true");

  await page.keyboard.press("Escape");
  await expect.poll(() => groupVisibility(page)).toEqual({ body1: true, body2: true });
  await expect(page.getByText("Isolation on — Esc or ⇧I to exit")).toHaveCount(0);
  // Esc left isolation BEFORE deselecting — the selection survives that rung.
  await expect(bodyOptions(page).filter({ hasText: "Body 1" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});
