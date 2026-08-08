/*
 * The host's contract. Every case here is a failure mode that a one-shot
 * "attach whatever is registered at init()" host would have shipped with:
 * contributions register on editor mount while the engine is still awaiting its
 * renderer, so the race is real, not hypothetical.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import * as THREE from "three";
import {
  contributionId,
  createPlatform,
  type Platform,
  type ViewportContext,
  type ViewportContribution,
  type ViewportContributionId,
} from "@/platform";
import { HtmlOverlayDriver } from "./HtmlOverlayDriver";
import { ViewportContributionHost } from "./ContributionHost";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";

const id = (name: string) =>
  contributionId<ViewportContributionId>(MODELING_MODULE_ID, `onecad.modeling.viewport.${name}`);

function makeHost() {
  const root = new THREE.Group();
  const overlay = new HtmlOverlayDriver();
  const overlayEl = document.createElement("div");
  const invalidate = vi.fn();
  const raycaster = new THREE.Raycaster();
  const host = new ViewportContributionHost({
    root,
    overlay,
    getOverlayEl: () => overlayEl,
    invalidate,
    raycastFromClient: () => raycaster,
  });
  return { host, root, overlay, overlayEl, invalidate, raycaster };
}

/** A contribution that records what it was handed and when it ran. */
function probe(name: string, log: string[], opts: { hover?: string | null } = {}) {
  let disposed = false;
  const contribution: ViewportContribution = {
    id: id(name),
    priority: 100,
    attach(ctx: ViewportContext) {
      ctx.onFrame(() => log.push(`${name}:frame`));
      ctx.onThemeChange(() => log.push(`${name}:theme`));
      ctx.registerSecondaryHover(() => opts.hover ?? null);
      return {
        dispose: () => {
          disposed = true;
          log.push(`${name}:dispose`);
        },
      };
    },
  };
  return { contribution, isDisposed: () => disposed };
}

const camera = new THREE.PerspectiveCamera();

