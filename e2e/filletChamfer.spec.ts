import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  bodyOptions,
  cancelOperation,
  confirmControl,
  confirmOperation,
  getFeatureLabels,
  openEditorDebug,
} from "./helpers";
import {
  seedSelection,
  toolPhases,
  dragEdgeOpHandle,
  openFilletOverflow,
  closeFilletOverflow,
} from "./modelToolHelpers";

/*
 * FILLET-CHAMFER-UNIFY W2 — the `chamfer` TOOL ID and its `H` binding are dead;
 * one "Fillet / Chamfer" tool remains (id/icon/shortcut stay `fillet`/`fillet`/
 * `F`), and the explicit chip segment decides which op gets authored — a drag
 * sizes it and never re-types it (D04 type lock). `H` now falls through to the
 * GLOBAL `home` binding.
 *
 * Folds the two specs this wave deletes:
 *   - `chamfer.spec.ts` (the armed-commit gesture: released-stays-armed, ✓
 *     commits, Enter/✕) — armed here via the chip segment rather than a
 *     dedicated tool, since that tool id no longer exists.
 *   - `fillet-direction.spec.ts` (the bisector-tier direction drive + segment
 *     lock) — since rewritten as the D04 type-lock cases.
 * Plus the W2 sweep cases: no separate Chamfer button, and `h` is inert as an
 * edge-op trigger.
 *
 * The seeded mock document publishes the box body's MESH1 through the ingest
 * path, so `body1`'s edges resolve on the bisector tier here — which is what
 * lets the type-lock cases below drive the real gesture rather than only the
 * chip segments.
 */

/** `e:5` of the mock box (80×60×30, origin-centred): the +x/+z top edge. Its
 *  midpoint and bisector are fixed by the mesh, so both are written out here. */
const EDGE_MID: [number, number, number] = [40, 0, 15];
const OUTWARD: [number, number, number] = [Math.SQRT1_2, 0, Math.SQRT1_2];

const EDGE_REF = {
  kind: "edge",
  id: "body1#e:5",
  bodyId: "body1",
  topoKey: "e:5",
  // NO `elementId`: a seeded ref stands in for a FRESH pick, and promotion is
  // fire-and-forget — the id lands a round-trip later. Seeding one the backend
  // never minted would be a fiction the edge-op lane now acts on, because a
  // promoted pick is addressed BY that id (WP-U4 / D-5).
  // Anchors the chip (and the real 3D handle it's now leader-lined off of, since
  // this edge resolves the bisector tier — UNIFY-UX Phase 1) ON the edge, not at
  // the world origin, which real MESH1 tier math needs to be honest anyway.
  anchor: { worldPoint: EDGE_MID },
};

async function armFillet(page: Page): Promise<void> {
  await openEditorDebug(page);
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
}

/** Arm Fillet, then lock Chamfer via the chip segment — the unified-tool
 *  replacement for the old dedicated `chamfer` tool id / arm helper. */
async function armChamferViaSegment(page: Page): Promise<void> {
  await armFillet(page);
  await openFilletOverflow(page);
  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpKind).toBe("Chamfer");
  await closeFilletOverflow(page); // the popover can float over the handle
}

/** The screen direction one world step of "away from the body" points, via the
 *  engine's own `projectPoint` (exposed by `?vpdebug`). */
async function awayOnScreen(page: Page): Promise<{ x: number; y: number }> {
  const d = await page.evaluate(
    ([mid, out]) => {
      const engine = (
        window as unknown as {
          __vpEngine?: { projectPoint(w: number[]): { x: number; y: number } | null };
        }
      ).__vpEngine;
      if (!engine) throw new Error("__vpEngine unavailable (?vpdebug only)");
      const a = engine.projectPoint(mid);
      const b = engine.projectPoint([mid[0] + 10 * out[0], mid[1] + 10 * out[1], mid[2] + 10 * out[2]]);
      if (!a || !b) throw new Error("edge midpoint does not project");
      return { x: b.x - a.x, y: b.y - a.y };
    },
    [EDGE_MID, OUTWARD] as const,
  );
  const len = Math.hypot(d.x, d.y);
  expect(len).toBeGreaterThan(1); // a head-on edge would make this test meaningless
  return { x: d.x / len, y: d.y / len };
}

