import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, extrudeDebug, getFeatureLabels, bodyOptions } from "./helpers";
import { seedSelection, findGizmoHandle, dragFromTo } from "./modelToolHelpers";

/*
 * WP-B W1 + W2 — TransformBody: the numeric placement UX, and the drag gizmo.
 *
 * This spec is unusually geometry-heavy for the mock lane, and deliberately so.
 * Every other mock body op is a stand-in (no CSG, no hollowing), so its specs can
 * only assert history rows and gesture state. A rigid placement is the ONE op
 * where the mock is exact — the same matrix moves the same points the kernel
 * would — so "the body moved by 30" is a real, checkable claim and is checked
 * against the SCENE, not a store.
 *
 * The other thing pinned here is the FOLD rule (SCHEMA §7.3): a placement is one
 * cumulative record per intent. Moving a body twice must leave ONE history row
 * whose params were re-edited — never a stack of nudges. A regression there would
 * look completely normal on screen and only show up as history rot, which is
 * exactly why the row COUNT is asserted at every step.
 *
 * The W2 gizmo cases at the bottom drag REAL handles found through the engine's
 * own raycast, then assert the commit against the SCENE. They deliberately never
 * hard-code a screen→world mapping: the camera decides which way +X runs on
 * screen, so each case reads the dragged placement off the debug surface and
 * asserts the committed geometry matches THAT — which is the actual claim (the
 * ghost, the chip and the committed body all agree), and is invariant to the
 * camera in a way a pixel-delta expectation would not be.
 */

/** The seeded mock box (80×60×30 at the origin) as published by ?vpdemo. */
const BODY = "body1";

/**
 * A rendered body's world extents, read off the SCENE graph.
 *
 * Deliberately not the projection store: a placement's whole job is to move
 * geometry, and the store carries no coordinates at all. MESH1 vertex buffers are
 * uploaded verbatim into an un-transformed `bodiesRoot` (the engine's Z-up
 * invariant), so a geometry-space bbox IS the world bbox here.
 */
async function bodyBounds(
  page: Page,
  bodyId: string,
): Promise<{ min: number[]; max: number[] } | null> {
  return page.evaluate((id) => {
    const engine = (
      window as unknown as {
        __vpEngine?: {
          bodiesRoot: {
            children: Array<{
              userData: { bodyId?: string };
              traverse(fn: (o: Record<string, unknown>) => void): void;
            }>;
          };
        };
      }
    ).__vpEngine;
    const node = engine?.bodiesRoot.children.find((c) => String(c.userData.bodyId ?? "") === id);
    if (!node) return null;
    const min = [Infinity, Infinity, Infinity];
    const max = [-Infinity, -Infinity, -Infinity];
    node.traverse((o) => {
      if (!o.isMesh) return;
      const g = o.geometry as
        | { attributes?: { position?: { array: ArrayLike<number>; count: number } } }
        | undefined;
      const pos = g?.attributes?.position;
      if (!pos) return;
      for (let i = 0; i < pos.count; i++) {
        for (let a = 0; a < 3; a++) {
          const v = pos.array[i * 3 + a];
          if (v < min[a]) min[a] = v;
          if (v > max[a]) max[a] = v;
        }
      }
    });
    return Number.isFinite(min[0]) ? { min, max } : null;
  }, bodyId);
}

/**
 * Wait for a body's geometry to reach the scene and return its extents. Mesh
 * ingestion is a pull (`document-changed` → `getBodyMesh` → registry → scene), so
 * a bbox read the instant the editor opens is legitimately null.
 */
async function waitForBody(page: Page, bodyId: string): Promise<{ min: number[]; max: number[] }> {
  await expect.poll(async () => (await bodyBounds(page, bodyId)) !== null).toBe(true);
  return (await bodyBounds(page, bodyId))!;
}

/** Poll the body's min-X until the scene has ingested the committed mesh. */
async function expectMinX(page: Page, bodyId: string, want: number): Promise<void> {
  await expect
    .poll(async () => {
      const b = await bodyBounds(page, bodyId);
      return b ? Math.round(b.min[0]) : null;
    })
    .toBe(want);
}

