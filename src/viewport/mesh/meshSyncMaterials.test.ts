/*
 * MeshIngest × assigned materials: the pooled-material lane end to end.
 *
 * The load-bearing behaviours here are the TWO-TRACK rule (functional color
 * beats an assigned material in the modeling view, per face), the fact that N
 * bodies wearing one material share ONE pooled instance, and the binding report
 * — which is the only place in the app that knows which body a face override
 * actually landed on.
 *
 * THREE is real (jsdom-safe geometry); the engine, client and material source
 * are fakes, exactly as in `meshSync.test.ts`.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as THREE from "three";
import { MeshIngest } from "./meshSync";
import * as reg from "./meshRegistry";
import { makeBoxMesh, makeCylinderMesh, type FaceColor } from "@/ipc/mockMeshes";
import { documentStore } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import { palette, resetPaletteCache } from "../engine/palette";
import { setBodyMaterialSource } from "../materialSourceBridge";
import type { BodyMaterialSource, PbrMaterialParams } from "../engine/pbrParams";
import type { CadClient } from "@/ipc/client";
import type { DocumentChange } from "@/ipc/types";
import type { ViewportEngine } from "../engine/ViewportEngine";

const tick = () => new Promise((r) => setTimeout(r, 0));

const BASE_PARAMS: PbrMaterialParams = {
  base_color: [0.8, 0.8, 0.8],
  base_metalness: 0,
  base_diffuse_roughness: 0,
  specular_weight: 1,
  specular_color: [1, 1, 1],
  specular_roughness: 0.3,
  specular_roughness_anisotropy: 0,
  specular_ior: 1.5,
  transmission_weight: 0,
  transmission_color: [1, 1, 1],
  transmission_depth: 0,
  coat_weight: 0,
  coat_roughness: 0,
  coat_ior: 1.6,
  coat_color: [1, 1, 1],
  coat_darkening: 1,
  fuzz_weight: 0,
  fuzz_color: [1, 1, 1],
  fuzz_roughness: 0.5,
  emission_color: [1, 1, 1],
  emission_luminance: 0,
  thin_film_weight: 0,
  thin_film_thickness: 0.5,
  thin_film_ior: 1.4,
  subsurface_weight: 0,
  subsurface_color: [0.8, 0.8, 0.8],
  subsurface_radius: 1,
  geometry_opacity: 1,
  geometry_thin_walled: false,
};

const params = (over: Partial<PbrMaterialParams> = {}): PbrMaterialParams => ({
  ...BASE_PARAMS,
  ...over,
});

function fakeEngine() {
  const bodiesRoot = new THREE.Group();
  return {
    bodiesRoot,
    invalidate: vi.fn(),
    refreshHighlights: vi.fn(),
    setHighlightState: vi.fn(),
  } as unknown as ViewportEngine & { bodiesRoot: THREE.Group };
}

function fakeClient(getMesh: ReturnType<typeof vi.fn> = vi.fn(async () => makeBoxMesh())) {
  const listeners = new Set<(c: DocumentChange) => void>();
  const client = {
    getBodyMesh: getMesh,
    onDocumentChanged: (cb: (c: DocumentChange) => void) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  } as unknown as CadClient;
  return { client, getMesh, emit: (c: DocumentChange) => listeners.forEach((l) => l(c)) };
}

/** A scriptable {@link BodyMaterialSource} plus the reports it received. */
function fakeSource() {
  const bodies = new Map<string, { key: string; params: PbrMaterialParams }>();
  const faces = new Map<string, [number, number, number]>();
  const reports: Array<{ bodyId: string; bound: string[]; unbound: string[] }> = [];
  const listeners = new Set<() => void>();

  const source: BodyMaterialSource = {
    poolKeyForBody: (bodyId) => bodies.get(bodyId)?.key ?? null,
    paramsForBody: (bodyId) => bodies.get(bodyId)?.params ?? null,
    faceOverrideBaseColors(_bodyId, faceElementIds) {
      const out: Record<string, [number, number, number]> = {};
      for (const id of faceElementIds) {
        const c = faces.get(id);
        if (c) out[id] = [...c] as [number, number, number];
      }
      return out;
    },
    reportFaceOverrideBindings(bodyId, bound, unbound) {
      reports.push({ bodyId, bound: [...bound], unbound: [...unbound] });
    },
    subscribe(cb) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
  return {
    source,
    reports,
    assign: (bodyId: string, key: string, p: PbrMaterialParams = params()) =>
      bodies.set(bodyId, { key, params: p }),
    unassign: (bodyId: string) => bodies.delete(bodyId),
    overrideFace: (faceId: string, color: [number, number, number]) => faces.set(faceId, color),
    notify: () => [...listeners].forEach((l) => l()),
    lastFor: (bodyId: string) => [...reports].reverse().find((r) => r.bodyId === bodyId),
  };
}

function setBodies(ids: Record<string, boolean>) {
  const full: Record<string, { id: string; name: string; visible: boolean }> = {};
  for (const [id, visible] of Object.entries(ids)) full[id] = { id, name: id, visible };
  documentStore.setState({ bodies: full });
}

const changed = (...bodyIds: string[]): DocumentChange => ({
  revision: 1,
  changedBodies: bodyIds.map((bodyId) => ({ bodyId, meshKey: `${bodyId}:fine:1` })),
  removedBodies: [],
});

function faceMaterialOf(engine: ReturnType<typeof fakeEngine>, bodyId: string) {
  const group = engine.bodiesRoot.children.find((c) => c.userData.bodyId === bodyId) as THREE.Group;
  const mesh = group.children.find((c) => c.userData.kind === "face") as THREE.Mesh;
  return mesh.material as THREE.MeshStandardMaterial;
}

/** The baked rgb triple of de-indexed vertex `i` (face `f` occupies 6·f … 6·f+5). */
function vertexColor(bodyId: string, i: number): number[] {
  const attr = reg.getEntry(bodyId)!.geometry.getAttribute("color");
  return [attr.array[i * 3], attr.array[i * 3 + 1], attr.array[i * 3 + 2]];
}

/**
 * A Float32-rounded triple. The BAKED attribute is a Float32Array, so a linear
 * color only compares equal to it after the same rounding; a `THREE.Color`'s
 * channels are plain JS numbers and are compared raw.
 */
const f32 = (c: readonly [number, number, number]) => [...Float32Array.from(c)];
const rgb = (m: THREE.Material & { color: THREE.Color }) => [m.color.r, m.color.g, m.color.b];

function setTheme(theme: "light" | "dark") {
  document.documentElement.dataset.theme = theme;
  resetPaletteCache();
}

let ingest: MeshIngest | null = null;

beforeEach(() => {
  reg.disposeAll();
  reg.__resetRegistryForTests();
});
afterEach(() => {
  ingest?.detach();
  ingest = null;
  setBodyMaterialSource(null);
  setTheme("light");
  toolStore.getState().setMode("model");
  viewportStore.setState({ isolatedBodyIds: null });
  settingsStore.setState({ displayMode: "shadedEdges" });
});

/** Attach an ingest with `source` already live and one box body loaded. */
async function loaded(
  source: BodyMaterialSource,
  bodies: Record<string, boolean> = { body1: true },
  mesh?: ArrayBuffer,
) {
  setBodies(bodies);
  const engine = fakeEngine();
  const { client, emit } = fakeClient(vi.fn(async () => mesh ?? makeBoxMesh()));
  ingest = new MeshIngest();
  ingest.attach(engine, client);
  ingest.setMaterialSource(source);
  emit(changed(...Object.keys(bodies)));
  await tick();
  return { engine, emit };
}

describe("MeshIngest assigned materials", () => {
  it("draws an assigned body with a pooled material, not the shared library one", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111", params({ base_color: [0.2, 0.4, 0.6], base_metalness: 1 }));
    const { engine } = await loaded(src.source);

    const mat = faceMaterialOf(engine, "body1");
    expect(rgb(mat)).toEqual([0.2, 0.4, 0.6]);
    expect(mat.metalness).toBe(1);
    // No overrides and no functional color ⇒ no bake, geometry stays indexed.
    expect(reg.getEntry("body1")!.hasVertexColors).toBe(false);
    expect(mat.vertexColors).toBe(false);
  });

  it("two bodies wearing the same material share ONE instance", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111");
    src.assign("body2", "mat_a:1111");
    const { engine } = await loaded(src.source, { body1: true, body2: true });

    expect(faceMaterialOf(engine, "body2")).toBe(faceMaterialOf(engine, "body1"));
  });

  it("a body with no material keeps the library look, unchanged", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111");
    const { engine } = await loaded(src.source, { body1: true, body2: true });

    const unassigned = faceMaterialOf(engine, "body2");
    expect(unassigned.color.getHex()).toBe(palette.bodyNeutral().getHex());
    expect(unassigned).not.toBe(faceMaterialOf(engine, "body1"));
  });

  it("restores the library material when the assignment is removed", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111", params({ base_color: [0.2, 0.4, 0.6] }));
    const { engine } = await loaded(src.source);
    expect(faceMaterialOf(engine, "body1").color.getHex()).not.toBe(
      palette.bodyNeutral().getHex(),
    );

    src.unassign("body1");
    src.notify();

    expect(faceMaterialOf(engine, "body1").color.getHex()).toBe(palette.bodyNeutral().getHex());
  });

  it("re-pools when the material EDITS (a new hash ⇒ a new key)", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111", params({ base_color: [0.2, 0.4, 0.6] }));
    const { engine } = await loaded(src.source);
    const before = faceMaterialOf(engine, "body1");

    src.assign("body1", "mat_a:2222", params({ base_color: [0.9, 0.1, 0.1] }));
    src.notify();

    const after = faceMaterialOf(engine, "body1");
    expect(after).not.toBe(before);
    expect(rgb(after)).toEqual([0.9, 0.1, 0.1]);
    // The superseded material is swept, not leaked.
    expect(before.dispose).toBeTruthy();
  });
});

