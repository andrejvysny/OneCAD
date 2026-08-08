/*
 * Host side of `ViewportContribution`.
 *
 * The engine owns the renderer, the frame loop and the palette; a contribution
 * owns some objects in a group and asks to be told about frames and theme
 * changes. This class is the part that has to be right for that to be safe:
 *
 *  - RECONCILIATION, not one-shot attach. Contributions register when the editor
 *    mounts, while the engine initializes asynchronously, so "attach whatever is
 *    registered at init()" loses whichever side lost the race. The host attaches
 *    what is there, then stays subscribed for the engine's life.
 *  - REVERSE-ORDER teardown, so a contribution can rely on what it was built on
 *    still existing while it tears down (the same rule `ModuleScope.dispose`
 *    follows).
 *  - FAILURE ISOLATION. A throwing `attach` is rolled back and skipped; a
 *    throwing frame/theme/hover callback detaches that contribution. One bad
 *    layer must not kill a frame or leave the renderer half torn down — the
 *    same guarantee `ContributionBoundary` gives React contributions.
 */
import * as THREE from "three";
import type { Registry, ViewportContext, ViewportContribution, ViewportLabelHandle } from "@/platform";
import type { HtmlOverlayDriver } from "./HtmlOverlayDriver";
import { logError } from "@/debug/log";

export interface ContributionHostDeps {
  /** The group contributions attach into — a child of the engine's interaction root. */
  root: THREE.Object3D;
  overlay: HtmlOverlayDriver;
  /** Null before `init()`; a contribution that needs a label waits for it. */
  getOverlayEl: () => HTMLElement | null;
  invalidate: () => void;
  raycastFromClient: (clientX: number, clientY: number) => THREE.Raycaster | null;
}

type FrameCallback = (camera: THREE.Camera, viewportHeight: number) => void;

interface Attached {
  readonly id: string;
  readonly dispose: () => void;
  readonly frames: FrameCallback[];
  readonly themes: (() => void)[];
  readonly hovers: ((x: number, y: number) => string | null)[];
}

let labelSeq = 0;

export class ViewportContributionHost {
  /** Attached contributions in REGISTRY order — the order frames and hovers run in. */
  private readonly attached: Attached[] = [];
  /**
   * Ids that threw and were detached. Held back from re-attachment while their
   * registration still exists: reconcile() runs on every registry notification,
   * so without this a contribution whose frame callback throws is re-attached by
   * the next unrelated register/dispose and throws again, forever.
   */
  private readonly failed = new Set<string>();
  private registry: Registry<ViewportContribution> | null = null;
  private unsubscribe: (() => void) | null = null;
  private disposed = false;

  constructor(private readonly deps: ContributionHostDeps) {}

  /**
   * Point the host at a registry (or `null` to detach from one). Attaches the
   * current entries and keeps following it.
   */
  setRegistry(registry: Registry<ViewportContribution> | null): void {
    if (this.disposed || registry === this.registry) return;
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.registry = registry;
    if (!registry) {
      this.detachAll();
      return;
    }
    this.reconcile();
    this.unsubscribe = registry.subscribe(() => this.reconcile());
  }

  private reconcile(): void {
    if (this.disposed || !this.registry) return;
    const entries = this.registry.entries();
    const live = new Set(entries.map((e) => e.id as string));

    // A contribution that failed is forgotten once its registration goes away,
    // so a re-register is a genuine second chance rather than a replay of the
    // same crash.
    for (const id of [...this.failed]) if (!live.has(id)) this.failed.delete(id);

    // Removals first, reverse order, so teardown sees a consistent world.
    let changed = false;
    for (let i = this.attached.length - 1; i >= 0; i--) {
      const a = this.attached[i];
      if (live.has(a.id)) continue;
      this.attached.splice(i, 1);
      this.safeDispose(a);
      changed = true;
    }

    const have = new Set(this.attached.map((a) => a.id));
    for (const entry of entries) {
      const id = entry.id as string;
      if (have.has(id) || this.failed.has(id)) continue;
      if (this.attach(entry)) changed = true;
    }
    // Keep the array in registry order even when something attached late.
    const rank = new Map(entries.map((e, i) => [e.id as string, i]));
    this.attached.sort((a, b) => (rank.get(a.id) ?? 0) - (rank.get(b.id) ?? 0));
    // A REMOVAL needs the frame just as much as an addition: disposal takes the
    // objects out of the scene graph, and on an on-demand renderer nothing else
    // schedules a repaint — the last framebuffer would keep showing them.
    if (changed) this.deps.invalidate();
  }

