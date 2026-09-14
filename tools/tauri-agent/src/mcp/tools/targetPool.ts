/**
 * The semantic pool (`session.refs`) is only as fresh as the last `ui_snapshot`. Live runs hit
 * ELEMENT_NOT_FOUND for elements plainly on screen because the pool was stale or scoped, and
 * every caller then spent a turn re-snapshotting by hand. A miss on a target that names the
 * element semantically (role/name, testId, text) is therefore answered by one full refresh and
 * one retry, here, for every consumer. Refs and raw points are never retried: a ref that is gone
 * is stale by definition, and a point has no pool to miss.
 */
import { isAgentError } from "../../errors.ts";
import { Resolver, refStoreFrom } from "../../semantic/resolve.ts";
import type { ResolveOpts, Resolved, Target } from "../../semantic/resolve.ts";
import type { BridgeLike } from "../../semantic/webdriver.ts";
import type { SnapNode, Snapshot } from "../../semantic/snapshot.ts";
import { takeSnapshot } from "../../semantic/snapshot.ts";

export interface PoolSession {
  readonly refs: Map<string, SnapNode>;
  lastSnapshot?: Snapshot;
}

export const POOL_REFRESHED_WARNING = "target pool refreshed: the element was not in the last snapshot, a fresh full snapshot found it";

export function poolMiss(target: Target): boolean {
  return !("ref" in target) && !("point" in target) && !("css" in target);
}

/** Replace the pool with a fresh full interactive snapshot and return it. */
export async function refreshPool(session: PoolSession, bridge: BridgeLike): Promise<Snapshot> {
  const snap = await takeSnapshot(bridge, { mode: "interactive" });
  session.refs.clear();
  for (const n of snap.nodes) session.refs.set(n.ref, n);
  session.lastSnapshot = snap;
  return snap;
}

export async function resolveWithRefresh(
  session: PoolSession,
  bridge: BridgeLike,
  target: Target,
  opts: ResolveOpts = {},
): Promise<{ resolved: Resolved; refreshed: boolean }> {
  const attempt = (): Promise<Resolved> =>
    new Resolver(bridge, refStoreFrom([...session.refs.values()])).resolve(target, opts);
  try {
    return { resolved: await attempt(), refreshed: false };
  } catch (e) {
    if (!(isAgentError(e) && e.code === "ELEMENT_NOT_FOUND" && poolMiss(target))) throw e;
    await refreshPool(session, bridge);
    return { resolved: await attempt(), refreshed: true };
  }
}
