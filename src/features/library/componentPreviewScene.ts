/*
 * componentPreviewScene — turning a component's MESH1 into something you can
 * look at (WP-B6).
 *
 * Two consumers, one geometry path:
 *   - `componentThumbnail()` rasterizes a component ONCE into a data URL for
 *     the library cards. One shared offscreen renderer does every card, because
 *     a WebGL context per card is how a browser runs out of them (the limit is
 *     ~16); a catalog can hold hundreds.
 *   - `buildPreviewObject()` hands the same geometry to a LIVE canvas for the
 *     detail view, where the user can orbit it.
 *
 * Colors come from `palette` — the viewport's own token-resolved colors — so a
 * preview reads as the same material as the real body, and re-themes with it.
 * No raw hex anywhere (the hex gate is a repo invariant).
 *
 * EVERY failure degrades to `null`, never a throw: a component that cannot be
 * previewed still has to be listed, searchable and placeable. A card falls back
 * to its icon, which is exactly what it showed before this existed.
 */
import * as THREE from "three";
import { createClient } from "@/ipc/client";
import { parseMeshPayload, type BodyMeshView } from "@/viewport/mesh/parseMeshPayload";
import { palette } from "@/viewport/engine/palette";
import type { ComponentParamValue } from "@/ipc/types";

/** `"<id>@<version>"`, plus the params when a caller previews a configured size. */
export function previewKey(
  id: string,
  version: string,
  params?: Record<string, ComponentParamValue>,
): string {
  const suffix =
    params && Object.keys(params).length > 0
      ? `#${Object.entries(params)
          .sort(([a], [b]) => a.localeCompare(b))
          .map(([k, v]) => `${k}=${String(v)}`)
          .join(",")}`
      : "";
  return `${id}@${version}${suffix}`;
}

/**
 * A `THREE.Mesh` for one parsed body, in WORLD coordinates (Z-up, verbatim —
 * the kernel already produces Z-up geometry and rotating it here would be the
 * same corruption the viewport's own hard invariant forbids).
 *
 * The caller owns disposal of `geometry`; the material is shared and must not
 * be disposed per object.
 */
export function buildPreviewObject(view: BodyMeshView): THREE.Mesh {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(view.positions, 3));
  if (view.normals) {
    geometry.setAttribute("normal", new THREE.BufferAttribute(view.normals, 3));
  }
  geometry.setIndex(new THREE.BufferAttribute(view.indices, 1));
  if (!view.normals) geometry.computeVertexNormals();
  geometry.computeBoundingSphere();
  return new THREE.Mesh(geometry, previewMaterial());
}

let material: THREE.MeshStandardMaterial | null = null;
function previewMaterial(): THREE.MeshStandardMaterial {
  material ??= new THREE.MeshStandardMaterial({
    color: palette.bodyNeutral(),
    roughness: 0.55,
    metalness: 0.1,
    flatShading: false,
  });
  return material;
}

/**
 * Frames `object` in `camera` from a fixed three-quarter view — the angle a
 * catalog thumbnail wants, and the one the detail view starts at.
 *
 * The distance comes from the bounding sphere and the camera's own FOV rather
 * than a constant, so an M2 washer and an M12 bolt both fill the frame instead
 * of one being a speck.
 */
export function frameObject(camera: THREE.PerspectiveCamera, object: THREE.Object3D): void {
  const box = new THREE.Box3().setFromObject(object);
  if (box.isEmpty()) return;
  const sphere = box.getBoundingSphere(new THREE.Sphere());
  const fov = (camera.fov * Math.PI) / 180;
  // 1.35 leaves a margin so the silhouette never touches the frame edge.
  const distance = (sphere.radius * 1.35) / Math.sin(fov / 2);
  const dir = new THREE.Vector3(1, -1, 0.75).normalize();
  camera.position.copy(sphere.center).addScaledVector(dir, distance);
  camera.up.set(0, 0, 1); // world is Z-up
  camera.lookAt(sphere.center);
  camera.near = Math.max(distance / 100, 0.01);
  camera.far = distance * 10;
  camera.updateProjectionMatrix();
}