const chip = (page: Page) => page.getByTestId("model-tool-chip");
const chipInput = (page: Page) => chip(page).getByLabel("Dimension value");

/** Select bodies and arm the placement tool with the `t` shortcut. */
async function armTransform(page: Page, ...bodyIds: string[]): Promise<void> {
  await seedSelection(
    page,
    bodyIds.map((id) => ({ kind: "body", id })),
  );
  await page.keyboard.press("t");
  await expect.poll(async () => (await extrudeDebug(page))?.transformPhase).toBe("armed");
  await expect(chip(page)).toBeVisible();
}

/** Type a value into the placement chip and commit it with the visible ✓. */
async function typeAndConfirm(page: Page, value: string): Promise<void> {
  await chipInput(page).fill(value);
  await chipInput(page).blur();
  await chip(page).getByTestId("chip-confirm").click();
  await expect(page.getByTestId("model-tool-chip")).toHaveCount(0);
}

/**
 * An axis segment. Testid-keyed on purpose: the chips are portaled under the
 * viewport's `aria-hidden` overlay root, so `getByRole` cannot see them at all.
 */
const axisButton = (page: Page, axis: "X" | "Y" | "Z") =>
  chip(page).getByTestId(`chip-axis-${axis.toLowerCase()}`);

/**
 * Switch the inspector to its FULL timeline. A body selection renders only the
 * first three lineage rows (InspectorPanel `SelectionState`), and a commit leaves
 * the placed bodies selected — so the freshly appended Move row is off-list until
 * a click on any history row flips the panel into the feature state.
 */
async function openFullTimeline(page: Page): Promise<void> {
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  await expect(page.getByTestId("history-suppress-f3")).toBeVisible();
}

const moveRows = async (page: Page): Promise<string[]> =>
  (await getFeatureLabels(page)).filter((l) => l === "Move");

test("t arms the placement cluster on a selected body", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await armTransform(page, BODY);

  // The cluster opens on Move · X · 0 — the whole W1 surface, no gizmo.
  await expect(chip(page).getByTestId("chip-transform-move")).toHaveAttribute("aria-pressed", "true");
  await expect(chip(page).getByTestId("chip-transform-rotate")).toHaveAttribute("aria-pressed", "false");
  await expect(axisButton(page, "X")).toHaveAttribute("aria-pressed", "true");
  await expect(chipInput(page)).toHaveValue("0");
  await expect(chip(page).getByTestId("chip-confirm")).toBeVisible();
  await expect(chip(page).getByTestId("chip-cancel")).toBeVisible();
});

test("t with nothing selected asks for a body and arms nothing", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await page.keyboard.press("t");
  await expect(page.getByText("Select a body to move")).toBeVisible();
  await expect(page.getByTestId("model-tool-chip")).toHaveCount(0);
});

test("Move X 30 → ✓ moves the body and writes ONE history row", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);
  const rowsBefore = await getFeatureLabels(page);

  await armTransform(page, BODY);
  await typeAndConfirm(page, "30");

  await expectMinX(page, BODY, Math.round(before.min[0] + 30));
  const after = await bodyBounds(page, BODY);
  // A placement is RIGID: the extents translate, they do not grow.
  expect(after!.max[0] - after!.min[0]).toBeCloseTo(before.max[0] - before.min[0], 3);
  expect(after!.min[1]).toBeCloseTo(before.min[1], 3);
  expect(after!.min[2]).toBeCloseTo(before.min[2], 3);

  const rows = await getFeatureLabels(page);
  expect(rows).toHaveLength(rowsBefore.length + 1);
  expect(rows.at(-1)).toBe("Move");
});

test("a SECOND placement on the same body FOLDS into the one record", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);

  await armTransform(page, BODY);
  await typeAndConfirm(page, "30");
  await expectMinX(page, BODY, Math.round(before.min[0] + 30));
  expect(await moveRows(page)).toHaveLength(1);

  // Re-arm the SAME body. The backend lineage query names the record we just
  // wrote, so the cluster opens on its stored cumulative value…
  await armTransform(page, BODY);
  await expect(chipInput(page)).toHaveValue("30");
  expect((await extrudeDebug(page))?.transformFold).not.toBeNull();

  // …and typing 50 REPLACES that component rather than adding to it.
  await typeAndConfirm(page, "50");
  await expectMinX(page, BODY, Math.round(before.min[0] + 50)); // not +80

  // STILL one row: the record was re-edited, not stacked.
  expect(await moveRows(page)).toHaveLength(1);
});

