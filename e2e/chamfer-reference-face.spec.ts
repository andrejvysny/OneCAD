import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, getFeatureLabels } from "./helpers";
import { seedSelection, toolPhases, openFilletOverflow } from "./modelToolHelpers";

/*
 * The CHAMFER REFERENCE FACE (SCHEMA §7.3 `referenceFaces`, kernel-hardening WP-F)
 * in the browser lane: arm a chamfer on the seed box, make it asymmetric, flip the
 * reference face, commit, and read the pair back off the committed record.
 *
 * WHY THE MOCK CAN PROVE THIS. `adjacentFaces` is the one part of `PrepareEdgeOp`
 * the mock lane can answer honestly: the seed box's twelve edges and six faces come
 * from the same tables `makeBoxMesh` renders, so `mockAdjacentFaces` reports the
 * TRUE pair rather than a fabrication. Edge `e:5` runs corner "101"→"111", and the
 * two faces whose rings hold both ends are `f:0` (+X) and `f:4` (+Z), in
 * face-ordinal order — so `adjacentFaces[0]` is `f:0`, the legacy smaller-ordinal
 * face a default pick must reproduce.
 *
 * WHAT THIS SPEC DOES NOT ASSERT, and why. The mock builds no chamfer GEOMETRY at
 * all (`mockClient.mutateOp` re-emits the target body and adds a history row — its
 * own stated MOCK LIMIT), so there is no mesh here in which the 4 mm leg could be
 * seen to move from one face to the other. That measurement is pinned against the
 * real kernel by `worker/tests/test_chamfer_reference_face.cpp` and the Rust lane.
 * What this lane owns is the UI chain: the control appears only when there is a
 * second face to flip to, the flip changes which face the pair names, and the pair
 * survives the commit into the record's stored params in the §7.3 lockstep form.
 */

const EDGE_REF = {
  kind: "edge",
  id: "body1#e:5",
  bodyId: "body1",
  topoKey: "e:5",
  elementId: "el-edge-5",
  anchor: { worldPoint: [40, 0, 15] as [number, number, number] },
};

async function armChamfer(page: Page): Promise<void> {
  await openEditorDebug(page);
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await openFilletOverflow(page);
  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpKind).toBe("Chamfer");
}

/** The armed chamfer's second-distance field (`=` when equal-leg). */
function d2Field(page: Page) {
  return page.getByLabel("Second distance");
}

/** The arm's authored pairs, off the `?vpdebug` phase surface. */
async function armedPairs(
  page: Page,
): Promise<Array<{ edgeId: string; faceId: string }> | null | undefined> {
  const phases = (await toolPhases(page)) as
    | { edgeOpReferenceFaces?: Array<{ edgeId: string; faceId: string }> | null }
    | null;
  return phases?.edgeOpReferenceFaces;
}

/** The newest projection row's id. */
async function lastFeatureId(page: Page): Promise<string> {
  const id = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { features: Array<{ id: string }> } } };
    };
    const features = w.__stores?.document.getState().features ?? [];
    return features[features.length - 1]?.id ?? null;
  });
  if (!id) throw new Error("the projection has no features");
  return id;
}

test("an EQUAL-LEG chamfer offers no flip control and authors no pair", async ({ page }) => {
  await armChamfer(page);
  // SCHEMA §7.3: an equal-leg chamfer has no reference face at all, and core
  // refuses a record that names one — so there is nothing to flip.
  await expect(page.getByTestId("chip-chamfer-flip")).toHaveCount(0);
  expect(await armedPairs(page)).toBeNull();
});

test("second leg → default pair → flip → commit: the record carries the flipped face", async ({
  page,
}) => {
  await armChamfer(page);
  const before = await getFeatureLabels(page);

  // Authoring the second leg makes the chamfer asymmetric, which is what gives it a
  // reference face — bound by DEFAULT to `adjacentFaces[0]`, the legacy
  // smaller-ordinal face, so the field's arrival moves no geometry.
  await d2Field(page).fill("1");
  await d2Field(page).blur();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpDistance2).toBe(1);
  await expect.poll(async () => (await armedPairs(page))?.length).toBe(1);
  const defaultPair = (await armedPairs(page))![0];

  // The control appears only now: the edge has TWO adjacent faces, so there is a
  // second one to measure on.
  const flip = page.getByTestId("chip-chamfer-flip");
  await expect(flip).toBeVisible();
  await flip.click();

  // Poll the RESULT, not the intent: `chamferFlipped` flips synchronously on the
  // click while the pairs are re-resolved after an awaited client round-trip, and
  // the debug surface publishes both on any tick in between (measured on CI
  // webkit: flag true, pair still the default). `armedPairs` is transiently null
  // while that round-trip is in flight, so the poll must tolerate it.
  await expect
    .poll(async () => (await armedPairs(page))?.[0]?.faceId ?? null)
    .not.toBe(defaultPair.faceId);
  const phases = (await toolPhases(page)) as {
    edgeOpReferenceFlipped?: boolean;
    edgeOpReferenceFaceError?: string | null;
  } | null;
  expect(phases?.edgeOpReferenceFlipped).toBe(true);
  expect(phases?.edgeOpReferenceFaceError ?? null).toBeNull();
  const flippedPair = (await armedPairs(page))![0];
  // SAME contour (keyed by the same edge), DIFFERENT face.
  expect(flippedPair.edgeId).toBe(defaultPair.edgeId);
  expect(flippedPair.faceId).not.toBe(defaultPair.faceId);

  // Commit: the fresh-author path materializes the preview session, so the record
  // gets exactly the op the preview carried — pairs and face inputs included.
  await d2Field(page).press("Enter");
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);

  const stored = await page.evaluate(async (id: string) => {
    const w = window as unknown as {
      __client?: { getOperationParams(recordId: string): Promise<Record<string, unknown>> };
    };
    return (await w.__client?.getOperationParams(id)) ?? null;
  }, await lastFeatureId(page));

  expect(stored).not.toBeNull();
  const pairs = stored!.referenceFaces as Array<{ edgeId: string; faceId: string }>;
  expect(pairs).toHaveLength(1);
  expect(pairs[0]).toEqual(flippedPair);
  // The two keys are LOCKSTEP on the Rust side (same length, same element id, a
  // face primary carrying an anchor) — core refuses a record where they are not.
  const refs = stored!.referenceFaceRefs as Array<{
    primary: { elementId: string; kind: string };
    anchor?: { worldPoint: [number, number, number] };
  }>;
  expect(refs).toHaveLength(1);
  expect(refs[0].primary.kind).toBe("face");
  expect(refs[0].primary.elementId).toBe(pairs[0].faceId);
  expect(refs[0].anchor?.worldPoint).toHaveLength(3);
});

test("clearing the second leg back to `=` drops the pair and hides the flip", async ({ page }) => {
  await armChamfer(page);
  await d2Field(page).fill("1");
  await d2Field(page).blur();
  await expect.poll(async () => (await armedPairs(page))?.length).toBe(1);

  // Back to equal-leg: core refuses a record that still names a reference face, so
  // the pair has to leave the op the moment the asymmetry does.
  await d2Field(page).fill("=");
  await d2Field(page).blur();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpDistance2).toBeNull();
  await expect.poll(async () => await armedPairs(page)).toBeNull();
  await expect(page.getByTestId("chip-chamfer-flip")).toHaveCount(0);
});