/** Two directional lights plus ambient — enough to read a machined shape. */
export function addPreviewLights(scene: THREE.Scene): void {
  scene.add(new THREE.AmbientLight(0xffffff, 1.4));
  const key = new THREE.DirectionalLight(0xffffff, 2.0);
  key.position.set(1, -1, 1.4);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xffffff, 0.7);
  fill.position.set(-1, 0.6, 0.4);
  scene.add(fill);
}

// ── The shared offscreen rasterizer (card thumbnails) ───────────────────────

interface Offscreen {
  renderer: THREE.WebGLRenderer;
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
}

let offscreen: Offscreen | null | undefined;

/**
 * The one shared offscreen renderer, or `null` where WebGL is unavailable
 * (jsdom, a headless browser without a GL backend, a context-loss storm).
 * `undefined` means "not tried yet"; `null` means "tried and cannot" — the
 * distinction keeps a failed init from being retried on every card.
 */
function offscreenRenderer(): Offscreen | null {
  if (offscreen !== undefined) return offscreen;
  try {
    const canvas = document.createElement("canvas");
    const renderer = new THREE.WebGLRenderer({
      canvas,
      alpha: true,
      antialias: true,
      preserveDrawingBuffer: true, // required: `toDataURL` reads the buffer back
    });
    renderer.setPixelRatio(1); // a thumbnail is rasterized at its own size
    // A lost context must NOT latch the `null` terminal state: the browser
    // reclaims offscreen contexts under GPU pressure, and every future
    // thumbnail for the session would silently fail. Reset to `undefined`
    // ("not tried yet") so the next rasterize builds a fresh context; the
    // already-rasterized data URLs in the cache stay valid.
    canvas.addEventListener("webglcontextlost", (e) => {
      e.preventDefault();
      offscreen?.renderer.dispose();
      offscreen = undefined;
    });
    const scene = new THREE.Scene();
    addPreviewLights(scene);
    offscreen = { renderer, scene, camera: new THREE.PerspectiveCamera(35, 1, 0.1, 1000) };
  } catch {
    offscreen = null;
  }
  return offscreen;
}

/** Rasterizes one parsed body to a PNG data URL, or `null` when WebGL is absent. */
export function rasterize(view: BodyMeshView, size = 256): string | null {
  const ctx = offscreenRenderer();
  if (!ctx) return null;
  const mesh = buildPreviewObject(view);
  try {
    ctx.renderer.setSize(size, size, false);
    ctx.camera.aspect = 1;
    ctx.scene.add(mesh);
    frameObject(ctx.camera, mesh);
    ctx.renderer.render(ctx.scene, ctx.camera);
    return ctx.renderer.domElement.toDataURL("image/png");
  } catch {
    return null;
  } finally {
    ctx.scene.remove(mesh);
    mesh.geometry.dispose();
  }
}

/** Resolved thumbnails, and the in-flight requests that will become them. */
const thumbnails = new Map<string, string | null>();
const inFlight = new Map<string, Promise<string | null>>();

/**
 * The component's thumbnail as a data URL, fetched and rasterized once.
 *
 * Concurrent callers for the same component share one request — a grid mounts
 * every card at once, and without this a 30-component catalog would ask the
 * worker to build the same screw 30 times.
 */
export async function componentThumbnail(
  id: string,
  version: string,
  params?: Record<string, ComponentParamValue>,
  size = 256,
): Promise<string | null> {
  const key = previewKey(id, version, params);
  const cached = thumbnails.get(key);
  if (cached !== undefined) return cached;
  const pending = inFlight.get(key);
  if (pending) return pending;

  const request = (async () => {
    try {
      const bodies = await createClient().componentPreviewMesh(id, version, params);
      const first = bodies[0];
      if (!first) return null;
      return rasterize(parseMeshPayload(first.mesh), size);
    } catch {
      // A component the kernel cannot build, a worker that is not up, a
      // malformed mesh: all the same answer here. The card shows its icon.
      return null;
    }
  })();
  inFlight.set(key, request);
  const url = await request;
  inFlight.delete(key);
  thumbnails.set(key, url);
  return url;
}

/**
 * Test/theme seam: drop every cached thumbnail and the shared material, so the
 * next read re-renders. The material holds a token color, which changes when
 * the theme does.
 */
export function resetComponentPreviews(): void {
  thumbnails.clear();
  inFlight.clear();
  material?.dispose();
  material = null;
}
