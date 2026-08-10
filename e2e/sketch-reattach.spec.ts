import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  waitForCameraSettled,
  findPlaneQuad,
  datumOptions,
  sketchOptions,
  enterSketchViaPlanePicker,
  selectSketchTool,
  clickAt,
  dofPill,
  bodyOptions,
  dragExtrudeDepth,
} from "./helpers";

/*
 * HISTORY-HARDEN H9 — sketch REATTACH V1 (`UpdateSketchAttachment`).
 *
 * A sketch is moved from the world plane it was drawn on to a DATUM plane; the
 * extrude standing on it follows. Both entry points are covered: the model-tree
 * sketch row and the Sketch history row.
 *
 * V1 SCOPE, and why: the target list is world planes + datums only. A host-FACE
 * sketch's frame is derived from the face by the worker and its projected
 * boundary is expressed in that frame (`projectedBoundaryVersion >= 1` for every
 * host-face sketch the app can create — `api::add_sketch_on_face` stamps 1, and
 * SKETCH-ON-FACE W2 closed the only path that could have produced 0), so moving
 * it would silently reinterpret that boundary with nothing to re-project it. The
 * menu item is therefore withheld on a face-hosted sketch.
 *
 * Mock-lane honesty: `mockClient.reattachSketch` really moves the sketch's plane
 * in the local solver lane and RE-SYNTHESIZES every extrude standing on it, so
 * the downstream-follows assertion below is a real geometry change, not a
 * fabricated success.
 */

/** Create one datum on the XY base at the seeded 10 mm offset. */
async function createDatum(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Datum plane", exact: true }).click();
  await waitForCameraSettled(page);
  const quad = await findPlaneQuad(page, "XY");
  await page.mouse.move(quad.x, quad.y);
  await page.mouse.down();
  await page.mouse.up();
  await page.getByTestId("model-tool-chip").getByTestId("chip-confirm").click();
  await expect(datumOptions(page)).toHaveCount(1);
}

/** Document revision (bumped by every reattach that actually moved something). */
async function revision(page: Page): Promise<number> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { revision: number } } };
    };
    return w.__stores?.document.getState().revision ?? -1;
  });
}

/**
 * Draw a rectangle on the world XY plane, finish, and extrude it — the document
 * this spec reattaches. Returns the id of the sketch that was created.
 */
async function buildSketchAndExtrude(page: Page): Promise<string> {
  const before = new Set(
    await page.evaluate(() => {
      const w = window as unknown as {
        __stores?: { document: { getState(): { sketches: Record<string, unknown> } } };
      };
      return Object.keys(w.__stores?.document.getState().sketches ?? {});
    }),
  );

  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -140, -90);
  await clickAt(page, 140, 90);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);

  const created = await page.evaluate((known: string[]) => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { sketches: Record<string, unknown> } } };
    };
    const ids = Object.keys(w.__stores?.document.getState().sketches ?? {});
    return ids.find((id) => !known.includes(id)) ?? ids[ids.length - 1];
  }, [...before]);

  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  // The depth must come from a real handle grab: confirming straight from the
  // armed state leaves it at zero and nothing commits at all (UNIFY-UX,
  // `7d7c82a`).
  await dragExtrudeDepth(page);
  await page.getByTestId("model-tool-chip").getByTestId("chip-confirm").click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toHaveCount(0);

  return created;
}

