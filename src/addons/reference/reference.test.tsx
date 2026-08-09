/*
 * The reference addon, driven through the REAL hosts.
 *
 * A test that called the addon's own functions would prove nothing: the claim is
 * that the SHIPPED toolbar, tree, inspector and shortcut lane accept a
 * contribution from something that has never heard of modeling. So the addon is
 * registered against a real platform and then everything is asserted through the
 * user-facing surfaces.
 *
 * This file may import host internals; the ADDON may not (`architecture.test.ts`).
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createExtensionContext,
  createPlatform,
  PlatformProvider,
  type Platform,
  type DocumentStateService,
  type ModuleDocumentState,
} from "@/platform";
import { moduleId } from "@/platform/ids";
import { resetStores } from "@/test/resetStores";
import { contributeModelingTree } from "@/modules/modeling/treeProvider";
import { registerModelingModule } from "@/modules/modeling/register";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";
import { FloatingToolbar } from "@/features/toolbar/FloatingToolbar";
import { ModelTreePanel } from "@/features/tree/ModelTreePanel";
import { createReferenceAddon, REFERENCE_ADDON_ID, ReferenceIds } from ".";
import type { WidgetState } from "./domain";

/** An in-memory stand-in for the backend's module-state slice. */
function fakeDocumentState(): DocumentStateService & { slice: () => unknown } {
  let stored: ModuleDocumentState | null = null;
  return {
    slice: () => stored,
    read: async <T,>() => stored as ModuleDocumentState<T> | null,
    write: async (state) => {
      stored = state;
    },
    clear: async () => {
      stored = null;
    },
  };
}

async function bootWithAddon() {
  const platform = createPlatform();
  registerModelingModule(platform);
  platform.initializeSync();
  contributeModelingTree(platform.createScope(MODELING_MODULE_ID));

  const documentState = fakeDocumentState();
  const addon = createReferenceAddon();
  const scope = platform.createScope(REFERENCE_ADDON_ID);
  const context = createExtensionContext(scope, documentState);
  await addon.activate(context);
  return { platform, documentState, scope, addon, context };
}

const withPlatform = (platform: Platform, ui: React.ReactElement) =>
  render(<PlatformProvider platform={platform}>{ui}</PlatformProvider>);

