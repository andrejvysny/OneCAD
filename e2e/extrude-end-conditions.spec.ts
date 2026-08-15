import { test, expect } from "./fixtures";
import {
  hideSeedSketches,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  extrudeDebug,
  getSketchSnapshot,
  planePointToClient,
} from "./helpers";
import { openExtrudeOverflow } from "./modelToolHelpers";

/*
 * MODEL-OPS W1 — the extrude end conditions are reachable from the UI.
 *
 * The worker has implemented Blind / ThroughAll / Symmetric / ToNext / ToFace
 * since W-WP6 (`ExtrudeOp.cpp effective_distance`) and the wire type has always
 * carried `extrudeMode`, but the tool only ever authored Blind and Symmetric —
 * so most of the kernel's extrude was simply unreachable. The exact geometry each
 * condition produces is pinned against the REAL worker in
 * `src-tauri/tests/wire_contract.rs`; this spec pins that the chip can author
 * them and that ToFace's pick phase is escapable.
 *
 * Note: the mock document seeds a body, so the body-reaching segments are ENABLED
 * here. Their zero-body disabled state is a pure render concern and is pinned in
 * `ModelToolChips.test.tsx` instead, where the store can be driven directly.
 */
test("the end-condition segments author ThroughAll and enter/escape the ToFace pick", async ({
  page,
}) => {
  await openEditorDebug(page);
  await hideSeedSketches(page);
  const targetVisibility = bodyOptions(page).first().getByRole("switch");
  await targetVisibility.click();
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -180, -110);
  await clickAt(page, 60, 90);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  const snap = await getSketchSnapshot(page);
  const xs = snap.lines.flatMap((line) => [line.p0[0], line.p1[0]]);
  const ys = snap.lines.flatMap((line) => [line.p0[1], line.p1[1]]);

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  const client = await planePointToClient(page, snap.plane, {
    x: Math.min(...xs) + (Math.max(...xs) - Math.min(...xs)) * 0.1,
    y: Math.min(...ys) + (Math.max(...ys) - Math.min(...ys)) * 0.1,
  });
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
  await targetVisibility.click();
  await expect(targetVisibility).toHaveAttribute("aria-checked", "true");
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();

  // The end conditions live behind the chip's `⋯` now — the collapsed chip
  // carries the dimension alone.
  await openExtrudeOverflow(page);

  // Blind is the default and drives the distance input.
  await expect(page.getByTestId("chip-end-blind")).toHaveAttribute("aria-pressed", "true");
  expect((await extrudeDebug(page))?.endCondition).toBe("Blind");

  await page.getByTestId("chip-end-throughall").click();
  await expect(page.getByTestId("chip-end-throughall")).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(async () => (await extrudeDebug(page))?.endCondition, { timeout: 3_000 })
    .toBe("ThroughAll");

  // ToFace hands pointer ownership to a face pick rather than applying outright;
  // Esc must fall back to Blind, never leave an unreachable ToFace armed (the
  // worker would reject a ToFace with no target at commit).
  await page.getByTestId("chip-end-toface").click();
  await expect
    .poll(async () => (await extrudeDebug(page))?.phase, { timeout: 3_000 })
    .toBe("facePick");

  await page.keyboard.press("Escape");
  await expect.poll(async () => (await extrudeDebug(page))?.phase, { timeout: 3_000 }).toBe("armed");
  expect((await extrudeDebug(page))?.endCondition).toBe("Blind");
  expect((await extrudeDebug(page))?.hasTargetFace).toBe(false);
});
