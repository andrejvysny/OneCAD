import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  CANVAS,
  bodyOptions,
  clickAt,
  clickAtClient,
  commitExtrudeAtHandle,
  dofPill,
  enterSketchViaPlanePicker,
  findFaceOnBody,
  getSketchSnapshot,
  hideSeedSketches,
  openEditorDebug,
  planePointToClient,
  selectSketchTool,
  waitForCameraSettled,
} from "./helpers";

/*
 * RENDER MATERIALS (OpenPBR P0+P1) — the assignment flows, in a real browser.
 *
 * Everything below the UI is already pinned by vitest (the data model, the
 * serializer, `materialQuery`, `overridePolicy`, `BodyMaterialPool`,
 * `openPbrToThree`). What no unit test can prove is the part this spec exists
 * for: that the panel, the inspector, the module's store, the document
 * transaction path and the LIVE three.js scene are all wired to each other —
 * i.e. that clicking "Steel" and picking it in a dropdown actually changes the
 * material a body is drawn with, and that two bodies wearing it are drawn with
 * ONE material instance rather than two identical ones.
 *
 * The viewport is a WebGL canvas with no per-entity DOM, so the rendering
 * assertions read the scene itself through the engine's `?vpdebug` handle
 * (`window.__vpEngine.bodiesRoot`) — the same white-box surface the other specs
 * raycast through — and check the face mesh's material: OpenPBR
 * `base_metalness`/`specular_roughness` land on three's `metalness`/`roughness`
 * (`openPbrToThree.ts`), and Steel (1 / 0.35) is unmistakably not the
 * design-token body look (`bodyMaterials.ts` — 0 / 0.5).
 *
 * ── WHAT THIS LANE CANNOT PROVE, and why it is not tested here ───────────────
 * The keep/replace prompt (`OverrideKeepDialog`) fires only for face overrides
 * the VIEWPORT reported as BOUND (`overridePolicy.ts` — prompting about faces
 * that are not on screen would be a lie). Binding needs the mesh's face ids to
 * BE the persistent ElementIds an override is keyed by, and the mock lane's
 * meshes carry snapshot TopoKeys (`mockMeshes.ts` never sets
 * `IDS_HAVE_ELEMENTIDS`), while `promoteSelection` mints `el_…` ids. So in this
 * lane no override ever binds, the prompt can never be raised by a genuine
 * gesture, and an Alt-drop on a face routes to the body by design
 * (`assignDragDrop.ts`). Rather than fake a binding, the prompt stays covered
 * by `overridePolicy.test.ts` + `RenderDialogHost.test.tsx`, and the third test
 * below pins what the mock lane DOES prove about a face override — that it is
 * authored, persisted and shown — plus the absence of a binding, so the day the
 * mock lane grows ElementId meshes this spec says so out loud.
 */

// ── Scene probes ─────────────────────────────────────────────────────────────

/** What a body's face mesh is actually being drawn with, straight off the scene. */
interface FaceMaterialSnap {
  uuid: string;
  type: string;
  metalness: number;
  roughness: number;
}

/**
 * The face mesh material of `bodyId` in `bodiesRoot` (`?vpdebug` — see
 * `openEditorDebug`). `null` while the body has no scene object yet, which is a
 * legitimate transient: mesh ingest is async, so every caller polls.
 *
 * Walks the group/`userData` shape `BodyObject.buildBodyObject` builds
 * (`body:<id>` group → one `kind === "face"` Mesh) rather than assuming a child
 * order — the edge `LineSegments2` is a sibling of the face mesh.
 */
async function faceMaterial(page: Page, bodyId: string): Promise<FaceMaterialSnap | null> {
  return page.evaluate((want) => {
    const engine = (
      window as unknown as {
        __vpEngine?: {
          bodiesRoot: {
            children: Array<{
              userData?: { bodyId?: string };
              children: Array<{
                userData?: { kind?: string };
                material?: { uuid: string; type: string; metalness: number; roughness: number };
              }>;
            }>;
          };
        };
      }
    ).__vpEngine;
    if (!engine) return null;
    for (const group of engine.bodiesRoot.children) {
      if (group.userData?.bodyId !== want) continue;
      for (const child of group.children) {
        const mat = child.material;
        if (child.userData?.kind !== "face" || !mat) continue;
        return {
          uuid: mat.uuid,
          type: mat.type,
          metalness: mat.metalness,
          roughness: mat.roughness,
        };
      }
    }
    return null;
  }, bodyId);
}