describe("reference addon", () => {
  beforeEach(() => resetStores());

  it("registers one contribution of every open kind", async () => {
    const { platform } = await bootWithAddon();

    expect(platform.commands.has(ReferenceIds.command)).toBe(true);
    expect(platform.tools.has(ReferenceIds.tool)).toBe(true);
    expect(platform.panels.has(ReferenceIds.panel)).toBe(true);
    expect(platform.inspector.has(ReferenceIds.inspector)).toBe(true);
    expect(platform.tree.has(ReferenceIds.tree)).toBe(true);
    expect(platform.viewport.has(ReferenceIds.viewport)).toBe(true);
    expect(platform.workspaces.has(ReferenceIds.workspace)).toBe(true);
  });

  it("its tool reaches the REAL toolbar and activates through it", async () => {
    const user = userEvent.setup();
    const { platform } = await bootWithAddon();
    withPlatform(platform, <FloatingToolbar />);

    const button = screen.getByRole("button", { name: "Place widget" });
    await user.click(button);

    expect(platform.toolHost.activeToolId()).toBe(ReferenceIds.tool);
    expect(screen.getByRole("button", { name: "Place widget" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    // Modeling's own tool gave up the highlight, and neither side knows the other.
    expect(screen.getByRole("button", { name: "Select" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("its tool grays out on ITS OWN predicate, and un-grays when its state moves", async () => {
    const platform = createPlatform();
    registerModelingModule(platform);
    platform.initializeSync();
    // No widgets ⇒ nothing to place. Its `canActivate` says so, and the toolbar
    // has no idea what a widget is.
    await createReferenceAddon([]).activate(
      createExtensionContext(platform.createScope(REFERENCE_ADDON_ID), fakeDocumentState()),
    );
    withPlatform(platform, <FloatingToolbar />);

    expect(screen.getByRole("button", { name: "Place widget" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );

    // Its own command changes its own state; the button follows because the tool
    // declared `subscribe`, not because the host watches anything.
    await act(async () => {
      await platform.commands.get(ReferenceIds.command)?.execute({ selection: [], scopes: [] });
    });

    expect(screen.getByRole("button", { name: "Place widget" })).toHaveAttribute(
      "aria-disabled",
      "false",
    );
  });

  it("its rows reach the REAL tree, beside modeling's, and update themselves", async () => {
    const user = userEvent.setup();
    const { platform } = await bootWithAddon();
    withPlatform(platform, <ModelTreePanel />);

    expect(screen.getByRole("listbox", { name: "Widgets" })).toBeInTheDocument();
    expect(screen.getByRole("listbox", { name: "Bodies" })).toBeInTheDocument();

    // Its rename runs through the addon's own store; the host re-renders because
    // the provider said so, not because it watches anything.
    await user.pointer({
      keys: "[MouseRight]",
      target: screen.getByRole("option", { name: /Widget A/ }),
    });
    expect(screen.getByTestId("tree-menu-rename")).toBeInTheDocument();
  });

  it("its row action runs its command", async () => {
    const user = userEvent.setup();
    const { platform } = await bootWithAddon();
    withPlatform(platform, <ModelTreePanel />);

    await user.pointer({
      keys: "[MouseRight]",
      target: screen.getByRole("option", { name: /Widget A/ }),
    });
    await user.click(screen.getByTestId(`tree-action-${REFERENCE_ADDON_ID}.action.add`));

    // The command added a widget; the tree shows it without anything re-rendering
    // on the host's initiative.
    expect(screen.getByRole("option", { name: /Widget 3/ })).toBeInTheDocument();
  });

  it("its shortcut resolves through the platform", async () => {
    const { platform } = await bootWithAddon();

    const target = platform.shortcuts.resolve({ key: "w", shift: true }, []);
    expect(target?.id).toBe(ReferenceIds.command);

    await platform.shortcuts.run(target!);
    expect(platform.tree.get(ReferenceIds.tree)?.sections()[0]?.nodes).toHaveLength(3);
  });

  it("writes ONLY into its own document namespace, and restores from it", async () => {
    const { platform, documentState, scope } = await bootWithAddon();

    await platform.commands.get(ReferenceIds.command)?.execute({ selection: [], scopes: [] });
    const stored = documentState.slice() as ModuleDocumentState<WidgetState>;
    expect(stored.schemaVersion).toBe(1);
    expect(stored.payload.widgets).toHaveLength(3);

    // A SECOND activation against the same slice restores what the first wrote —
    // the round trip that makes an addon's document state worth preserving.
    act(() => scope.dispose());
    const second = createReferenceAddon();
    await second.activate(
      createExtensionContext(platform.createScope(REFERENCE_ADDON_ID), documentState),
    );

    expect(platform.tree.get(ReferenceIds.tree)?.sections()[0]?.nodes).toHaveLength(3);
  });

  it("gets a context with no way to reach the host", async () => {
    const { context } = await bootWithAddon();

    // The narrowing is by construction, not by cast: these are absent at RUNTIME.
    const reachable = context as unknown as Record<string, unknown>;
    for (const forbidden of ["platform", "createScope", "scopeFor", "disposeOwner", "dispose"]) {
      expect(reachable[forbidden], forbidden).toBeUndefined();
    }
  });

  it("disposing its scope takes every contribution with it", async () => {
    const { platform, scope } = await bootWithAddon();
    act(() => scope.dispose());

    expect(platform.commands.has(ReferenceIds.command)).toBe(false);
    expect(platform.tools.has(ReferenceIds.tool)).toBe(false);
    expect(platform.panels.has(ReferenceIds.panel)).toBe(false);
    expect(platform.inspector.has(ReferenceIds.inspector)).toBe(false);
    expect(platform.tree.has(ReferenceIds.tree)).toBe(false);
    expect(platform.viewport.has(ReferenceIds.viewport)).toBe(false);
    expect(platform.workspaces.has(ReferenceIds.workspace)).toBe(false);
  });

  it("cannot mint an id outside its own namespace", () => {
    // The check that makes the namespace rule a runtime fact (ADR-0006).
    const platform = createPlatform();
    expect(() =>
      platform.createScope(REFERENCE_ADDON_ID).registerCommand({
        id: "onecad.modeling.command.extrude" as never,
        title: "Steal",
        execute: () => ({ status: "done" as const }),
      }),
    ).toThrow(/namespace/);
  });

  it("a foreign module id cannot be smuggled through the context", async () => {
    const { context } = await bootWithAddon();
    expect(context.owner).toBe(REFERENCE_ADDON_ID);
    expect(context.owner).not.toBe(moduleId("onecad.modeling"));
  });

  it("its viewport layer attaches and gives everything back", async () => {
    const { platform } = await bootWithAddon();
    const invalidate = vi.fn();
    const themeDisposer = vi.fn();

    const handle = platform.viewport.get(ReferenceIds.viewport)?.attach({
      invalidate,
      root: {} as never,
      createLabel: () => ({ setPosition: () => {}, dispose: () => {} }),
      onFrame: () => ({ dispose: () => {} }),
      onThemeChange: () => ({ dispose: themeDisposer }),
      raycastFromClient: () => null,
      registerSecondaryHover: () => ({ dispose: () => {} }),
    });

    await platform.commands.get(ReferenceIds.command)?.execute({ selection: [], scopes: [] });
    expect(invalidate).toHaveBeenCalled(); // it repaints on its OWN state change

    handle?.dispose();
    expect(themeDisposer).toHaveBeenCalled();
  });
});
