import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditor, bodyOptions } from "./helpers";

/*
 * HISTORY-HARDEN H7b / H7a — authoring while ROLLED BACK inserts AT the cursor.
 *
 * Core rule (`onecad-core/src/history/timeline.rs:174-185` `insert_at_cursor`,
 * itself OneCAD-CPP `AddOperationCommand.cpp:33-37`): the record goes in at
 * `min(cursor, len)`, the cursor advances past it, and that record plus every
 * later step become Dirty. H7a made every authoring path send `atCursor: true`;
 * this pins the visible half — the new row appears WHERE the bar is, the bar moves
 * with it, and the ops still beyond it stay drafts instead of silently vanishing.
 *
 * The authored op here is an in-editor STEP import (File ▸ Import STEP…), the
 * cheapest real authoring path on the mock lane — no sketch/gesture choreography
 * between the rollback and the commit, so a failure can only be the insert rule.
 * The extrude-shaped equivalent is covered against the same mock cursor in
 * `src/ipc/mockClient.rollback.test.ts`.
 */

interface Row {
  id: string;
  label: string;
  status: string;
}

async function timeline(page: Page): Promise<{ applied: number; rows: Row[] }> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { appliedOps: number; features: Row[] } } };
    };
    const s = w.__stores?.document.getState();
    return { applied: s?.appliedOps ?? -1, rows: (s?.features ?? []) as Row[] };
  });
}

async function openFullTimeline(page: Page): Promise<void> {
  await bodyOptions(page).first().click();
  await expect(page.getByTestId("history-row-f3")).toBeVisible();
  await page.getByTestId("history-row-f3").click();
  await expect(page.getByTestId("history-suppress-f3")).toBeVisible();
}

test("a feature authored while rolled back lands at the cursor, not at the tail", async ({
  page,
}) => {
  await openEditor(page);
  await openFullTimeline(page);

  // Roll to row f2 (index 1) ⇒ cursor 2; f3/f4/f5 become drafts.
  await page.getByTestId("history-row-f2").click({ button: "right" });
  await page.getByTestId("history-menu-roll-here").click();
  await expect.poll(async () => (await timeline(page)).applied).toBe(2);
  await expect(page.getByTestId("history-rollback-banner")).toHaveText(/3 operations rolled back/);

  // Author one op (File ▸ Import STEP… — Rust owns the dialog; the mock "picks").
  await page.getByRole("button", { name: "File" }).click();
  await page.getByRole("menuitem", { name: /Import STEP/ }).click();
  await expect(page.getByText(/^Imported 1 body/)).toBeVisible();

  const after = await timeline(page);
  // (1) It landed AT the cursor — index 2, between f2 and the drafts — and the
  //     cursor advanced past it (`cursor = index + 1`), NOT at the tail.
  expect(after.rows.map((r) => r.id).slice(0, 3)).toEqual(["f1", "f2", after.rows[2].id]);
  expect(after.rows[2].label).toBe("Import");
  expect(after.rows.at(-1)!.id).toBe("f5");
  expect(after.applied).toBe(3);
  expect(after.rows).toHaveLength(6);

  // (2) The inserted node + everything after it is Dirty (pending regen); the
  //     applied prefix ahead of it is untouched.
  expect(after.rows.slice(0, 2).map((r) => r.status)).toEqual(["ok", "ok"]);
  expect(after.rows.slice(3).every((r) => r.status === "dirty")).toBe(true);

  // (3) The chrome follows: the same three ops are still beyond the bar, and the
  //     new row is INSIDE it (applied), not a draft.
  await expect(page.getByTestId("history-rollback-banner")).toHaveText(/3 operations rolled back/);
  await expect(page.getByTestId(`history-row-${after.rows[2].id}`)).toHaveAttribute(
    "data-applied",
    "true",
  );
  await expect(page.getByTestId("history-row-f3")).toHaveAttribute("data-applied", "false");

  // (4) Rolling to end restores the whole timeline, the insert included.
  await page.getByTestId("history-roll-to-end").click();
  await expect.poll(async () => (await timeline(page)).applied).toBe(6);
  await expect(page.getByTestId("history-rollback-banner")).toHaveCount(0);
});