/** Poll until `bodyId` has a face mesh in the scene, and return its material. */
async function settledFaceMaterial(page: Page, bodyId: string): Promise<FaceMaterialSnap> {
  await expect
    .poll(async () => (await faceMaterial(page, bodyId)) !== null, {
      message: `body ${bodyId} never got a face mesh`,
    })
    .toBe(true);
  return (await faceMaterial(page, bodyId)) as FaceMaterialSnap;
}

/** The id of the primary selected entity (`__stores.selection`, dev-only). */
async function selectedId(page: Page): Promise<string> {
  const id = await page.evaluate(
    () =>
      (
        window as unknown as {
          __stores?: { selection: { getState(): { selected: Array<{ id: string }> } } };
        }
      ).__stores?.selection.getState().selected[0]?.id ?? null,
  );
  if (id === null) throw new Error("nothing is selected");
  return id;
}

// ── Chrome ───────────────────────────────────────────────────────────────────

/**
 * The three left-sidebar occupants share one footprint through
 * `sidebarTabStore`, so the Bodies listbox (Model) and the material library
 * (Materials) are never on screen at the same time — a flow that assigns from
 * the tree and then reads a usage count has to hop between them.
 */
async function openMaterialsTab(page: Page): Promise<void> {
  await page.getByTestId("sidebar-tab-materials").click();
  await expect(page.getByTestId("starter-material-Steel")).toBeVisible();
}

async function openModelTab(page: Page): Promise<void> {
  await page.getByTestId("sidebar-tab-model").click();
  await expect(page.getByRole("listbox", { name: "Bodies" })).toBeVisible();
}

/** A document material's row, found by NAME — its id is minted per document. */
function libraryRow(page: Page, name: string) {
  return page.locator('[data-testid^="material-row-"]').filter({ hasText: name });
}

/** The row's assignment count (bodies + faces wearing it). */
function usageBadge(page: Page, name: string) {
  return libraryRow(page, name).locator('[data-testid^="material-usage-"]');
}

/** Click-to-add a starter template, which mints a fresh document material. */
async function addStarter(page: Page, name: string): Promise<void> {
  await page.getByTestId(`starter-material-${name}`).click();
  await expect(libraryRow(page, name)).toBeVisible();
}

/** Select a body in the tree and return its id (never assumed from position). */
async function selectBody(page: Page, index: number): Promise<string> {
  await bodyOptions(page).nth(index).click();
  await expect(page.getByTestId("material-select-body-material")).toBeVisible();
  return selectedId(page);
}

async function assignBodyMaterial(page: Page, label: string): Promise<void> {
  await page.getByTestId("material-select-body-material").selectOption({ label });
}

/**
 * Draw a rectangle and extrude it — the same profile-first arm every other
 * model-mode spec uses (`variables.spec.ts` / `model-undo.spec.ts`), copied
 * rather than shared because each spec's version differs in what it hides
 * first. Returns nothing: the caller picks the new body out of the tree.
 */
async function drawAndExtrude(page: Page): Promise<void> {
  // The region click must hit the new profile, not an occluding seeded face.
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);

  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  const snap = await getSketchSnapshot(page);
  const p0 = snap.lines[0]?.p0;
  const p2 = snap.lines[2]?.p0;
  if (!p0 || !p2) throw new Error("rectangle snapshot is incomplete");
  const centroid = { x: (p0[0] + p2[0]) / 2, y: (p0[1] + p2[1]) / 2 };

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  const client = await planePointToClient(page, snap.plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          Boolean(
            (
              window as unknown as {
                __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown };
              }
            ).__vpEngine?.sketchStaticHitTest(x, y),
          ),
        client,
      ),
    )
    .toBe(true);
  await clickAtClient(page, client.x, client.y);
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await commitExtrudeAtHandle(page);
  await expect(bodyOptions(page)).toHaveCount(2);
}

