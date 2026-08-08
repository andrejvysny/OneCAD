import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, findFaceOnBody, getFeatureLabels } from "./helpers";
import { seedSelection, toolPhases, expectArmed } from "./modelToolHelpers";

/*
 * OFFSET FACE (SCHEMA §7.3 `op.offsetFace` + §7.6 `PrepareOffsetFace`).
 *
 * The gesture is Shell's arm and Extrude's manipulator, but the thing worth
 * testing end-to-end is neither: it is the AUTHORING TRANSACTION in front of them.
 * The operative set an OffsetFace freezes is NOT the user's picks — the kernel
 * auto-propagates an offset across G1-tangent junctions and cannot hold a tangent
 * neighbour fixed — so the worker computes the closure first, Rust mints ids for
 * every face in it, and only then does the tool arm. Nothing about that is
 * visible in the DOM, so the specs read `offsetFaceCount` / `offsetPrepared` off
 * the `?vpdebug` surface and drive the rest through the real chip.
 *
 * MOCK LANE, gesture only. `mockClient.prepareOffsetFace` answers with the picks
 * themselves (no G1 expansion — that needs OCCT) and publishes no candidate mesh,
 * so there is nothing honest to assert about geometry here. The real numbers are
 * pinned against the kernel by `worker/tests/test_offsetface.cpp` and
 * `src-tauri/tests/offset_face.rs`.
 */
/**
 * A REAL face of the demo body, found through the engine's own `probePick`.
 *
 * Deliberately not a fabricated `f:2` literal: the arrow + ghost lane resolves
 * each operative face's PLANAR FRAME off the local mesh, and a topoKey that no
 * mesh entry can name degrades the tool to the screen-space drag — which would
 * quietly test the fallback while claiming to test the manipulator.
 */
async function realFace(page: Page): Promise<Record<string, unknown>> {
  const hit = await findFaceOnBody(page);
  return {
    kind: "face",
    id: `${hit.bodyId}#${hit.topoKey}`,
    bodyId: hit.bodyId,
    topoKey: hit.topoKey,
    anchor: { worldPoint: [0, 0, 15] },
  };
}

/** A sibling face on the SAME body — only its count matters (multi-face gating). */
const SECOND_FACE = {
  kind: "face",
  id: "body1#f:3",
  bodyId: "body1",
  topoKey: "f:3",
  elementId: "el-face-3",
  anchor: { worldPoint: [0, 0, -15] },
};

/** A face on a DIFFERENT body — the `crossBody` refusal's input. */
const FOREIGN_FACE = {
  kind: "face",
  id: "body2#f:0",
  bodyId: "body2",
  topoKey: "f:0",
  elementId: "el-face-0",
  anchor: { worldPoint: [90, 0, 0] },
};

const offsetTool = (page: Page) => page.getByRole("button", { name: "Offset face", exact: true });

async function armOffsetFace(
  page: Page,
  faces?: Array<Record<string, unknown>>,
): Promise<void> {
  await seedSelection(page, faces ?? [await realFace(page)]);
  await offsetTool(page).click();
  await expectArmed(page, "offsetFace");
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
}

test("Offset face asks for faces when nothing is selected", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  // The toolbar button itself is disabled with nothing selected (below) — this
  // scenario is the reactive fallback for reaching the tool without it, e.g. the
  // keyboard shortcut, same pattern as filletChamfer.spec.ts's equivalent test.
  await page.keyboard.press("Shift+O");
  await expect(page.getByText("Select faces to offset, then Offset face")).toBeVisible();
  expect((await toolPhases(page))?.offsetFacePhase).toBe("idle");
  // No handshake, so nothing was frozen and a ✓ could not commit anything.
  expect((await toolPhases(page))?.offsetPrepared ?? null).toBeNull();
});

test("a CROSS-BODY selection is refused and never arms", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await seedSelection(page, [await realFace(page), FOREIGN_FACE]);
  await page.keyboard.press("Shift+O");
  // An offset is defined against ONE body: a pick spanning two has no target to
  // name, so the tool refuses rather than binding to whichever came first.
  await expect(page.getByText(/must belong to the same body/)).toBeVisible();
  expect((await toolPhases(page))?.offsetFacePhase).toBe("idle");
  await expect(page.getByTestId("chip-confirm")).toHaveCount(0);
});