test("double-clicking the Move row seeds the same cluster and Enter re-edits it in place", async ({
  page,
}) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);

  await armTransform(page, BODY);
  await typeAndConfirm(page, "30");
  await expectMinX(page, BODY, Math.round(before.min[0] + 30));

  // Reach the row in the inspector's full timeline and re-open it. The SELECT
  // before the double-click is not ceremony: selecting a row reflows the list
  // (the feature header grows an "editable" line), so a bare dblclick's two
  // clicks would land on different rows and never dispatch `dblclick` at all.
  await openFullTimeline(page);
  const row = page.locator('[data-testid^="history-row-"]').filter({ hasText: "Move" }).first();
  await row.click();
  await row.dblclick();

  await expect.poll(async () => (await extrudeDebug(page))?.transformPhase).toBe("armed");
  await expect(chipInput(page)).toHaveValue("30"); // seeded from the STORED params
  expect((await extrudeDebug(page))?.transformFold).not.toBeNull();

  // Enter inside the chip input applies the value and confirms in one gesture.
  await chipInput(page).fill("12");
  await chipInput(page).press("Enter");

  await expectMinX(page, BODY, Math.round(before.min[0] + 12));
  expect(await moveRows(page)).toHaveLength(1); // the SAME row
});

test("undo restores the body's original position", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);

  await armTransform(page, BODY);
  await typeAndConfirm(page, "30");
  await expectMinX(page, BODY, Math.round(before.min[0] + 30));

  await page.keyboard.press("ControlOrMeta+z");

  await expectMinX(page, BODY, Math.round(before.min[0]));
  expect(await moveRows(page)).toHaveLength(0);
  // The body is still THERE — an undone placement must not delete it.
  expect(await bodyBounds(page, BODY)).not.toBeNull();
});

test("Rotate Z 90 commits a rotation, not a translation", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);

  await armTransform(page, BODY);
  await chip(page).getByTestId("chip-transform-rotate").click();
  await axisButton(page, "Z").click();
  await expect(axisButton(page, "Z")).toHaveAttribute("aria-pressed", "true");
  await typeAndConfirm(page, "90");

  // 90° about Z swaps the box's X and Y extents (80×60 → 60×80).
  await expect
    .poll(async () => {
      const b = await bodyBounds(page, BODY);
      return b ? Math.round(b.max[0] - b.min[0]) : null;
    })
    .toBe(Math.round(before.max[1] - before.min[1]));

  const after = await bodyBounds(page, BODY);
  expect(after!.max[1] - after!.min[1]).toBeCloseTo(before.max[0] - before.min[0], 2);
  expect(after!.max[2] - after!.min[2]).toBeCloseTo(before.max[2] - before.min[2], 3); // Z untouched
  expect(await moveRows(page)).toHaveLength(1);
});

test("✕ cancels an armed placement with no row and no motion", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);
  const rowsBefore = await getFeatureLabels(page);

  await armTransform(page, BODY);
  await chipInput(page).fill("30");
  await chipInput(page).blur();
  await chip(page).getByTestId("chip-cancel").click();

  await expect(page.getByTestId("model-tool-chip")).toHaveCount(0);
  expect(await getFeatureLabels(page)).toEqual(rowsBefore);
  const after = await bodyBounds(page, BODY);
  expect(after!.min[0]).toBeCloseTo(before.min[0], 3);
});