// ── Specs ────────────────────────────────────────────────────────────────────

test("a starter material assigned to a body reaches the viewport and the library count", async ({
  page,
}) => {
  // `?vpdemo` drives the mock box through the full ingest path, so there is a
  // real body mesh in the scene to read a material off.
  await openEditorDebug(page, { mockBody: true });
  await waitForCameraSettled(page);

  await openMaterialsTab(page);
  // An untouched document has no render slice at all (ADR-0004 skip-if-empty).
  await expect(page.getByTestId("material-library-empty")).toBeVisible();

  await addStarter(page, "Steel");
  await expect(page.getByTestId("material-library-empty")).toHaveCount(0);
  // Added, not assigned: a starter click writes the LIBRARY only.
  await expect(usageBadge(page, "Steel")).toHaveText("0");

  await openModelTab(page);
  const bodyId = await selectBody(page, 0);
  const before = await settledFaceMaterial(page, bodyId);
  expect(before.metalness).toBe(0); // the design-token body look
  expect(before.roughness).toBe(0.5);

  await assignBodyMaterial(page, "Steel");

  // Steel is base_metalness 1 / specular_roughness 0.35 (`starterMaterials.ts`),
  // and stays on the CHEAP tier — no coat, no transmission, spec-default
  // specular — so the pooled material is a plain MeshStandardMaterial.
  await expect.poll(async () => (await faceMaterial(page, bodyId))?.metalness).toBe(1);
  const after = await settledFaceMaterial(page, bodyId);
  expect(after.roughness).toBe(0.35);
  expect(after.type).toBe("MeshStandardMaterial");
  expect(after.uuid).not.toBe(before.uuid);

  await openMaterialsTab(page);
  await expect(usageBadge(page, "Steel")).toHaveText("1");
});

test("two bodies wearing one material share a single three.js material instance", async ({
  page,
}) => {
  // `?vpdemo=cyl` publishes the bored bushing BESIDE the box, so there are two
  // independently ingested bodies — the only way to prove the pool hands out one
  // instance rather than one per body.
  await page.goto("/?vpdebug&vpdemo=cyl");
  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible();
  await waitForCameraSettled(page);
  await expect(bodyOptions(page)).toHaveCount(2);

  await openMaterialsTab(page);
  await addStarter(page, "Brass");

  await openModelTab(page);
  const firstId = await selectBody(page, 0);
  await settledFaceMaterial(page, firstId);
  await assignBodyMaterial(page, "Brass");
  await expect.poll(async () => (await faceMaterial(page, firstId))?.metalness).toBe(1);

  const secondId = await selectBody(page, 1);
  expect(secondId).not.toBe(firstId);
  await settledFaceMaterial(page, secondId);
  await assignBodyMaterial(page, "Brass");
  await expect.poll(async () => (await faceMaterial(page, secondId))?.metalness).toBe(1);

  // ONE program, one uniform upload: `BodyMaterialPool.acquire` is keyed by
  // `${materialId}:${resolvedMaterialHash}` and returns the same instance.
  const a = await settledFaceMaterial(page, firstId);
  const b = await settledFaceMaterial(page, secondId);
  expect(b.uuid).toBe(a.uuid);

  await openMaterialsTab(page);
  await expect(usageBadge(page, "Brass")).toHaveText("2");
});

test("a face override is authored from the inspector over an inherited body material", async ({
  page,
}) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForCameraSettled(page);

  await openMaterialsTab(page);
  await addStarter(page, "Steel");
  await addStarter(page, "Brass");

  await openModelTab(page);
  const bodyId = await selectBody(page, 0);
  await settledFaceMaterial(page, bodyId);
  await assignBodyMaterial(page, "Steel");
  await expect.poll(async () => (await faceMaterial(page, bodyId))?.metalness).toBe(1);

  // A GENUINE face pick, through the engine's own raycast — the inspector's
  // face section is gated on a promoted ElementId (`subElement`), which only a
  // real pick + `promoteSelection` produces.
  const face = await findFaceOnBody(page, bodyId);
  await clickAtClient(page, face.x, face.y);
  const effective = page.getByTestId("material-face-effective");
  // Before an override exists the face reports what it INHERITS, by name.
  await expect(effective).toHaveText("From body: Steel");

  await page.getByTestId("material-select-face-override").selectOption({ label: "Brass" });
  await expect(effective).toHaveText("Override: Brass");

  await openMaterialsTab(page);
  await expect(usageBadge(page, "Steel")).toHaveText("1");
  await expect(usageBadge(page, "Brass")).toHaveText("1");

  // …and the override does NOT bind in this lane, so the body reports none and
  // the keep/replace prompt is unreachable here. See the header note: the mock's
  // mesh face ids are TopoKeys, the override is keyed by the promoted ElementId.
  // If this ever goes red, the mock lane grew ElementId meshes and the prompt
  // became e2e-testable — which is a reason to extend this spec, not to relax it.
  await openModelTab(page);
  await selectBody(page, 0);
  await expect(page.getByTestId("material-override-count")).toHaveCount(0);
});

