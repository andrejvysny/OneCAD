import type { TreeProviderId } from "@/platform";

/** Provider-local tree node identity; opaque to the generic tree host. */
export interface TreeNodeLocator {
  readonly providerId: TreeProviderId;
  readonly nodeId: string;
}

type TreeRevealListener = (target: TreeNodeLocator) => boolean;

const listeners = new Set<TreeRevealListener>();

/** Requests a mounted tree host to expand and scroll an existing row. */
export function requestTreeReveal(target: TreeNodeLocator): boolean {
  let accepted = false;
  for (const listener of listeners) accepted = listener(target) || accepted;
  return accepted;
}

/** The request has no queue: an unmounted host must not retain stale locators. */
export function subscribeTreeReveal(listener: TreeRevealListener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