/** The armed edge-op cluster's primary field: "Radius (mm)" for a Fillet,
 *  "Distance (mm)" for a Chamfer (WP-U10) — matched by either name so a spec
 *  that flips the op type mid-flow does not need its own locator per state. */
function primaryField(page: Page) {
  return page.getByLabel(/^(Radius|Distance) \(mm\)$/);
}

/** The armed chip's current value (the only readout of the dragged size). */
async function chipValue(page: Page): Promise<number> {
  return Number.parseFloat(await primaryField(page).inputValue());
}

/** The projection's last feature row (`__stores.document`, dev-only). */
async function lastFeature(
  page: Page,
): Promise<{ id: string; label: string; valueText: string }> {
  const f = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        document: {
          getState(): { features: Array<{ id: string; label: string; valueText: string }> };
        };
      };
    };
    const all = w.__stores?.document.getState().features ?? [];
    return all[all.length - 1];
  });
  if (!f) throw new Error("the projection has no features");
  return f;
}

/** The armed chamfer's second-distance field (`=` when equal-leg). */
function d2Field(page: Page) {
  return page.getByLabel("Second distance");
}

/** `e:1` of the same box: the +x BOTTOM edge, untouched by a chamfer on `e:5`. */
const SURVIVING_EDGE_REF = {
  kind: "edge",
  id: "body1#e:1",
  bodyId: "body1",
  topoKey: "e:1",
  anchor: { worldPoint: [40, 0, -15] },
};

/** The one user-facing wording for a refused promotion (`src/ipc/promote.ts`). */
const STALE_PICK_HINT = "Selection is out of date — pick again";

/** The ids currently in `selectionStore` (dev-only `__stores`). */
async function selectedRefIds(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { selection: { getState(): { selected: Array<{ id: string }> } } };
    };
    return (w.__stores?.selection.getState().selected ?? []).map((r) => r.id);
  });
}

/** Open the timeline and double-click the row `id` (the parametric re-edit entry).
 *  A re-edit has no picks of its own — `filletOutward` never resolves, so it stays
 *  DEGRADED (no 3D handle) — but its [Fillet|Chamfer] segments still live behind
 *  the overflow, so every caller needs it open regardless. */
async function reopenRow(page: Page, id: string): Promise<void> {
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  const row = page.getByTestId(`history-row-${id}`);
  await expect(row).toBeVisible();
  await row.dblclick();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await openFilletOverflow(page);
}

// ── the unified tool + the dead `chamfer` id / `H` binding ───────────────────