test("undo reverts a material assignment, redo puts it back", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForCameraSettled(page);

  await openMaterialsTab(page);
  await addStarter(page, "Steel");
  await openModelTab(page);
  const bodyId = await selectBody(page, 0);
  const neutral = await settledFaceMaterial(page, bodyId);

  await assignBodyMaterial(page, "Steel");
  await expect.poll(async () => (await faceMaterial(page, bodyId))?.metalness).toBe(1);

  // The assignment went through `documentState` → the backend's transaction
  // path, so the ORDINARY model-mode undo chord reverts it — there is no second,
  // module-private mutation lane that skips history (docs/ARCHITECTURE.md §9).
  await page.keyboard.press("Meta+z");
  await expect(page.getByTestId("material-select-body-material")).toHaveValue("");
  await expect.poll(async () => (await faceMaterial(page, bodyId))?.metalness).toBe(0);
  await expect.poll(async () => (await faceMaterial(page, bodyId))?.uuid).toBe(neutral.uuid);

  // Undo took back the ASSIGNMENT, not the library entry the starter click added
  // — those were two separate writes.
  await openMaterialsTab(page);
  await expect(libraryRow(page, "Steel")).toBeVisible();
  await expect(usageBadge(page, "Steel")).toHaveText("0");

  await page.keyboard.press("Meta+Shift+z");
  await expect(usageBadge(page, "Steel")).toHaveText("1");
  await expect.poll(async () => (await faceMaterial(page, bodyId))?.metalness).toBe(1);
});

test("unassigning returns the body to the library look", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForCameraSettled(page);

  await openMaterialsTab(page);
  await addStarter(page, "Steel");
  await openModelTab(page);
  const bodyId = await selectBody(page, 0);
  const neutral = await settledFaceMaterial(page, bodyId);

  await assignBodyMaterial(page, "Steel");
  await expect.poll(async () => (await faceMaterial(page, bodyId))?.metalness).toBe(1);
  const pooled = await settledFaceMaterial(page, bodyId);

  // "None" is a real answer, not an empty state: it clears the assignment, the
  // body goes back to the shared token material, and the pooled one is swept.
  await assignBodyMaterial(page, "None");
  await expect.poll(async () => (await faceMaterial(page, bodyId))?.uuid).toBe(neutral.uuid);
  const back = await settledFaceMaterial(page, bodyId);
  expect(back.metalness).toBe(0);
  expect(back.uuid).not.toBe(pooled.uuid);

  await openMaterialsTab(page);
  await expect(usageBadge(page, "Steel")).toHaveText("0");
});

test("a body extruded in this session takes a material like any other", async ({ page }) => {
  await openEditorDebug(page);
  await waitForCameraSettled(page);
  await drawAndExtrude(page);

  await openMaterialsTab(page);
  await addStarter(page, "Copper");
  await openModelTab(page);

  // The freshly committed body is the second row; its id is read back rather
  // than guessed, because `nextBodyId` is the mock's business.
  const bodyId = await selectBody(page, 1);
  await settledFaceMaterial(page, bodyId);
  await assignBodyMaterial(page, "Copper");
  await expect.poll(async () => (await faceMaterial(page, bodyId))?.metalness).toBe(1);
  const after = await settledFaceMaterial(page, bodyId);
  expect(after.roughness).toBe(0.35);
});
