import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditor } from "./helpers";

/*
 * HISTORY-HARDEN H9 — repair works in a MULTI-BODY document without the user
 * having to pre-select the operated body.
 *
 * Before H9 the frontend had to guess which body a failing step operated on
 * (`historyActions.deriveOperatedBody`): a repair candidate is a snapshot
 * `TopoKey` that can only be promoted against a body, and the projection carried
 * no feature→body link. With more than one body the panel REFUSED outright with
 * "operated body is ambiguous", so repair was unavailable in exactly the
 * documents where topological naming is hardest. The backend now derives the
 * body from the failing record's own inputs and ships it on the `needs-repair`
 * item (`NeedsRepairItemDto.bodyId`).
 *
 * Mock-lane feasibility: the mock has no regen and therefore never emits a
 * `needs-repair` event on its own, so the spec pushes the event through the SAME
 * store entry point the real backend event uses (`repairStore.applyEvent`, which
 * `EditorScreen` wires `client.onNeedsRepair` to). Everything downstream of that
 * is the real thing: the real panel, the real `resolveRefs`, the real
 * `promoteSelection`, and the real `EditOperationInput` command.
 */

type RepairItemLike = {
  opId: string;
  refId: string;
  reason: string;
  candidateCount: number;
  bodyId?: string;
};

/** Push a `needs-repair` event exactly as the backend relay does. */
async function pushNeedsRepair(page: Page, items: RepairItemLike[]): Promise<void> {
  await page.evaluate((payload) => {
    const w = window as unknown as {
      __stores?: {
        repair: {
          getState(): {
            applyEvent(e: { revision: number; snapshotId: number; items: RepairItemLike[] }): void;
            openPanel(): void;
          };
        };
      };
    };
    w.__stores?.repair.getState().applyEvent({ revision: 9, snapshotId: 9, items: payload });
  }, items);
}

/**
 * Give the document a SECOND body. The ambiguity this spec is about is a
 * frontend-derivation problem, and `documentStore.bodies` IS the frontend's view
 * of how many bodies exist — so seeding it there is the condition itself, not a
 * shortcut around one.
 */
async function addSecondBody(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        document: {
          getState(): { bodies: Record<string, unknown> };
          setState(p: Record<string, unknown>): void;
        };
      };
    };
    const doc = w.__stores?.document;
    if (!doc) return;
    doc.setState({
      bodies: {
        ...doc.getState().bodies,
        body2: { id: "body2", name: "Body 2", visible: true },
      },
    });
  });
}

test("a repair item's bodyId lets a MULTI-BODY document rebind with no body pre-selected", async ({
  page,
}) => {
  await openEditor(page);
  await addSecondBody(page);

  // f3 is the seeded Fillet feature; its input0 is a fillet edge slot, which IS
  // element-addressable (`inputPathFor("Fillet", 0)` → `filletEdges{0}`).
  await pushNeedsRepair(page, [
    { opId: "f3", refId: "f3.input0", reason: "ambiguous", candidateCount: 2, bodyId: "body2" },
  ]);
  await expect(page.getByTestId("repair-banner")).toBeVisible();
  await page.getByTestId("repair-banner").click();

  // Nothing is selected: pre-H9 this is the state that produced the sticky
  // "operated body is ambiguous (2 bodies)" refusal.
  await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { selection: { getState(): { clear(): void } } };
    };
    w.__stores?.selection.getState().clear();
  });

  const head = page.getByTestId("repair-item-head-f3.input0");
  await expect(head).toBeVisible();
  await head.click();

  // The candidates load (the slot HAS a rebind path, so the panel fetched them).
  const candidate = page.locator('[data-testid^="repair-candidate-f3.input0-"]').first();
  await expect(candidate).toBeVisible();
  await expect(page.getByTestId("repair-not-rebindable-f3.input0")).toHaveCount(0);

  await candidate.click();

  // The repair went through — no ambiguity refusal anywhere on the status bar.
  await expect(page.getByText("Reference repaired")).toBeVisible();
  await expect(page.getByText(/operated body is ambiguous/)).toHaveCount(0);
  // (WHICH body the candidate was promoted against is asserted at the unit level —
  // `historyActions.test.ts` "the item's bodyId WINS over an unrelated single-body
  // selection". This spec's job is that the UI path no longer refuses.)
});

test("a slot with NO rebind path says so instead of offering a doomed click", async ({ page }) => {
  await openEditor(page);

  // A Hole's `inputs[0]` is the HOST BODY. A repair candidate is an element
  // TopoKey, so nothing can be written there — pre-H9 the panel offered
  // candidates and sent `filletEdges{0}`, which the core rejects.
  await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { setState(p: Record<string, unknown>): void } };
    };
    w.__stores?.document.setState({
      features: [
        {
          id: "h1",
          kind: "extrude",
          opType: "Hole",
          label: "Hole",
          valueText: "",
          status: "needsRepair",
        },
      ],
    });
  });
  await pushNeedsRepair(page, [
    { opId: "h1", refId: "h1.input0", reason: "no-candidates", candidateCount: 0, bodyId: "body1" },
  ]);
  await page.getByTestId("repair-banner").click();
  await page.getByTestId("repair-item-head-h1.input0").click();

  await expect(page.getByTestId("repair-not-rebindable-h1.input0")).toBeVisible();
  await expect(
    page.getByText("This reference must be re-picked in the feature editor"),
  ).toBeVisible();
  await expect(page.locator('[data-testid^="repair-candidate-h1.input0-"]')).toHaveCount(0);
});

test("hovering a candidate drops a world-anchored marker in the viewport", async ({ page }) => {
  await openEditor(page);
  await pushNeedsRepair(page, [
    { opId: "f3", refId: "f3.input0", reason: "ambiguous", candidateCount: 2, bodyId: "body1" },
  ]);
  await page.getByTestId("repair-banner").click();
  await page.getByTestId("repair-item-head-f3.input0").click();

  const candidate = page.locator('[data-testid^="repair-candidate-f3.input0-"]').first();
  await expect(candidate).toBeVisible();
  // `repairStore.hoveredWorldPos` had ZERO consumers before H9 — hovering told
  // the user nothing about WHERE on the model a candidate sits.
  await expect(page.locator('[data-testid="repair-marker"]')).toHaveCount(0);
  await candidate.hover();
  await expect(page.locator('[data-testid="repair-marker"]')).toBeVisible();

  // Leaving the row clears it (no stale crosshair left behind).
  await page.getByTestId("repair-close").hover();
  await expect(page.locator('[data-testid="repair-marker"]')).toHaveCount(0);
});