test("a multi-body selection commits ONE record that moves both bodies", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });

  // A second body: the mock's STEP import fabricates one and ingests it through
  // the same document-changed path (there is only one seeded body otherwise).
  await page.getByRole("button", { name: "File" }).click();
  await page.getByRole("menuitem", { name: /Import STEP/ }).click();
  await expect.poll(async () => (await getFeatureLabels(page)).includes("Import")).toBe(true);

  const bodyIds = await page.evaluate(() =>
    Object.keys(
      (
        window as unknown as {
          __stores?: { document: { getState(): { bodies: Record<string, unknown> } } };
        }
      ).__stores?.document.getState().bodies ?? {},
    ),
  );
  expect(bodyIds.length).toBeGreaterThanOrEqual(2);
  const [a, b] = bodyIds;
  const beforeA = await waitForBody(page, a);
  const beforeB = await waitForBody(page, b);
  const rowsBefore = await getFeatureLabels(page);

  await armTransform(page, a, b);
  await typeAndConfirm(page, "40");

  await expectMinX(page, a, Math.round(beforeA.min[0] + 40));
  await expectMinX(page, b, Math.round(beforeB.min[0] + 40));
  // ONE record covering both — not one per body.
  expect(await getFeatureLabels(page)).toHaveLength(rowsBefore.length + 1);
  expect(await moveRows(page)).toHaveLength(1);
});

// ── WP-B W2: the drag gizmo ──────────────────────────────────────────────────

/** The live placement as the controller publishes it (`?vpdebug`). */
async function placement(page: Page): Promise<{
  translate: number[];
  angleDeg: number;
  mode: string;
  axis: string;
  copy: boolean;
  gizmo: boolean;
}> {
  // Null until the first `updateDebug` — i.e. before any tool has ever armed.
  const d = (await extrudeDebug(page)) ?? {};
  return {
    translate: (d.transformTranslate as number[]) ?? [0, 0, 0],
    angleDeg: (d.transformAngleDeg as number) ?? 0,
    mode: (d.transformMode as string) ?? "move",
    axis: (d.transformAxis as string) ?? "X",
    copy: (d.transformCopy as boolean) ?? false,
    gizmo: (d.transformGizmo as boolean) ?? false,
  };
}

/** Enter confirms the armed placement (the same gesture the W1 cases use). */
async function confirmWithEnter(page: Page): Promise<void> {
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("model-tool-chip")).toHaveCount(0);
}

/** Poll a body's bbox until its min corner matches `want` (rounded), then return it. */
async function expectMin(page: Page, bodyId: string, want: number[]): Promise<void> {
  await expect
    .poll(async () => {
      const b = await bodyBounds(page, bodyId);
      return b ? b.min.map((v) => Math.round(v)) : null;
    })
    .toEqual(want.map((v) => Math.round(v)));
}

test("the gizmo appears with the armed placement and disappears with it", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForBody(page, BODY);
  expect((await placement(page)).gizmo).toBeFalsy();

  await armTransform(page, BODY);
  await expect.poll(async () => (await placement(page)).gizmo).toBe(true);
  // Every handle is reachable — three kinds, keyed by axis.
  await findGizmoHandle(page, { kind: "axis", axis: "X" });
  await findGizmoHandle(page, { kind: "plane", axis: "Z" });
  await findGizmoHandle(page, { kind: "ring", axis: "Z" });

  await chip(page).getByTestId("chip-cancel").click();
  await expect.poll(async () => (await placement(page)).gizmo).toBe(false);
});

test("dragging the X arrow moves the body along X only, and Enter commits it", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);
  await armTransform(page, BODY);

  const grab = await findGizmoHandle(page, { kind: "axis", axis: "X" });
  await dragFromTo(page, grab, { x: grab.x + 120, y: grab.y });

  // The chip is live off the same FSM the ghost reads…
  const dragged = await placement(page);
  expect(dragged.mode).toBe("move");
  expect(dragged.axis).toBe("X");
  expect(Math.abs(dragged.translate[0])).toBeGreaterThan(0);
  expect(dragged.translate[1]).toBe(0); // an arrow writes ONE component
  expect(dragged.translate[2]).toBe(0);
  await expect(chipInput(page)).toHaveValue(String(dragged.translate[0]));
  // …and the release keeps it armed: nothing is on the timeline yet.
  await expect.poll(async () => (await extrudeDebug(page))?.transformPhase).toBe("armed");
  expect(await moveRows(page)).toHaveLength(0);

  await confirmWithEnter(page);
  await expectMin(page, BODY, [before.min[0] + dragged.translate[0], before.min[1], before.min[2]]);
  expect(await moveRows(page)).toHaveLength(1);
});

