import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  dofPill,
  bodyOptions,
  findExtrudeHandle,
  extrudeDebug,
  getSketchSnapshot,
  planePointToClient,
  clickAtClient,
  CANVAS,
} from "./helpers";

/*
 * MODEL-HARDEN Wave 1 — the professional extrude commit gesture (mock lane).
 *
 * The old flow committed on the drag RELEASE. The Wave 1 flow keeps the tool
 * ARMED after release (live preview + editable chip cluster) and commits only on
 * an EXPLICIT gesture: Enter, the chip ✓, or a click away. Esc cancels and never
 * leaves a body behind. Each test uses the production profile-first flow: draw
 * a rectangle → finish → select its filled region → click Extrude.
 */
async function armExtrude(page: import("@playwright/test").Page): Promise<void> {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();
  await enterSketchViaPlanePicker(page);
  // The plane-picker entry animates the camera (CadOrbitControls, 250ms) — settle
  // before any draw click races it (see helpers.ts waitForCameraSettled).
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  const snap = await getSketchSnapshot(page);
  const p0 = snap.lines[0]?.p0;
  const p2 = snap.lines[2]?.p0;
  if (!p0 || !p2) throw new Error("rectangle snapshot is incomplete");
  const centroid = { x: (p0[0] + p2[0]) / 2, y: (p0[1] + p2[1]) / 2 };

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  const client = await planePointToClient(page, snap.plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) => Boolean((window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
          .__vpEngine?.sketchStaticHitTest(x, y)),
        client,
      ),
    )
    .toBe(true);
  await clickAtClient(page, client.x, client.y);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as {
            __stores?: { selection: { getState(): { selected: Array<{ kind: string }> } } };
          }).__stores?.selection.getState().selected[0]?.kind,
      ),
    )
    .toBe("sketchRegion");
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
}

/**
 * Drag the depth handle and release (which now KEEPS the tool armed). The grab must
 * actually land — the handle scan and the real pointerdown are a frame apart, so a
 * miss would leave the tool armed the WHOLE time and make "release keeps armed" pass
 * vacuously (finding 15). Assert the phase became "dragging" mid-drag; a single-frame
 * miss re-scans + re-grabs, a persistent miss fails loudly.
 */
async function dragReleaseHandle(page: import("@playwright/test").Page): Promise<void> {
  await expect(async () => {
    const { x, y } = await findExtrudeHandle(page);
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x, y + 12, { steps: 4 });
    const phase = (await extrudeDebug(page))?.phase;
    if (phase !== "dragging") {
      await page.mouse.up(); // release the missed grab before retrying
      throw new Error(`extrude grab missed — phase was ${String(phase)}`);
    }
  }).toPass({ timeout: 10_000, intervals: [200, 400, 800] });
  await page.mouse.up();
  await expect.poll(async () => (await extrudeDebug(page))?.phase).toBe("armed");
}

