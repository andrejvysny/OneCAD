import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  dofPill,
  getSketchEntityCount,
  liveDimField,
} from "./helpers";

/*
 * Live dimension chips — Escape/Enter ladder (SP-1 W4).
 *
 * The mock solver is an identity-echo (no real solve); DOF is a coarse count
 * (mockSketch.ts) — not the focus here, which is FOCUS OWNERSHIP: Enter on a
 * focused chip commits the segment WITHOUT finishing the sketch (only a plain
 * Enter over the viewport, with DOM focus elsewhere, ends the chain — see
 * SketchController.onKeyDown's Enter branch vs `LiveDimField`'s own); Escape
 * on a focused chip drops just that field; a second Escape (now unfocused)
 * runs the existing gesture-ending ladder; a third falls through to the
 * global Esc ladder (`useShortcuts.ts` `runCancel`), which switches the active
 * tool to Select — it does NOT leave sketch mode (Select is a sketch tool too).
 */
test("Enter on a chip commits without finishing; Escape unfocuses, then ends the chain, then leaves the tool", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Line");

  await clickAt(page, -150, -90); // anchor — armed with 1 anchor
  await page.keyboard.press("5");
  await expect(liveDimField(page, "length")).toBeFocused();
  await page.keyboard.press("0");

  // ── Enter commits the segment but does NOT finish the sketch ────────────────
  await page.keyboard.press("Enter");
  await expect(dofPill(page)).not.toHaveText("DOF: 0"); // the async commit settled
  expect(await getSketchEntityCount(page)).toBe(1);
  await expect(page.getByText(/^Editing /)).toBeVisible(); // still in sketch mode
  await expect(page.getByRole("button", { name: "Line", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  ); // the chain continues armed on Line, not bounced to Select

  // ── Re-open a chip for the NEXT leg of the chain ────────────────────────────
  // A prior commit does not itself blur the DOM (React only stops calling
  // `.focus()`; nothing calls `.blur()`) — clear any lingering literal focus
  // before typing so the fresh digit reaches `tryStartLiveDimTyping` rather
  // than a stale (already-unfocused-in-the-FSM) input's native onChange.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press("6");
  await expect(liveDimField(page, "length")).toBeFocused();

  // ── First Escape: unfocuses the chip, the gesture (chain) stays alive ───────
  await page.keyboard.press("Escape");
  await expect(liveDimField(page, "length")).toBeVisible(); // chips still open
  expect(await getSketchEntityCount(page)).toBe(1); // nothing new committed

  // ── Second Escape: ends the chain (no field focused ⇒ the gesture ladder) ───
  await page.keyboard.press("Escape");
  await expect(liveDimField(page, "length")).toHaveCount(0); // chips closed
  expect(await getSketchEntityCount(page)).toBe(1); // still just the one segment
  await expect(page.getByRole("button", { name: "Line", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  ); // ending the chain does not itself change tool

  // ── Third Escape: no gesture left ⇒ the global ladder switches tool ─────────
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByRole("button", { name: "Line", exact: true })).not.toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByText(/^Editing /)).toBeVisible(); // sketch mode itself is untouched
});
