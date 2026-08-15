import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  clickAt,
  dofPill,
  getSketchCircle,
  getSketchEntityCount,
  liveDimField,
} from "./helpers";

/*
 * Live dimension chips — polygon tool (SP-1 W4).
 *
 * The mock solver is an identity-echo (no real solve); DOF is a coarse count
 * (mockSketch.ts). A regular n-gon settles at a FIXED DOF 4 regardless of n
 * (centre 2 + radius 1 + rotation 1 — polygon.spec.ts already pins this via
 * the tool's own Coincident/OnCurve/Equal regularity set), so the typed
 * radius's extra Radius constraint drops it to 3 — asserted directly, no
 * separate baseline draw needed.
 *
 * Idle vs armed digit routing is phase-dependent (SketchController's
 * `tryStartLiveDimTyping` requires ≥1 anchor; the polygon-sides branch is
 * gated to the idle phase once it is): IDLE digits are the side-count verb
 * (polygon.spec.ts already covers re-pinning this); ARMED digits open the
 * radius chip instead (Tab order: radius first — `polygonFrame`'s fields are
 * `[radius, sides]`, since the side count is a machine verb the digit ladder
 * never reaches via typing).
 */
test("idle digits set sides, armed digits type the radius chip, sides stays put", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await page.keyboard.press("g"); // arm Polygon
  await expect(page.getByRole("button", { name: "Polygon", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByText("Polygon — 6 sides · 3–9 to change")).toBeVisible();

  // ── Idle digit: sides, not a live-dim chip (re-pins polygon.spec.ts) ────────
  await page.keyboard.press("5");
  await expect(page.getByText("Polygon — 5 sides · 3–9 to change")).toBeVisible();
  await expect(liveDimField(page, "radius")).toHaveCount(0); // no chip while idle

  // ── Center click arms the gesture; hint + chips switch to "type the radius" ─
  await clickAt(page, 0, 0);
  await expect(page.getByText("Polygon — 5 sides · type to set radius · Tab for sides")).toBeVisible();
  await expect(liveDimField(page, "radius")).toBeVisible();

  // Armed digits open the RADIUS chip (Tab order: radius first) — sides is
  // untouched by typing once armed.
  await page.keyboard.press("3");
  await expect(liveDimField(page, "radius")).toBeFocused();
  await page.keyboard.press("0");
  await expect(page.getByText("Polygon — 5 sides · type to set radius · Tab for sides")).toBeVisible();

  await page.keyboard.press("Enter"); // locks radius=30mm AND commits the n-gon
  await expect(dofPill(page)).not.toHaveText("DOF: 0");

  // 5 ring lines + 1 construction circumcircle (polygon.spec.ts convention).
  expect(await getSketchEntityCount(page)).toBe(6);
  const circle = await getSketchCircle(page);
  expect(circle.radius).toBeCloseTo(30, 3);

  await expect(page.getByText("Radius", { exact: true })).toHaveCount(1);
  // 4 (regular n-gon) − 1 (Radius) − 2 (the centre click snapped to the origin,
  // now persisted as a `Fixed` — see the note in polygon.spec.ts).
  await expect(dofPill(page)).toHaveText("DOF: 1");
});