  private attach(entry: ViewportContribution): boolean {
    const frames: FrameCallback[] = [];
    const themes: (() => void)[] = [];
    const hovers: ((x: number, y: number) => string | null)[] = [];
    const labels: ViewportLabelHandle[] = [];
    const id = entry.id as string;

    const ctx: ViewportContext = {
      invalidate: this.deps.invalidate,
      root: this.deps.root,
      createLabel: (element) => this.createLabel(element, labels),
      onFrame: (cb) => addAndReturn(frames, cb),
      onThemeChange: (cb) => addAndReturn(themes, cb),
      raycastFromClient: this.deps.raycastFromClient,
      registerSecondaryHover: (fn) => addAndReturn(hovers, fn),
    };

    let handle: { dispose(): void };
    try {
      handle = entry.attach(ctx);
    } catch (err) {
      // Roll back whatever the failed attach managed to register, so a broken
      // contribution cannot leave a label or a frame callback behind.
      for (const l of [...labels]) l.dispose();
      logError("err", "viewport contribution attach failed", {
        contributionId: id,
        message: err instanceof Error ? err.message : String(err),
      });
      return false;
    }

    this.attached.push({
      id,
      frames,
      themes,
      hovers,
      dispose: () => {
        try {
          handle.dispose();
        } finally {
          // Snapshot: each label's dispose() splices itself out of `labels`, so
          // iterating the live array skips every second one. `finally`: a
          // contribution's own disposer throwing must not strand host-owned
          // overlay items.
          for (const l of [...labels]) l.dispose();
        }
      },
    });
    return true;
  }

  private createLabel(element: HTMLElement, own: ViewportLabelHandle[]): ViewportLabelHandle {
    const id = `__vp_contrib_label_${labelSeq++}`;
    const host = this.deps.getOverlayEl();
    const pos = new THREE.Vector3();
    let registered = false;
    if (host) {
      host.appendChild(element);
      this.deps.overlay.register(id, element, pos);
      registered = true;
    }
    const handle: ViewportLabelHandle = {
      setPosition: (world) => {
        pos.set(world.x, world.y, world.z);
        if (registered) this.deps.overlay.setWorldPos(id, pos);
      },
      dispose: () => {
        if (registered) this.deps.overlay.unregister(id);
        registered = false;
        element.remove();
        const i = own.indexOf(handle);
        if (i >= 0) own.splice(i, 1);
      },
    };
    own.push(handle);
    return handle;
  }

  /** Run every contribution's frame callbacks, in registry order. */
  runFrame(camera: THREE.Camera, viewportHeight: number): void {
    for (const a of [...this.attached]) {
      for (const cb of [...a.frames]) {
        if (!this.guard(a, () => cb(camera, viewportHeight), "frame")) break;
      }
    }
  }

  /** Tell every contribution the palette changed (see the applyTheme invariant). */
  applyTheme(): void {
    for (const a of [...this.attached]) {
      for (const cb of [...a.themes]) {
        if (!this.guard(a, cb, "theme")) break;
      }
    }
  }

  /**
   * The first contribution-supplied hover token, in registry order. Built-in
   * hover sources are consulted by the caller BEFORE this.
   */
  secondaryHover(clientX: number, clientY: number): string | null {
    for (const a of [...this.attached]) {
      for (const fn of [...a.hovers]) {
        let token: string | null = null;
        if (!this.guard(a, () => { token = fn(clientX, clientY); }, "hover")) break;
        if (token !== null) return token;
      }
    }
    return null;
  }

  /** Runs `fn`; on a throw, detaches the contribution and returns false. */
  private guard(a: Attached, fn: () => void, lane: string): boolean {
    try {
      fn();
      return true;
    } catch (err) {
      logError("err", `viewport contribution ${lane} callback failed`, {
        contributionId: a.id,
        message: err instanceof Error ? err.message : String(err),
      });
      const i = this.attached.indexOf(a);
      if (i >= 0) this.attached.splice(i, 1);
      this.failed.add(a.id);
      this.safeDispose(a);
      // Its objects just left the scene graph — schedule the frame that shows that.
      this.deps.invalidate();
      return false;
    }
  }

  private safeDispose(a: Attached): void {
    try {
      a.dispose();
    } catch (err) {
      logError("err", "viewport contribution dispose failed", {
        contributionId: a.id,
        message: err instanceof Error ? err.message : String(err),
      });
    }
  }

  private detachAll(): void {
    const had = this.attached.length > 0;
    for (const a of this.attached.reverse()) this.safeDispose(a);
    this.attached.length = 0;
    if (had && !this.disposed) this.deps.invalidate();
  }

  /** Ids currently attached, in run order (introspection / tests). */
  get attachedIds(): string[] {
    return this.attached.map((a) => a.id);
  }

  dispose(): void {
    this.disposed = true;
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.registry = null;
    this.detachAll();
  }
}

function addAndReturn<T>(list: T[], item: T): { dispose(): void } {
  list.push(item);
  return {
    dispose: () => remove(list, item),
  };
}

function remove<T>(list: T[], item: T): void {
  const i = list.indexOf(item);
  if (i >= 0) list.splice(i, 1);
}