test("the Offset face toolbar button grays out and explains why for a cross-body selection", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await seedSelection(page, [await realFace(page), FOREIGN_FACE]);
  await expect(offsetTool(page)).toHaveAttribute("aria-disabled", "true");
  await offsetTool(page).hover();
  await expect(page.getByRole("tooltip")).toHaveText(/must belong to the same body/);
  // A disabled button is truly inert — clicking it must not arm the tool
  // (no debug-surface tool switch ever fires, so assert on the DOM instead).
  await offsetTool(page).click({ force: true });
  await expect(offsetTool(page)).toHaveAttribute("aria-pressed", "false");
  await expect(page.getByTestId("chip-confirm")).toHaveCount(0);
});

test("arming freezes the handshake's closure and records its revision", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await armOffsetFace(page);
  const d = await toolPhases(page);
  expect(d?.offsetFaceCount).toBe(1); // MOCK LIMIT: the closure IS the picks
  expect(d?.offsetDistanceType).toBe("Offset");
  expect(d?.offsetChainTangent).toBe(true);
  // A real planar face resolves its frame, so the 3D arrow + ghost lane is live
  // rather than the screen-space fallback.
  expect(d?.offsetDegraded).toBe(false);
  // The commit gate: a handshake that answered against THIS head.
  expect(typeof d?.offsetPrepared).toBe("number");
});

test("a single face offers the distance types; a MULTI-face closure offers none", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await armOffsetFace(page);
  // The box face is planar, so the planar pair is what the chip renders.
  await expect(page.getByTestId("chip-offset-type-offset")).toBeVisible();
  await expect(page.getByTestId("chip-offset-type-total")).toBeVisible();
  // ABSENT, not disabled: a planar face has no radius to grey out.
  await expect(page.getByTestId("chip-offset-type-radius")).toHaveCount(0);
  await expect(page.getByTestId("chip-offset-tangent")).toBeVisible();

  const face = await realFace(page);
  await page.getByTestId("chip-cancel").click();
  await armOffsetFace(page, [face, SECOND_FACE]);
  expect((await toolPhases(page))?.offsetFaceCount).toBe(2);
  // SCHEMA §7.3: only `Offset` admits a multi-face set, and one segment is not a
  // choice — so the group is not rendered at all.
  await expect(page.getByTestId("chip-offset-type-offset")).toHaveCount(0);
  await expect(page.getByTestId("chip-offset-type-total")).toHaveCount(0);
});

test("the chip's typed distance reaches the armed op", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await armOffsetFace(page);
  const input = page.getByLabel("Dimension value");
  await input.fill("6");
  await input.blur();
  await expect.poll(async () => (await toolPhases(page))?.offsetDistance).toBe(6);
  expect((await toolPhases(page))?.offsetFacePhase).toBe("armed"); // blur never commits
});

test("the visible ✓ commits the armed offset", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await armOffsetFace(page);
  const before = await getFeatureLabels(page);

  await page.getByTestId("chip-confirm").click();

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  // Matches `dto.rs default_label` — the row must not read "OffsetFace".
  expect((await getFeatureLabels(page)).at(-1)).toBe("Offset face");
  await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByTestId("chip-confirm")).toHaveCount(0);
});

test("Enter commits the armed offset; ✕ cancels it with no row", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await armOffsetFace(page);
  const before = await getFeatureLabels(page);

  await page.getByTestId("chip-cancel").click();
  await expect.poll(async () => (await toolPhases(page))?.offsetFacePhase).toBe("idle");
  expect(await getFeatureLabels(page)).toEqual(before);
  // The handshake is dropped with the arm: a stale frozen closure must never
  // survive a cancel and be reachable by the next ✓.
  expect((await toolPhases(page))?.offsetPrepared ?? null).toBeNull();

  await armOffsetFace(page);
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  expect((await getFeatureLabels(page)).at(-1)).toBe("Offset face");
});

test("⇧O arms Offset face from the model toolbar's keymap", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await seedSelection(page, [await realFace(page)]);
  // Model ⇧O is the 3D offset; sketch ⇧O is the 2D one. Same operation, one
  // dimension apart, mode-resolved (see the COLLISIONS note in shortcuts/keymap).
  await page.keyboard.press("Shift+O");
  await expectArmed(page, "offsetFace");
  await expect(offsetTool(page)).toHaveAttribute("aria-pressed", "true");
});