test.describe("sketch reattach (H9)", () => {
  test("the TREE row moves a world sketch onto a datum and the extrude follows", async ({
    page,
  }) => {
    await openEditorDebug(page);
    // The precondition this test needs is "an extrude committed onto the new
    // sketch", which is what a moved revision proves. It is deliberately NOT a
    // body COUNT: the rectangle overlaps the seeded body, so auto Add/Cut
    // resolves to Cut in either drag direction and the extrude modifies `body1`
    // rather than adding one — correct behaviour, and irrelevant to reattach.
    const revBeforeBuild = await revision(page);
    await buildSketchAndExtrude(page);
    expect(await revision(page)).toBeGreaterThan(revBeforeBuild);
    await createDatum(page);

    // The newest sketch row is the one just drawn.
    const row = sketchOptions(page).last();
    const revBefore = await revision(page);

    await row.click({ button: "right" });
    await expect(page.getByTestId("tree-menu-reattach")).toBeVisible();
    await page.getByTestId("tree-menu-reattach").click();

    // World planes are always offered; the datum shows up by name.
    await expect(page.getByTestId("reattach-world-XY")).toBeVisible();
    await expect(page.getByTestId("reattach-world-XZ")).toBeVisible();
    await expect(page.getByTestId("reattach-world-YZ")).toBeVisible();
    const datumItem = page.locator('[data-testid^="reattach-datum-"]').first();
    await expect(datumItem).toBeVisible();
    await datumItem.click();

    // The reattach published: a hint that names the target, and a moved revision
    // (the mock re-synthesizes every extrude on the sketch — the real backend
    // dirties from the sketch's producing step to the end, `RegenHint::ToEnd`).
    await expect(page.getByText(/Sketch reattached to /)).toBeVisible();
    expect(await revision(page)).toBeGreaterThan(revBefore);
    // …and it is NOT the silent-no-op path.
    await expect(page.getByText(/precede its host/)).toHaveCount(0);
  });

  test("the HISTORY row offers it on a Sketch feature only, and dispatches the same command", async ({
    page,
  }) => {
    await openEditorDebug(page);
    // The SEEDED timeline (f1 Sketch · f2 Extrude · f3 Fillet · f4 Sketch ·
    // f5 Extrude) is the shortest way to a full history with both row kinds;
    // selecting a body reveals the per-row affordances (same as history-suppress).
    // Selecting the body lists the lineage; selecting a ROW switches the
    // inspector to the FEATURE state, which is the one with per-row affordances
    // (same two-step as `history-suppress.spec.ts`).
    await bodyOptions(page).first().click();
    await expect(page.getByTestId("history-row-f3")).toBeVisible();
    await page.getByTestId("history-row-f3").click();
    await expect(page.getByTestId("history-suppress-f3")).toBeVisible();

    // An EXTRUDE row has no attachment to move.
    await page.getByTestId("history-row-f2").click({ button: "right" });
    await expect(page.getByTestId("history-menu-suppress")).toBeVisible();
    await expect(page.getByTestId("history-menu-reattach")).toHaveCount(0);

    // A SKETCH row does. (Right-clicking the next row closes the open popover on
    // its own outside-pointerdown — Escape here would clear the SELECTION too and
    // collapse the inspector back out of the feature state.) Picking a target resolves the row's record id → its
    // sketch id (`getOperationParams().sketchId`) and dispatches the reattach.
    const revBefore = await revision(page);
    await page.getByTestId("history-row-f1").click({ button: "right" });
    await expect(page.getByTestId("history-menu-reattach")).toBeVisible();
    await page.getByTestId("history-menu-reattach").click();
    await page.getByTestId("reattach-world-YZ").click();
    await expect(page.getByText("Sketch reattached to YZ plane")).toBeVisible();
    expect(await revision(page)).toBeGreaterThan(revBefore);
  });

  test("a FACE-HOSTED sketch is not offered a reattach", async ({ page }) => {
    await openEditorDebug(page);
    // Stamp a host face on the seeded sketch row — the projection field
    // (`SketchDto.hostFace`) the menu gates on. A real face-hosted sketch also
    // carries a projected boundary expressed in that face's frame, which is why
    // moving it is out of V1 scope.
    await page.evaluate(() => {
      const w = window as unknown as {
        __stores?: {
          document: {
            getState(): { sketches: Record<string, Record<string, unknown>> };
            setState(p: Record<string, unknown>): void;
          };
        };
      };
      const doc = w.__stores?.document;
      if (!doc) return;
      const sketches = doc.getState().sketches;
      const id = Object.keys(sketches)[0];
      doc.setState({
        sketches: {
          ...sketches,
          [id]: { ...sketches[id], hostFace: { bodyId: "body1", elementId: "el_1" } },
        },
      });
    });

    await sketchOptions(page).first().click({ button: "right" });
    await expect(page.getByTestId("tree-action-onecad.modeling.command.deleteSketch.action")).toBeVisible();
    await expect(page.getByTestId("tree-menu-reattach")).toHaveCount(0);
  });
});