describe("MeshIngest face overrides", () => {
  it("bakes an override color onto its face, over the material's base elsewhere", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111", params({ base_color: [0.2, 0.4, 0.6] }));
    src.overrideFace("f:1", [0.9, 0.1, 0.1]);
    const { engine } = await loaded(src.source);

    const entry = reg.getEntry("body1")!;
    // The override forced the vertex-colored layout on a body that had no
    // functional color at all.
    expect(entry.hasVertexColors).toBe(true);
    expect(vertexColor("body1", 6)).toEqual(f32([0.9, 0.1, 0.1])); // f:1
    // Unset faces fall back to the BODY material's base, not the theme token.
    expect(vertexColor("body1", 0)).toEqual(f32([0.2, 0.4, 0.6])); // f:0

    // …and the face material is the `:vc` twin: white base, reading the attribute.
    const mat = faceMaterialOf(engine, "body1");
    expect(mat.vertexColors).toBe(true);
    expect(mat.color.getHex()).toBe(new THREE.Color(1, 1, 1).getHex());
  });

  /*
   * THE TWO-TRACK RULE. Functional color (what the user painted, or what a STEP
   * file authored) is information; a material is appearance. In the MODELING
   * view the information wins, per face — otherwise assigning a material would
   * silently erase a deliberate colour-coding.
   */
  it("lets a FUNCTIONAL face color outrank an override on the same face", async () => {
    const RED: FaceColor = [214, 74, 62, 255];
    const src = fakeSource();
    src.assign("body1", "mat_a:1111", params({ base_color: [0.2, 0.4, 0.6] }));
    src.overrideFace("f:0", [0, 1, 0]); // same face the mesh authored red
    src.overrideFace("f:1", [0, 1, 0]); // …and one it did not
    await loaded(
      src.source,
      { body1: true },
      makeBoxMesh(40, 40, 40, 0, [0, 0, 0], [RED, null, null, null, null, null]),
    );

    const authored = new THREE.Color().setRGB(
      RED[0] / 255,
      RED[1] / 255,
      RED[2] / 255,
      THREE.SRGBColorSpace,
    );
    expect(vertexColor("body1", 0)).toEqual(f32([authored.r, authored.g, authored.b]));
    expect(vertexColor("body1", 6)).toEqual(f32([0, 1, 0]));
  });

  it("reports the override as BOUND when the mesh has that face", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111");
    src.overrideFace("f:1", [0.9, 0.1, 0.1]);
    await loaded(src.source);

    expect(src.lastFor("body1")).toEqual({ bodyId: "body1", bound: ["f:1"], unbound: [] });
  });

  /*
   * The H5-B shape, in the appearance lane: a regen renumbers the topology and
   * an override that used to name a real face names nothing. Reporting it as
   * UNBOUND — rather than deleting the assignment, or silently binding it to
   * whatever now carries that id — is the whole point.
   */
  it("reports an override as UNBOUND once its face leaves the tessellation", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111");
    src.overrideFace("f:4", [0.9, 0.1, 0.1]);

    setBodies({ body1: true });
    const engine = fakeEngine();
    let mesh = makeBoxMesh(); // f:0..f:5
    const { client, emit } = fakeClient(vi.fn(async () => mesh));
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    ingest.setMaterialSource(src.source);
    emit(changed("body1"));
    await tick();
    expect(src.lastFor("body1")).toEqual({ bodyId: "body1", bound: ["f:4"], unbound: [] });

    mesh = makeCylinderMesh(); // f:0..f:2 — f:4 is gone
    emit(changed("body1"));
    await tick();

    expect(src.lastFor("body1")).toEqual({ bodyId: "body1", bound: [], unbound: ["f:4"] });
    // The assignment is untouched; only the binding evidence changed.
    expect(reg.getEntry("body1")!.hasVertexColors).toBe(false);
  });

  it("retracts the report when the body leaves the document", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111");
    src.overrideFace("f:1", [0.9, 0.1, 0.1]);
    const { emit } = await loaded(src.source);
    expect(src.lastFor("body1")!.bound).toEqual(["f:1"]);

    emit({ revision: 2, changedBodies: [], removedBodies: ["body1"] });

    expect(src.lastFor("body1")).toEqual({ bodyId: "body1", bound: [], unbound: [] });
  });

  it("re-bakes in place when an override's color changes", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111");
    src.overrideFace("f:1", [0.9, 0.1, 0.1]);
    await loaded(src.source);
    const attr = reg.getEntry("body1")!.geometry.getAttribute("color");

    src.overrideFace("f:1", [0, 0, 1]);
    src.notify();

    expect(vertexColor("body1", 6)).toEqual(f32([0, 0, 1]));
    // IN PLACE: the same attribute object, rewritten — never a new one, which
    // would strand the highlight clones that share it by reference.
    expect(reg.getEntry("body1")!.geometry.getAttribute("color")).toBe(attr);
  });
});