test("dragging the Z plane quad moves in TWO axes at once", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);
  await armTransform(page, BODY);

  const grab = await findGizmoHandle(page, { kind: "plane", axis: "Z" });
  // A diagonal drag: the quad's plane carries both of its in-plane axes.
  await dragFromTo(page, grab, { x: grab.x + 110, y: grab.y - 70 });

  const dragged = await placement(page);
  expect(dragged.mode).toBe("move");
  expect(dragged.translate[0]).not.toBe(0);
  expect(dragged.translate[1]).not.toBe(0);
  // The quad's NORMAL is the one axis a plane drag cannot move.
  expect(dragged.translate[2]).toBe(0);

  await confirmWithEnter(page);
  await expectMin(page, BODY, [
    before.min[0] + dragged.translate[0],
    before.min[1] + dragged.translate[1],
    before.min[2],
  ]);
});

test("dragging the Z ring re-types the cluster to Rotate and commits a rotation", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);
  await armTransform(page, BODY);

  const grab = await findGizmoHandle(page, { kind: "ring", axis: "Z" });
  // Tangential-ish: any sweep about the ring's own axis will do — the assertion
  // below reads back the angle that actually resulted.
  await dragFromTo(page, grab, { x: grab.x - 90, y: grab.y + 90 });

  const dragged = await placement(page);
  expect(dragged.mode).toBe("rotate"); // the HANDLE re-typed the placement…
  expect(dragged.axis).toBe("Z");
  expect(dragged.translate).toEqual([0, 0, 0]); // …and a rotation is not a move
  expect(Math.abs(dragged.angleDeg)).toBeGreaterThanOrEqual(15); // the 15° snap
  // …and the chip segments follow the FSM, not the other way round.
  await expect(chip(page).getByTestId("chip-transform-rotate")).toHaveAttribute("aria-pressed", "true");
  await expect(axisButton(page, "Z")).toHaveAttribute("aria-pressed", "true");

  await confirmWithEnter(page);

  // Rotating an 80×60 box about its own centre on Z gives an AABB of
  // (80|cosθ| + 60|sinθ|) × (80|sinθ| + 60|cosθ|), and leaves Z untouched.
  const rad = (Math.abs(dragged.angleDeg) * Math.PI) / 180;
  const wantX = 80 * Math.abs(Math.cos(rad)) + 60 * Math.abs(Math.sin(rad));
  await expect
    .poll(async () => {
      const b = await bodyBounds(page, BODY);
      return b ? Math.round(b.max[0] - b.min[0]) : null;
    })
    .toBe(Math.round(wantX));
  const after = (await bodyBounds(page, BODY))!;
  expect(after.max[2] - after.min[2]).toBeCloseTo(before.max[2] - before.min[2], 3);
  expect(await moveRows(page)).toHaveLength(1);
});

test("Alt-drag places a COPY: one more body, sources left where they were", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const before = await waitForBody(page, BODY);
  const bodiesBefore = await bodyOptions(page).count();
  await armTransform(page, BODY);

  const grab = await findGizmoHandle(page, { kind: "axis", axis: "X" });
  await dragFromTo(page, grab, { x: grab.x + 120, y: grab.y }, ["Alt"]);

  const dragged = await placement(page);
  expect(dragged.copy).toBe(true);
  // Alt writes the SAME flag the segment shows — one writer, visible state.
  await expect(chip(page).getByTestId("chip-transform-copy")).toHaveAttribute("aria-pressed", "true");

  await confirmWithEnter(page);

  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  // The SOURCE never moved — that is the whole difference between copy and move.
  await expectMin(page, BODY, before.min);
  expect(await moveRows(page)).toHaveLength(1);
});

test("the Copy segment toggles the flag without a gizmo drag", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForBody(page, BODY);
  await armTransform(page, BODY);

  const copyBtn = chip(page).getByTestId("chip-transform-copy");
  await expect(copyBtn).toHaveAttribute("aria-pressed", "false");
  await copyBtn.click();
  await expect(copyBtn).toHaveAttribute("aria-pressed", "true");
  await expect.poll(async () => (await placement(page)).copy).toBe(true);
});
