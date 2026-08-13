/*
 * ComponentPreview3D — the LIVE, orbitable preview of one component (WP-B6).
 *
 * Its own tiny Three.js scene rather than the app's `ViewportEngine`: that
 * engine owns the document's bodies, its picking, its grid and its camera
 * state, and a component browser has none of those. Borrowing it would mean
 * either mutating the open document's scene to show a catalog item, or
 * standing up a second engine instance with everything switched off. A ~120
 * line scene is the smaller, honester thing.
 *
 * ONE WebGL context, mounted only while the detail view is open — the card grid
 * uses the shared offscreen rasterizer (`componentPreviewScene`) precisely so
 * that this is the only live context in the library UI.
 */
import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { createClient } from "@/ipc/client";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { palette } from "@/viewport/engine/palette";
import type { ComponentParamValue } from "@/ipc/types";
import {
  addPreviewLights,
  buildPreviewObject,
  frameObject,
  previewKey,
} from "./componentPreviewScene";

export interface ComponentPreview3DProps {
  componentId: string;
  componentVersion: string;
  params?: Record<string, ComponentParamValue>;
  /** Square edge in CSS pixels. */
  size?: number;
}

type State = "loading" | "ready" | "unavailable";

export function ComponentPreview3D({
  componentId,
  componentVersion,
  params,
  size = 220,
}: ComponentPreview3DProps) {
  const host = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<State>("loading");
  // The effect keys on the params' VALUE, not the object: a caller that
  // rebuilds the map each render (the inspector does — it derives it from the
  // record every time) would otherwise tear down and rebuild a WebGL context on
  // every keystroke.
  const paramsKey = previewKey(componentId, componentVersion, params);
  const paramsRef = useRef(params);
  paramsRef.current = params;

  useEffect(() => {
    const mount = host.current;
    if (!mount) return;
    let disposed = false;
    setState("loading");

    let renderer: THREE.WebGLRenderer | null = null;
    let mesh: THREE.Mesh | null = null;
    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(35, 1, 0.1, 1000);
    addPreviewLights(scene);

    // Orbit state, kept here rather than in React: a drag updates it every
    // pointer event, and re-rendering the component 60 times a second to move a
    // camera would be the wrong machine entirely.
    const target = new THREE.Vector3();
    let radius = 1;
    let theta = Math.PI / 4;
    let phi = Math.PI / 3;
    let dragging = false;
    let lastX = 0;
    let lastY = 0;

    const applyCamera = () => {
      camera.position.set(
        target.x + radius * Math.sin(phi) * Math.cos(theta),
        target.y + radius * Math.sin(phi) * Math.sin(theta),
        target.z + radius * Math.cos(phi),
      );
      camera.up.set(0, 0, 1); // world is Z-up, always
      camera.lookAt(target);
      camera.updateProjectionMatrix();
      renderer?.render(scene, camera);
    };

    const onPointerDown = (e: PointerEvent) => {
      dragging = true;
      lastX = e.clientX;
      lastY = e.clientY;
      (e.target as Element).setPointerCapture?.(e.pointerId);
    };
    const onPointerMove = (e: PointerEvent) => {
      if (!dragging) return;
      theta -= (e.clientX - lastX) * 0.01;
      // Clamped off the poles: at phi exactly 0 or π the up vector and the view
      // direction are parallel and `lookAt` has no defined roll.
      phi = Math.min(Math.PI - 0.05, Math.max(0.05, phi - (e.clientY - lastY) * 0.01));
      lastX = e.clientX;
      lastY = e.clientY;
      applyCamera();
    };
    const endDrag = () => {
      dragging = false;
    };

    void (async () => {
      try {
        const bodies = await createClient().componentPreviewMesh(
          componentId,
          componentVersion,
          paramsRef.current,
        );
        if (disposed) return;
        const first = bodies[0];
        if (!first) {
          setState("unavailable");
          return;
        }
        mesh = buildPreviewObject(parseMeshPayload(first.mesh));

        renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
        renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
        renderer.setSize(size, size, false);
        renderer.setClearColor(palette.clear(), 0);
        mount.appendChild(renderer.domElement);
        renderer.domElement.style.touchAction = "none";
        renderer.domElement.style.cursor = "grab";

        scene.add(mesh);
        frameObject(camera, mesh);
        // Seed the orbit from the framed camera so the first drag continues
        // from what the user is already looking at.
        target.copy(new THREE.Box3().setFromObject(mesh).getCenter(new THREE.Vector3()));
        radius = camera.position.distanceTo(target);
        const offset = camera.position.clone().sub(target);
        theta = Math.atan2(offset.y, offset.x);
        phi = Math.acos(Math.min(1, Math.max(-1, offset.z / radius)));

        renderer.domElement.addEventListener("pointerdown", onPointerDown);
        renderer.domElement.addEventListener("pointermove", onPointerMove);
        renderer.domElement.addEventListener("pointerup", endDrag);
        renderer.domElement.addEventListener("pointercancel", endDrag);

        applyCamera();
        setState("ready");
      } catch {
        // No worker, no WebGL, an unbuildable component: the panel says so
        // rather than showing an empty frame that looks like a bug.
        if (!disposed) setState("unavailable");
      }
    })();

    return () => {
      disposed = true;
      const canvas = renderer?.domElement;
      canvas?.removeEventListener("pointerdown", onPointerDown);
      canvas?.removeEventListener("pointermove", onPointerMove);
      canvas?.removeEventListener("pointerup", endDrag);
      canvas?.removeEventListener("pointercancel", endDrag);
      if (mesh) {
        scene.remove(mesh);
        mesh.geometry.dispose(); // the MATERIAL is shared — never disposed here
      }
      if (canvas?.parentElement) canvas.parentElement.removeChild(canvas);
      renderer?.dispose();
    };
    // `paramsKey` stands in for `params` (see above); `paramsRef` is read, not
    // depended on, so a new object with the same values changes nothing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [componentId, componentVersion, paramsKey, size]);

  return (
    <div
      data-testid="component-preview-3d"
      data-state={state}
      className="relative flex items-center justify-center rounded-md border border-border bg-well"
      style={{ width: size, height: size }}
      title={state === "ready" ? "Drag to orbit" : undefined}
    >
      <div ref={host} />
      {state !== "ready" && (
        <div className="absolute text-[11.5px] text-ink-7">
          {state === "loading" ? "Rendering…" : "No preview available"}
        </div>
      )}
    </div>
  );
}