test("selected region → Extrude → release stays armed; visible ✓ commits a body", async ({ page }) => {
  await armExtrude(page);
  const bodiesBefore = await bodyOptions(page).count();

  await dragReleaseHandle(page);

  // Release did NOT commit — the tool is still armed (debug surface) with the chip.
  expect((await extrudeDebug(page))?.phase).toBe("armed");
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
  await expect(page.getByRole("button", { name: "Extrude", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore); // nothing committed yet

  // Explicit visible confirm.
  await page.getByTestId("chip-confirm").click();
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect(bodyOptions(page).last()).toHaveAttribute("aria-selected", "true");
});

test("visible ✕ after release cancels the tool and creates no body", async ({ page }) => {
  await armExtrude(page);
  const bodiesBefore = await bodyOptions(page).count();

  await dragReleaseHandle(page);
  expect((await extrudeDebug(page))?.phase).toBe("armed");

  await page.getByTestId("chip-cancel").click();
  // Cancel returns to Select and discards the preview.
  await expect(page.getByRole("button", { name: "Extrude", exact: true })).not.toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore); // no body
});

test("clicking empty canvas away from the handle commits (click-away)", async ({ page }) => {
  await armExtrude(page);
  const bodiesBefore = await bodyOptions(page).count();

  // A true click on empty canvas, off the handle, biased toward centre so it clears
  // the floating side panels (tree left / inspector right / toolbar top).
  const box = await page.locator(CANVAS).boundingBox();
  const h = await findExtrudeHandle(page);
  const cx = (box?.x ?? 0) + (box?.width ?? 0) / 2;
  const tx = h.x + (h.x < cx ? 120 : -120);
  await page.mouse.move(tx, h.y);
  await page.mouse.down();
  await page.mouse.up();

  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
});

test("exact-preview failure keeps the last preview and blocks confirmation", async ({ page }) => {
  await page.goto("/?toolsdemo");
  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible();

  // toolsdemo exposes the production controller/client instances. Wait until its
  // asynchronously seeded region exists, then arm the normal exact-profile draft.
  await expect(async () => {
    await page.evaluate(() => {
      (
        window as unknown as {
          __toolsGate?: { arm(): void };
        }
      ).__toolsGate?.arm();
    });
    expect((await extrudeDebug(page))?.phase).toBe("armed");
  }).toPass({ timeout: 10_000, intervals: [100, 200, 400] });
  await expect.poll(async () => Number((await extrudeDebug(page))?.lastL2Epoch ?? 0)).toBeGreaterThan(0);

  const bodiesBefore = await bodyOptions(page).count();
  await page.evaluate(() => {
    type PreviewFailure = {
      kind: "invalidCommand";
      message: string;
      structural: boolean;
    };
    type PreviewResult = {
      sessionId: string;
      epoch: number;
      bodyId: string;
      error: PreviewFailure;
    };
    const w = window as unknown as {
      __toolsGate?: {
        client: {
          updatePreview(sessionId: string, params: Record<string, unknown>, epoch: number): void;
        };
        controller: {
          sendPreview(depth: number): void;
          onPreviewResult(result: PreviewResult): void;
        };
      };
      __extrudePreview?: { depth?: number };
    };
    const gate = w.__toolsGate;
    if (!gate) throw new Error("tools gate unavailable");
    let sent: { sessionId: string; epoch: number } | null = null;
    gate.client.updatePreview = (sessionId, _params, epoch) => {
      sent = { sessionId, epoch };
    };
    gate.controller.sendPreview(Number(w.__extrudePreview?.depth ?? 10) + 1);
    if (!sent) throw new Error("preview throttle did not dispatch");
    const dispatched = sent as { sessionId: string; epoch: number };
    gate.controller.onPreviewResult({
      sessionId: dispatched.sessionId,
      epoch: dispatched.epoch,
      bodyId: `preview:${dispatched.sessionId}`,
      error: {
        kind: "invalidCommand",
        message: "injected stale region binding",
        structural: true,
      },
    });
  });

  await expect(page.getByText(/Extrude preview failed: injected stale region binding/)).toBeVisible();
  await expect.poll(async () => (await extrudeDebug(page))?.l1Present).toBe(true);
  await expect
    .poll(async () => ((await extrudeDebug(page))?.previewError as { structural?: boolean } | undefined)?.structural)
    .toBe(true);

  await page.getByTestId("chip-confirm").click();
  await expect(page.getByText(/Cannot confirm invalid preview: injected stale region binding/)).toBeVisible();
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore);
  expect((await extrudeDebug(page))?.phase).toBe("armed");
});

test("Enter-commit waits for the final exact preview epoch (commit barrier)", async ({ page }) => {
  await armExtrude(page);
  const bodiesBefore = await bodyOptions(page).count();

  await dragReleaseHandle(page);
  await page.keyboard.press("Enter");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);

  // EXTRUDE-REGION-PARITY: confirm flushes the full final params as the newest
  // preview epoch and the commit BARRIER holds `endPreview(true)` until that
  // exact candidate answers (or times out). The commit result therefore lands on
  // exactly the final epoch — a commit that raced ahead of its own final preview
  // would leave committedEpoch behind finalEpoch.
  const dbg = await extrudeDebug(page);
  const finalEpoch = Number(dbg?.finalEpoch);
  const committedEpoch = Number(dbg?.committedEpoch);
  expect(finalEpoch).toBeGreaterThan(0);
  expect(committedEpoch).toBe(finalEpoch);
});