describe("ViewportContributionHost", () => {
  let platform: Platform;

  beforeEach(() => {
    platform = createPlatform();
  });

  it("attaches what is ALREADY registered when it starts following the registry", () => {
    const log: string[] = [];
    platform.scopeFor(MODELING_MODULE_ID).registerViewportContribution(probe("a", log).contribution);

    const { host } = makeHost();
    host.setRegistry(platform.viewport);

    expect(host.attachedIds).toEqual([id("a")]);
  });

  it("attaches a contribution registered AFTER it started following — the mount race", () => {
    const log: string[] = [];
    const { host } = makeHost();
    host.setRegistry(platform.viewport);
    expect(host.attachedIds).toEqual([]);

    platform.scopeFor(MODELING_MODULE_ID).registerViewportContribution(probe("late", log).contribution);
    expect(host.attachedIds).toEqual([id("late")]);
  });

  it("disposes a contribution whose registration goes away, exactly once", () => {
    const log: string[] = [];
    const scope = platform.createScope(MODELING_MODULE_ID);
    scope.registerViewportContribution(probe("a", log).contribution);

    const { host } = makeHost();
    host.setRegistry(platform.viewport);
    scope.dispose();

    expect(host.attachedIds).toEqual([]);
    expect(log.filter((l) => l === "a:dispose")).toHaveLength(1);
  });

  it("runs frame and theme callbacks in REGISTRY order, not registration order", () => {
    const log: string[] = [];
    const scope = platform.scopeFor(MODELING_MODULE_ID);
    // Registered late-priority first on purpose.
    scope.registerViewportContribution({ ...probe("second", log).contribution, priority: 200 });
    scope.registerViewportContribution({ ...probe("first", log).contribution, priority: 100 });

    const { host } = makeHost();
    host.setRegistry(platform.viewport);
    host.runFrame(camera, 800);
    host.applyTheme();

    expect(log.filter((l) => l.endsWith(":frame"))).toEqual(["first:frame", "second:frame"]);
    expect(log.filter((l) => l.endsWith(":theme"))).toEqual(["first:theme", "second:theme"]);
  });

  it("takes the FIRST non-null secondary hover token, in registry order", () => {
    const log: string[] = [];
    const scope = platform.scopeFor(MODELING_MODULE_ID);
    scope.registerViewportContribution({
      ...probe("quiet", log, { hover: null }).contribution,
      priority: 100,
    });
    scope.registerViewportContribution({
      ...probe("loud", log, { hover: "token:1" }).contribution,
      priority: 200,
    });

    const { host } = makeHost();
    host.setRegistry(platform.viewport);
    expect(host.secondaryHover(10, 10)).toBe("token:1");
  });

  it("runs no callbacks after dispose", () => {
    const log: string[] = [];
    platform.scopeFor(MODELING_MODULE_ID).registerViewportContribution(probe("a", log).contribution);
    const { host } = makeHost();
    host.setRegistry(platform.viewport);

    host.dispose();
    log.length = 0;
    host.runFrame(camera, 800);
    host.applyTheme();

    expect(log).toEqual([]);
    expect(host.attachedIds).toEqual([]);
  });

  it("stops following the registry after dispose", () => {
    const { host } = makeHost();
    host.setRegistry(platform.viewport);
    host.dispose();

    const log: string[] = [];
    platform.scopeFor(MODELING_MODULE_ID).registerViewportContribution(probe("a", log).contribution);
    expect(host.attachedIds).toEqual([]);
  });

  it("rolls back and skips a contribution whose attach() throws", () => {
    const log: string[] = [];
    const scope = platform.scopeFor(MODELING_MODULE_ID);
    scope.registerViewportContribution({
      id: id("boom"),
      priority: 100,
      attach(ctx) {
        // Register a label BEFORE throwing — the rollback has to take it back.
        ctx.createLabel(document.createElement("div"));
        throw new Error("attach failed");
      },
    });
    scope.registerViewportContribution({ ...probe("ok", log).contribution, priority: 200 });

    const { host, overlay, overlayEl } = makeHost();
    host.setRegistry(platform.viewport);

    expect(host.attachedIds).toEqual([id("ok")]);
    expect(overlay.size, "the failed attach left no overlay item").toBe(0);
    expect(overlayEl.childElementCount, "…and no orphan element").toBe(0);
    // The healthy neighbour still runs.
    host.runFrame(camera, 800);
    expect(log).toContain("ok:frame");
  });

  it("detaches a contribution whose frame callback throws, and keeps drawing", () => {
    const log: string[] = [];
    const scope = platform.scopeFor(MODELING_MODULE_ID);
    scope.registerViewportContribution({
      id: id("boom"),
      priority: 100,
      attach(ctx) {
        ctx.onFrame(() => {
          throw new Error("frame failed");
        });
        return { dispose: () => log.push("boom:dispose") };
      },
    });
    scope.registerViewportContribution({ ...probe("ok", log).contribution, priority: 200 });

    const { host } = makeHost();
    host.setRegistry(platform.viewport);
    host.runFrame(camera, 800);

    expect(log).toContain("boom:dispose");
    expect(log).toContain("ok:frame");
    expect(host.attachedIds).toEqual([id("ok")]);

    // And it stays gone — one bad frame must not become a per-frame throw.
    log.length = 0;
    host.runFrame(camera, 800);
    expect(log).toEqual(["ok:frame"]);
  });

  it("gives a label the host's overlay, and takes it back on dispose", () => {
    const scope = platform.createScope(MODELING_MODULE_ID);
    scope.registerViewportContribution({
      id: id("labelled"),
      attach(ctx) {
        const label = ctx.createLabel(document.createElement("div"));
        label.setPosition({ x: 1, y: 2, z: 3 });
        return { dispose: () => label.dispose() };
      },
    });

    const { host, overlay, overlayEl } = makeHost();
    host.setRegistry(platform.viewport);
    expect(overlay.size).toBe(1);
    expect(overlayEl.childElementCount).toBe(1);

    scope.dispose();
    expect(overlay.size).toBe(0);
    expect(overlayEl.childElementCount).toBe(0);
  });
  // ── hardening (Codex implementation review) ────────────────────────────────

  it("schedules a frame when a contribution is REMOVED, not only when one is added", () => {
    const log: string[] = [];
    const scope = platform.createScope(MODELING_MODULE_ID);
    scope.registerViewportContribution(probe("a", log).contribution);

    const { host, invalidate } = makeHost();
    host.setRegistry(platform.viewport);
    invalidate.mockClear();

    scope.dispose();
    // Disposal took its objects out of the scene graph. On an on-demand renderer
    // nothing else schedules a repaint, so the last framebuffer would keep
    // showing them.
    expect(invalidate).toHaveBeenCalled();
  });

  it("schedules a frame when setRegistry(null) detaches everything", () => {
    const log: string[] = [];
    platform.scopeFor(MODELING_MODULE_ID).registerViewportContribution(probe("a", log).contribution);
    const { host, invalidate } = makeHost();
    host.setRegistry(platform.viewport);
    invalidate.mockClear();

    host.setRegistry(null);
    expect(invalidate).toHaveBeenCalled();
  });

  it("does NOT re-attach a contribution that already threw, on the next registry change", () => {
    const log: string[] = [];
    const scope = platform.scopeFor(MODELING_MODULE_ID);
    scope.registerViewportContribution({
      id: id("boom"),
      priority: 100,
      attach(ctx) {
        log.push("boom:attach");
        ctx.onFrame(() => {
          throw new Error("frame failed");
        });
        return { dispose: () => {} };
      },
    });

    const { host } = makeHost();
    host.setRegistry(platform.viewport);
    host.runFrame(camera, 800);
    expect(host.attachedIds).toEqual([]);

    // Any unrelated registration re-runs reconcile(). Without the failed-set the
    // crashed contribution comes straight back and throws on the next frame,
    // forever.
    scope.registerViewportContribution(probe("other", log).contribution);
    expect(host.attachedIds).toEqual([id("other")]);
    expect(log.filter((l) => l === "boom:attach")).toHaveLength(1);
  });

  it("gives a re-REGISTERED contribution a genuine second chance", () => {
    const log: string[] = [];
    const scope = platform.createScope(MODELING_MODULE_ID);
    const bad: ViewportContribution = {
      id: id("boom"),
      priority: 100,
      attach(ctx) {
        log.push("boom:attach");
        ctx.onFrame(() => {
          throw new Error("frame failed");
        });
        return { dispose: () => {} };
      },
    };
    scope.registerViewportContribution(bad);

    const { host } = makeHost();
    host.setRegistry(platform.viewport);
    host.runFrame(camera, 800);

    // Registration GONE, then back: the suppression is keyed to the old
    // registration, not to the id forever.
    scope.dispose();
    platform.createScope(MODELING_MODULE_ID).registerViewportContribution(bad);
    expect(host.attachedIds).toEqual([id("boom")]);
    expect(log.filter((l) => l === "boom:attach")).toHaveLength(2);
  });

  it("releases EVERY label on detach, not every second one", () => {
    const scope = platform.createScope(MODELING_MODULE_ID);
    scope.registerViewportContribution({
      id: id("multi"),
      attach(ctx) {
        // Three, because the bug (splicing the array being iterated) leaves the
        // middle one behind and two would look like an off-by-one.
        //
        // Deliberately NOT disposed by the contribution: that is what puts the
        // HOST's fallback loop in charge, which is the code path under test. A
        // contribution that tidies up after itself hides the defect entirely.
        ctx.createLabel(document.createElement("div"));
        ctx.createLabel(document.createElement("div"));
        ctx.createLabel(document.createElement("div"));
        return { dispose: () => {} };
      },
    });

    const { host, overlay, overlayEl } = makeHost();
    host.setRegistry(platform.viewport);
    expect(overlay.size).toBe(3);

    scope.dispose();
    expect(overlay.size).toBe(0);
    expect(overlayEl.childElementCount).toBe(0);
  });

  it("reclaims host-owned labels even when the contribution's own disposer throws", () => {
    const scope = platform.createScope(MODELING_MODULE_ID);
    scope.registerViewportContribution({
      id: id("badDispose"),
      attach(ctx) {
        ctx.createLabel(document.createElement("div"));
        ctx.createLabel(document.createElement("div"));
        return {
          dispose: () => {
            throw new Error("dispose failed");
          },
        };
      },
    });

    const { host, overlay, overlayEl } = makeHost();
    host.setRegistry(platform.viewport);
    expect(overlay.size).toBe(2);

    scope.dispose();
    expect(overlay.size, "a throwing disposer must not strand overlay items").toBe(0);
    expect(overlayEl.childElementCount).toBe(0);
  });
});
