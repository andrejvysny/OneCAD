import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, bodyOptions } from "./helpers";
import { seedSelection, toolPhases } from "./modelToolHelpers";

/*
 * HISTORY-HARDEN H10 — the dependency view's dependent COUNT reaches the delete/
 * suppress confirmations through a REAL producer-chain, not a stubbed fetcher.
 *
 * The seeded base fixture (f1..f5) carries no tracked lineage on the mock lane —
 * `featureTouched` (the mock's stand-in for `inputs.bodies ∪ outputs`) is only
 * populated by a real op commit, never by the static seed (see
 * `mockClient.ts` `mockFeatureDependencies`'s HONEST LIMIT doc). So this spec
 * builds a real two-op chain on body1 (Fillet, then Shell — both touch the SAME
 * body, no sketch/geometry drawing needed) and proves the FIRST op's row shows
 * "1 dependent" once the second op exists downstream of it.
 */

const EDGE_REF = {
  kind: "edge",
  id: "body1#e:5",
  bodyId: "body1",
  topoKey: "e:5",
  elementId: "el-edge-5",
  anchor: { worldPoint: [40, 0, 15] },
};

const FACE_REF = {
  kind: "face",
  id: "body1#f:2",
  bodyId: "body1",
  topoKey: "f:2",
  elementId: "el-face-2",
  anchor: { worldPoint: [0, 0, 0] },
};

/** The projection's features (`__stores.document`, dev-only). */
async function featureIds(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { features: Array<{ id: string }> } } };
    };
    return (w.__stores?.document.getState().features ?? []).map((f) => f.id);
  });
}

/** Commit a plain Fillet on body1's edge (no drag — the default radius applies).
 *  Returns the newly-added row's id, having POLLED for the commit to actually
 *  land — the confirm click resolves before the mock's simulated latency does. */
async function commitFillet(page: Page): Promise<string> {
  const before = await featureIds(page);
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await page.getByTestId("chip-confirm").click();
  await expect.poll(async () => (await featureIds(page)).length).toBe(before.length + 1);
  return (await featureIds(page)).at(-1)!;
}

/** Commit a plain Shell on body1's face (Enter commits with no drag — same as
 *  `shell-preview.spec.ts`'s "Enter commits the armed shell" case). */
async function commitShell(page: Page): Promise<string> {
  const before = await featureIds(page);
  await seedSelection(page, [FACE_REF]);
  await page.getByRole("button", { name: "Shell", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.shellPhase).toBe("armed");
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await featureIds(page)).length).toBe(before.length + 1);
  return (await featureIds(page)).at(-1)!;
}

/**
 * Open the FULL timeline (per-row affordances). A body selection only renders a
 * 3-row LINEAGE SLICE (InspectorPanel `SelectionState`), which a freshly-created
 * row (well past index 2) never lands in — so this reaches the full view via the
 * always-present seeded `f3` row first, same as `history-suppress.spec.ts`'s
 * `openFeatureTimeline`, THEN selects `rowId`.
 */
async function openFullTimelineOn(page: Page, rowId: string): Promise<void> {
  await bodyOptions(page).first().click();
  await expect(page.getByTestId("history-row-f3")).toBeVisible();
  await page.getByTestId("history-row-f3").click();
  await expect(page.getByTestId(`history-row-${rowId}`)).toBeVisible();
  await page.getByTestId(`history-row-${rowId}`).click();
  await expect(page.getByTestId(`history-suppress-${rowId}`)).toBeVisible();
}

test("a real two-op producer chain on the same body shows the dependent count on the earlier op's delete/suppress", async ({
  page,
}) => {
  await openEditorDebug(page);

  const filletId = await commitFillet(page);
  const shellId = await commitShell(page);
  expect(shellId).not.toBe(filletId);

  await openFullTimelineOn(page, filletId);

  // The row's own × two-click confirm: hover primes the fetch, the × arms it.
  await page.getByTestId(`history-suppress-${filletId}`).hover();
  await page.getByTestId(`history-delete-${filletId}`).click();
  await expect(page.getByTestId(`history-delete-confirm-${filletId}`)).toHaveAttribute(
    "title",
    "Confirm delete — 1 dependent",
  );

  // The context menu offers the same count on Suppress and Delete.
  await page.getByTestId(`history-row-${filletId}`).click({ button: "right" });
  await expect(page.getByTestId("history-menu-suppress")).toHaveText("Suppress — 1 dependent");
  await expect(page.getByTestId("history-menu-delete")).toHaveText("Delete — 1 dependent");

  // Close via an outside mousedown (`Popover.tsx`'s own close rule) rather than
  // right-clicking the shell row directly: the two rows sit close enough that the
  // open menu visually covers it, and Escape is unsafe here — it is also the
  // editor's "clear selection" chord (`history-rollback.spec.ts`'s own note),
  // which would kick the panel out of the feature timeline entirely.
  await page.getByText("History").click();
  await expect(page.getByTestId("history-menu-suppress")).toHaveCount(0);

  // The LATER op (Shell) has nothing downstream of IT — no count at all.
  await page.getByTestId(`history-row-${shellId}`).click({ button: "right" });
  await expect(page.getByTestId("history-menu-suppress")).toHaveText("Suppress");
  await expect(page.getByTestId("history-menu-delete")).toHaveText("Delete");
});