test("the toolbar shows ONE Fillet / Chamfer button, no separate Chamfer button", async ({ page }) => {
  await openEditorDebug(page);
  await expect(page.getByRole("button", { name: "Fillet / Chamfer", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Chamfer", exact: true })).toHaveCount(0);
});

test("`h` in model mode does not arm an edge op (H is Home now, not Chamfer)", async ({ page }) => {
  await openEditorDebug(page);
  // Touch a real tool switch once first so the debug surface actually exists —
  // `window.__extrudePreview` is first published by `onToolChange`'s own cancel
  // sweep (`cancelFillet` calls `updateDebug` unconditionally), and a fresh boot
  // has never fired one, so polling for "idle" straight off `h` would otherwise
  // hang against `undefined` forever rather than genuinely observe idle.
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("idle");

  await page.keyboard.press("h");
  // No edge op armed — the minimum bar for a key that used to be a tool trigger
  // and is now a plain (engine-side, storeless) camera action.
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("idle");
  await expect(page.getByRole("button", { name: "Fillet / Chamfer", exact: true })).not.toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("F arms the unified tool with an edge seeded; an empty selection hints for Fillet", async ({ page }) => {
  await openEditorDebug(page);
  // No edge selected → the tool arms nothing and names the CURRENT (default) type,
  // exactly as before unification (armEdgeOpFromSelection's empty-selection guard).
  await page.keyboard.press("f");
  // C7 named the second entry point in this copy: a face arms the tool too.
  await expect(page.getByText("Select edges or a face, then Fillet")).toBeVisible();

  await page.keyboard.press("Escape");
  await seedSelection(page, [EDGE_REF]);
  await page.keyboard.press("f");
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await expect(page.getByRole("button", { name: "Fillet / Chamfer", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

// ── type lock: a drag never re-types the edge op ─────────────────────────────

// D04 type lock (spec), TODO.md SESSION 37: dragging sizes the op; only the segment re-types it.
test("a Fillet arm is type-locked: dragging INTO the body keeps Fillet and bottoms at the minimum, dragging back out grows it", async ({
  page,
}) => {
  await armFillet(page);

  const armed = await toolPhases(page);
  expect(armed?.edgeOpAxisSource).toBe("bisector"); // a real world-axis handle to drag
  expect(armed?.edgeOpAuto).toBe(false);
  expect(armed?.edgeOpKind).toBe("Fillet");

  const away = await awayOnScreen(page);
  await dragEdgeOpHandle(page, -away.x * 200, -away.y * 200); // hard INTO the body
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  expect((await toolPhases(page))?.edgeOpKind).toBe("Fillet"); // no re-type
  // The locked type projects the drag onto its own half-line: bottomed at the
  // minimum rather than bouncing back up as a magnitude would.
  await expect.poll(async () => await chipValue(page)).toBeCloseTo(0.1, 5);

  await dragEdgeOpHandle(page, away.x * 200, away.y * 200);
  await expect.poll(async () => await chipValue(page)).toBeGreaterThan(1); // …and it recovers
  expect((await toolPhases(page))?.edgeOpKind).toBe("Fillet");
});

// D04 type lock (spec), TODO.md SESSION 37: the explicit segment is the only type switch.
test("the explicit segment is the only type switch: a picked Chamfer survives drags either way", async ({
  page,
}) => {
  await armFillet(page);
  expect((await toolPhases(page))?.edgeOpAuto).toBe(false);

  await openFilletOverflow(page);
  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpKind).toBe("Chamfer");
  await closeFilletOverflow(page); // the popover can float over the handle

  // Which way a Chamfer grows is not this test's business — only that neither
  // direction re-types it and the size stays a positive magnitude.
  const away = await awayOnScreen(page);
  for (const sign of [-1, 1]) {
    await dragEdgeOpHandle(page, sign * away.x * 140, sign * away.y * 140);
    await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
    expect((await toolPhases(page))?.edgeOpKind).toBe("Chamfer");
    expect(await chipValue(page)).toBeGreaterThan(0);
  }
  await openFilletOverflow(page);
  await expect(page.getByTestId("chip-edgeop-chamfer")).toHaveAttribute("aria-pressed", "true");
});

// ── armed-commit gesture (ported from chamfer.spec.ts, armed via segment) ───

test("a released drag stays ARMED and commits nothing", async ({ page }) => {
  await armChamferViaSegment(page);
  const before = await getFeatureLabels(page);

  await dragEdgeOpHandle(page, 0, -40);

  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await expect(confirmControl(page)).toBeVisible();
  await expect(page.getByRole("button", { name: "Fillet / Chamfer", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(await getFeatureLabels(page)).toEqual(before); // release NEVER commits
});

test("the visible ✓ commits Chamfer after a flip", async ({ page }) => {
  await armChamferViaSegment(page);
  const before = await getFeatureLabels(page);

  await dragEdgeOpHandle(page, 0, -40);
  await confirmOperation(page);

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Chamfer");
  // A committed tool hands the pointer back to Select and drops its chip.
  await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(confirmControl(page)).toHaveCount(0);
});

test("the visible ✓ commits Fillet with no flip", async ({ page }) => {
  await armFillet(page);
  const before = await getFeatureLabels(page);

  await confirmOperation(page);

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Fillet");
});

// ── the commit clears what it consumed (WP-U4 / D-5) ────────────────────────

test("a committed chamfer drops the edge it ate; the next arm carries no stale hint", async ({
  page,
}) => {
  // The reviewer's "displaced highlight after a chamfer": the consumed edge's ref
  // used to survive the commit and be re-pointed at whatever sat nearest it in the
  // new mesh. Now the commit clears its own inputs, and nothing guesses.
  await armChamferViaSegment(page);
  const before = await getFeatureLabels(page);

  await dragEdgeOpHandle(page, 0, -40);
  await confirmOperation(page);
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Chamfer");

  await expect.poll(async () => await selectedRefIds(page)).toEqual([]);
  await expect(page.getByText(STALE_PICK_HINT)).toHaveCount(0);

  // …and a SURVIVING edge arms the next op cleanly, with no residue from the last.
  await seedSelection(page, [SURVIVING_EDGE_REF]);
  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await expect(page.getByText(STALE_PICK_HINT)).toHaveCount(0);
});

test("the commit's REGEN drops a bystander pick, and ⌘Z/⇧⌘Z leaves no stale hint", async ({
  page,
}) => {
  // Two different mechanisms, one publish. The chamfer's own inputs go through
  // `clearConsumedSelection`; a ref that was merely SELECTED when the regen
  // landed is judged by the mesh swap instead (`viewport/mesh/rebindPick`).
  //
  // `e:1` is untouched by a chamfer on `e:5` and its ordinal still resolves in
  // the new table — element counts only grow across these regens — which is
  // exactly why keeping it on that evidence would point the next op at whatever
  // inherited the ordinal. Unpromoted, it has no authority across a regen, so it
  // goes, silently.
  await armChamferViaSegment(page);
  const before = await getFeatureLabels(page);

  // A second pick, made while the op is armed: the closure is already adopted,
  // so this only changes what the selection holds when the publish lands.
  await seedSelection(page, [EDGE_REF, SURVIVING_EDGE_REF]);
  await expect.poll(async () => await selectedRefIds(page)).toEqual(["body1#e:5", "body1#e:1"]);

  await dragEdgeOpHandle(page, 0, -40);
  await confirmOperation(page);
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);

  await expect.poll(async () => await selectedRefIds(page)).toEqual([]);
  await expect(page.getByText(STALE_PICK_HINT)).toHaveCount(0);

  // ⌘Z restores the edge the chamfer ate, the user picks it again, and ⇧⌘Z eats
  // it a second time. Nothing anywhere says the pick is out of date.
  await page.keyboard.press("Meta+z");
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length);
  await seedSelection(page, [EDGE_REF]);
  await page.keyboard.press("Meta+Shift+z");
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  await expect(page.getByText(STALE_PICK_HINT)).toHaveCount(0);
});

test("Enter commits the armed op; ✕ cancels it with no row", async ({ page }) => {
  await armChamferViaSegment(page);
  const before = await getFeatureLabels(page);

  await cancelOperation(page);
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("idle");
  expect(await getFeatureLabels(page)).toEqual(before); // ✕ writes nothing

  // Re-arm (plain Fillet this time) and finish with the keyboard instead.
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await page.keyboard.press("Enter");

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Fillet");
});

// ── re-edit TYPE flip (W3) ──────────────────────────────────────────────────

test("a committed Fillet row re-edits its TYPE: dblclick → Chamfer segment → Enter swaps the SAME row, ⌘Z restores", async ({
  page,
}) => {
  // (1) Commit a real Fillet — the seeded mock rows carry no stored params, so a
  // re-edit has to run against a feature this spec authored.
  await armFillet(page);
  await confirmOperation(page);
  await expect.poll(async () => (await getFeatureLabels(page)).at(-1)).toBe("Fillet");
  const rowCount = (await getFeatureLabels(page)).length;
  const { id } = await lastFeature(page);

  // (2) Reach the FULL timeline: the body state shows only the first rows, the
  // feature state shows every one (InspectorPanel BodyState vs FeatureState).
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  const row = page.getByTestId(`history-row-${id}`);
  await expect(row).toBeVisible();

  // (3) Double-click re-opens it armed on its COMMITTED type, with the segments.
  await row.dblclick();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await openFilletOverflow(page);
  await expect(page.getByTestId("chip-edgeop-fillet")).toHaveAttribute("aria-pressed", "true");
  // The re-edit opens the committed type with the drag lane OFF — only an explicit
  // segment may re-type a committed op.
  expect((await toolPhases(page))?.edgeOpAuto).toBe(false);

  // (4) Flip + Enter. The size is untouched, so this commits a PURE opType swap.
  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpKind).toBe("Chamfer");
  await primaryField(page).click();
  await page.keyboard.press("Enter");

  // (5) The SAME row swapped — no new feature was authored.
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Chamfer");
  expect((await getFeatureLabels(page)).length).toBe(rowCount);
  expect((await lastFeature(page)).id).toBe(id);

  // (6) …and the swap is a normal undoable edit.
  await page.keyboard.press("Meta+z");
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Fillet");
  expect((await getFeatureLabels(page)).length).toBe(rowCount);
});

// ── two-distance chamfer (SCHEMA §7.3, 2026-08-03 — WP-C T2a) ────────────────

test("a Chamfer arm gains a second-distance field; a Fillet arm does not", async ({ page }) => {
  await armFillet(page);
  await openFilletOverflow(page);
  // SCHEMA §7.3 forbids a Fillet from carrying `distance2`, so the field that
  // authors it must not even be reachable there.
  await expect(d2Field(page)).toHaveCount(0);

  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect(d2Field(page)).toHaveValue("="); // equal-leg, not a blank
  expect((await toolPhases(page))?.edgeOpDistance2).toBeNull();

  // …and flipping back hides it again.
  await page.getByTestId("chip-edgeop-fillet").click();
  await expect(d2Field(page)).toHaveCount(0);
});

test("arm Chamfer → type a second distance → commit: the row reads `d1×d2`", async ({ page }) => {
  await armChamferViaSegment(page);
  await openFilletOverflow(page);
  const before = await getFeatureLabels(page);

  await d2Field(page).fill("2.5");
  // Enter in the second-distance field applies the value THEN confirms the op —
  // the same single-fire contract the first distance's input has.
  await d2Field(page).press("Enter");

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  const row = await lastFeature(page);
  expect(row.label).toBe("Chamfer");
  // Mirrors Rust `dto.rs feature_value_text` (pinned there); the LEADING number
  // is what a re-edit seeds d1 from.
  expect(row.valueText).toBe("1.0×2.5 mm");
});

test("a two-distance chamfer BLOCKS the type flip, and allows it once d2 is cleared", async ({
  page,
}) => {
  // (1) Commit a two-distance chamfer this spec owns (the seeded mock rows carry
  // no stored params, so a re-edit has to run against one authored here).
  await armChamferViaSegment(page);
  await openFilletOverflow(page);
  await d2Field(page).fill("2.5");
  await d2Field(page).press("Enter");
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Chamfer");
  const { id } = await lastFeature(page);
  const rowCount = (await getFeatureLabels(page)).length;

  // (2) Re-open it: BOTH legs are seeded from the stored params.
  await reopenRow(page, id);
  await expect(page.getByTestId("chip-edgeop-chamfer")).toHaveAttribute("aria-pressed", "true");
  await expect(d2Field(page)).toHaveValue("2.5");
  expect((await toolPhases(page))?.edgeOpDistance2).toBe(2.5);

  // (3) Flipping to Fillet would DROP the second leg, so the backend refuses the
  // edit with the standard allow-list reason — and the hint names the field.
  await page.getByTestId("chip-edgeop-fillet").click();
  await primaryField(page).click();
  await page.keyboard.press("Enter");
  // SCOPED to the status hint: the refusal is now stated in TWO places — here,
  // and in the inspector's own `Failed: …` line, which an armed re-edit reaches
  // since the completed-attempt recap stopped shadowing it. Both carry the same
  // sentence, so an unscoped `getByText` is a strict-mode violation.
  await expect(page.getByTestId("status-hint")).toHaveText(/clear distance2 first/);
  // The rejected flip wrote nothing: the row is still the same Chamfer.
  expect((await lastFeature(page)).label).toBe("Chamfer");
  expect((await lastFeature(page)).valueText).toBe("1.0×2.5 mm");
  expect((await getFeatureLabels(page)).length).toBe(rowCount);

  // (4) Clear the second leg back to `=` — an ordinary params edit.
  await reopenRow(page, id);
  await d2Field(page).fill("=");
  await d2Field(page).press("Enter");
  await expect.poll(async () => (await lastFeature(page)).valueText).toBe("1.0 mm");
  expect((await lastFeature(page)).label).toBe("Chamfer");

  // (5) …and NOW the plain W3 swap goes through on the SAME row.
  await reopenRow(page, id);
  await expect(d2Field(page)).toHaveValue("=");
  await page.getByTestId("chip-edgeop-fillet").click();
  await primaryField(page).click();
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Fillet");
  expect((await lastFeature(page)).id).toBe(id);
  expect((await getFeatureLabels(page)).length).toBe(rowCount);
});

// ── arming from a FACE selection (C7 / D9) ──────────────────────────────────

/*
 * The review's second session found Fillet unreachable: the edges could not be
 * clicked (T8) AND the tool was disabled with a face selected (A-265) or a whole
 * body selected (A-289). Picking is fixed in `Picker.ts` (`edge-pick.spec.ts`);
 * this is the other entry point — a selected face arms the tool over that face's
 * boundary edges, which is what most CAD does.
 */
const FACE_REF = {
  kind: "face",
  id: "body1#f:0",
  bodyId: "body1",
  topoKey: "f:0",
  // The +X face's centre. No `elementId` for the same reason `EDGE_REF` has
  // none: a seeded ref stands in for a FRESH pick.
  anchor: { worldPoint: [40, 0, 0] as [number, number, number] },
};

test("a FACE selection arms Fillet over that face's four edges, and commits", async ({ page }) => {
  await openEditorDebug(page);
  await seedSelection(page, [FACE_REF]);
  const before = await getFeatureLabels(page);

  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  // The mock box's +X face is bounded by e:1, e:5, e:9 and e:10 — and the hint
  // says WHERE the four came from, so a whole-face arm is not mistaken for the
  // single edge the user thought they clicked.
  await expect(page.getByTestId("status-hint")).toHaveText(/Fillet 4 edges of Face 0/);

  await page.keyboard.press("Enter");
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Fillet");
});

test("a whole-BODY selection leaves Fillet disabled, with a reason", async ({ page }) => {
  await openEditorDebug(page);
  await seedSelection(page, [{ kind: "body", id: "body1" }]);
  const button = page.getByRole("button", { name: "Fillet / Chamfer", exact: true });
  await expect(button).toHaveAttribute("aria-disabled", "true");
  await button.hover();
  await expect(page.getByRole("tooltip")).toHaveText(/Select edges or a face/);
  // "Fillet this body" has no defensible edge set, so the button stays inert.
  await button.click({ force: true });
  await expect(button).toHaveAttribute("aria-pressed", "false");
});
