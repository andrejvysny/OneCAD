/*
 * Shared e2e helpers: boot into the editor, enter a sketch via the REAL plane
 * picker, and drive the WebGL canvas with deliberate pointer "clicks".
 *
 * The viewport is a WebGL canvas with no per-entity DOM, so every assertion in
 * the specs goes through store-driven chrome instead: the status-bar DOF pill,
 * the sketch chrome bar ("Editing …" + "Under-constrained · DOF N"), the
 * inspector Constraints list, the model tree, and the toolbar aria-pressed
 * state. This module only provides the interaction primitives; the specs own the
 * assertions.
 *
 * Selectors used (all pre-existing in the app — no test-only attributes added):
 *   - [data-testid="viewport-canvas"]  — ViewportRoot container (engine appends
 *                                        its <canvas> here; fills the same box).
 *   - [data-plane-pick-label]          — PlanePicker hover chip (HTML overlay).
 *   - role=button aria-label/aria-pressed — FloatingToolbar tools.
 *   - status-bar / chrome / inspector text — see specs.
 */
import { type Page, type Locator, expect } from "@playwright/test";

export const CANVAS = '[data-testid="viewport-canvas"]';

export type SketchToolLabel = "Line" | "Rectangle" | "Circle" | "Arc";

/** Start screen → new document → live editor with a ready WebGL engine. */
export async function openEditor(page: Page): Promise<void> {
  await page.goto("/");
  await page.getByRole("button", { name: "New project" }).click();
  // The editor shell (and thus the viewport canvas) only mounts after the mock
  // newDocument() resolves and appStore flips screen → editor.
  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible();
}

/** The viewport container's bounding box (the canvas fills the same rect). */
async function canvasBox(page: Page): Promise<{ x: number; y: number; width: number; height: number }> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("viewport canvas has no bounding box");
  return box;
}

/** Canvas-center-relative offset (px) → absolute page coordinates. */
async function toPage(page: Page, dx: number, dy: number): Promise<{ x: number; y: number }> {
  const box = await canvasBox(page);
  return { x: box.x + box.width / 2 + dx, y: box.y + box.height / 2 + dy };
}

/**
 * A deliberate click at a canvas-center-relative offset: move → down → up with
 * no intervening drag, so the SketchController classifies it as a tap (its
 * wasClick gate rejects moves > 4px). Offsets stay within the central drawing
 * zone that is clear of the floating tree/inspector/toolbar panels.
 */
export async function clickAt(page: Page, dx: number, dy: number): Promise<void> {
  const p = await toPage(page, dx, dy);
  await page.mouse.move(p.x, p.y);
  await page.mouse.down();
  await page.mouse.up();
}

/** Select a sketch drawing tool from the floating toolbar and confirm it armed. */
export async function selectSketchTool(page: Page, label: SketchToolLabel): Promise<void> {
  const btn = page.getByRole("button", { name: label, exact: true });
  await btn.click();
  await expect(btn).toHaveAttribute("aria-pressed", "true");
}

/** The status-bar DOF pill (e.g. "DOF: 3"). Sketch mode always shows it. */
export function dofPill(page: Page): Locator {
  // Only the status-bar pill starts with "DOF:"; the chrome/inspector use "DOF N".
  return page.getByText(/^DOF: \d/);
}

/** Sketch rows in the model tree (the mock seeds three; entering adds one). */
export function sketchOptions(page: Page): Locator {
  return page.getByRole("listbox", { name: "Sketches" }).getByRole("option");
}

/**
 * Click, then wait for the DOF pill to change — a settle gate for the line tool.
 * Each committed segment round-trips through the ASYNC mock sketchUpsert, and
 * successive commits read the live session at start; firing the next click
 * before the previous commit lands would let them clobber each other (and skip
 * the shared-endpoint coincidence). Adding a segment always shifts DOF by ≥1, so
 * this polls deterministically with no fixed sleep.
 */
export async function clickAtAwaitingDofChange(page: Page, dx: number, dy: number): Promise<void> {
  const pill = dofPill(page);
  const before = (await pill.textContent()) ?? "";
  await clickAt(page, dx, dy);
  await expect(pill).not.toHaveText(before);
}

/**
 * Enter a sketch through the real user path: model-mode "New sketch" → plane
 * picker → hover a plane quad (await the overlay chip) → click it.
 *
 * The hover is polled: the SketchController that owns planePickerHover is created
 * only once the engine finishes initialising, so the first pointermove may land
 * before the listener (or the picker) exists. toPass() re-hovers until the chip
 * appears, which doubles as the engine-ready gate.
 */
export async function enterSketchViaPlanePicker(page: Page): Promise<void> {
  await page.getByRole("button", { name: "New sketch", exact: true }).click();
  // Plane-pick phase chrome (activeSketchId still null).
  await expect(page.getByText("Select a sketch plane")).toBeVisible();

  const center = await toPage(page, 0, 0);
  const chip = page.locator("[data-plane-pick-label]");
  await expect(async () => {
    // Two moves guarantee a fresh pointermove delta reaches the controller.
    await page.mouse.move(center.x, center.y - 1);
    await page.mouse.move(center.x, center.y);
    expect(await chip.isVisible()).toBe(true);
  }).toPass({ timeout: 20_000, intervals: [150, 250, 400, 600] });

  // Click the highlighted quad → creates the sketch on that plane.
  await page.mouse.down();
  await page.mouse.up();

  // Session is live: chrome swaps to "Editing …" and Line is the default tool.
  await expect(page.getByText(/^Editing /)).toBeVisible();
  await expect(page.getByRole("button", { name: "Line", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
}