describe("MeshIngest material state mechanics", () => {
  it("dims pooled materials in sketch mode alongside the library ones", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111");
    const { engine } = await loaded(src.source);
    const mat = faceMaterialOf(engine, "body1");

    toolStore.getState().setMode("sketch");
    expect(mat.opacity).toBe(0.35);
    expect(mat.transparent).toBe(true);

    toolStore.getState().setMode("model");
    expect(mat.opacity).toBe(1);
  });

  /*
   * The theming invariant, on the seam between the two lanes: a theme flip must
   * move the TOKEN fallback of an unmaterialled body and move nothing at all on
   * a materialled one. Getting this backwards either freezes imported bodies in
   * the old theme or repaints a user's material with a UI colour.
   */
  it("theme refresh re-bakes token fallbacks but leaves material colors alone", async () => {
    const RED: FaceColor = [214, 74, 62, 255];
    const colored = makeBoxMesh(40, 40, 40, 0, [0, 0, 0], [RED, null, null, null, null, null]);
    const src = fakeSource();
    src.assign("body1", "mat_a:1111", params({ base_color: [0.2, 0.4, 0.6] }));
    src.overrideFace("f:1", [0, 1, 0]);

    setBodies({ body1: true, body2: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient(
      vi.fn(async (id: string) => (id === "body1" ? makeBoxMesh() : colored)),
    );
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    ingest.setMaterialSource(src.source);
    emit(changed("body1", "body2"));
    await tick();

    const lightNeutral = f32([
      palette.bodyNeutral().r,
      palette.bodyNeutral().g,
      palette.bodyNeutral().b,
    ]);
    expect(vertexColor("body2", 6)).toEqual(lightNeutral); // unset face, token
    expect(vertexColor("body1", 0)).toEqual(f32([0.2, 0.4, 0.6])); // material base

    setTheme("dark");
    ingest.refreshColors();

    const darkNeutral = f32([
      palette.bodyNeutral().r,
      palette.bodyNeutral().g,
      palette.bodyNeutral().b,
    ]);
    expect(darkNeutral).not.toEqual(lightNeutral); // non-vacuity: the token moved
    expect(vertexColor("body2", 6)).toEqual(darkNeutral); // …and followed it
    expect(vertexColor("body1", 0)).toEqual(f32([0.2, 0.4, 0.6])); // data, unmoved
    expect(vertexColor("body1", 6)).toEqual(f32([0, 1, 0])); // override, unmoved
  });

  it("picks up a source registered on the bridge BEFORE the ingest attaches", async () => {
    const src = fakeSource();
    src.assign("body1", "mat_a:1111", params({ base_color: [0.2, 0.4, 0.6] }));
    setBodyMaterialSource(src.source);

    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit(changed("body1"));
    await tick();

    expect(faceMaterialOf(engine, "body1").color.r).toBeCloseTo(0.2, 5);
  });

  it("picks up a source registered AFTER the ingest attaches", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit(changed("body1"));
    await tick();
    expect(faceMaterialOf(engine, "body1").color.getHex()).toBe(palette.bodyNeutral().getHex());

    const src = fakeSource();
    src.assign("body1", "mat_a:1111", params({ base_color: [0.2, 0.4, 0.6] }));
    setBodyMaterialSource(src.source);

    expect(faceMaterialOf(engine, "body1").color.r).toBeCloseTo(0.2, 5);
  });
});
