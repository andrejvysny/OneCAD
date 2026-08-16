/*
 * Material-source bridge — the seam a module publishes per-body appearance
 * through, without the viewport ever importing the module.
 *
 * Direction is the whole point. `src/viewport/**` must not depend on
 * `src/modules/**` (docs/ARCHITECTURE.md), so the viewport declares the shape it
 * can consume ({@link BodyMaterialSource}, `engine/pbrParams.ts`) and whoever
 * owns materials REGISTERS one here. Today that is `onecad.render`; nothing in
 * this file knows that.
 *
 * A module singleton, exactly like `engineBridge.ts`, and for the same reason:
 * the two lifetimes do not nest. A module activates once at bootstrap, while
 * `MeshIngest` attaches and detaches with the viewport engine (and twice over
 * under StrictMode), so neither side can be the other's parent. Late arrivals in
 * both directions are handled — `MeshIngest` reads the current value at attach
 * AND subscribes, so registration order does not matter.
 *
 * Type-only imports keep this file free of any runtime dependency, so the
 * registering module pulls in no THREE.
 */
import type { BodyMaterialSource } from "./engine/pbrParams";

let current: BodyMaterialSource | null = null;
const listeners = new Set<(source: BodyMaterialSource | null) => void>();

/** Publish the live source, or `null` to withdraw it (module teardown). */
export function setBodyMaterialSource(source: BodyMaterialSource | null): void {
  current = source;
  for (const l of [...listeners]) l(source);
}

export function getBodyMaterialSource(): BodyMaterialSource | null {
  return current;
}

/** Notified on every publish/withdraw. Returns the unsubscribe. */
export function subscribeBodyMaterialSource(
  cb: (source: BodyMaterialSource | null) => void,
): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}
