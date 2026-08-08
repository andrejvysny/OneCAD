/*
 * Render a component tree that needs the Platform.
 *
 * Once a surface reads the contribution registry instead of a module-load-time
 * table, `usePlatform()` throws without a provider — so a test that renders it
 * bare is no longer testing the shipped mechanism. This boots a REAL platform
 * with the real modeling module rather than a stub registry: a fake would let a
 * registration bug pass here and fail only in the app.
 *
 * Only the bootstrap lane is registered (tools + commands). UI contributions
 * belong to the editor-mount scope (`contributeModelingUi`); a test that needs
 * those registers them itself, which is also what keeps this helper from
 * dragging the editor chunk into every suite.
 */
import type { ReactElement, ReactNode } from "react";
import { render, type RenderOptions, type RenderResult } from "@testing-library/react";
import { createPlatform, PlatformProvider, type ModuleScope, type Platform } from "@/platform";
import { registerModelingModule } from "@/modules/modeling/register";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";

export interface PlatformRenderResult extends RenderResult {
  readonly platform: Platform;
}

/**
 * A booted platform with the modeling module active.
 *
 * `contribute` runs against an EDITOR-MOUNT-style child scope, the same way
 * `EditorShell` registers UI. It is a parameter rather than a default because
 * pulling `ui.ts` in unconditionally would drag every editor component into
 * every suite that only needs a tool registry.
 */
export function bootTestPlatform(contribute?: (scope: ModuleScope) => void): Platform {
  const platform = createPlatform();
  registerModelingModule(platform);
  platform.initializeSync();
  contribute?.(platform.createScope(MODELING_MODULE_ID));
  return platform;
}

export function renderWithPlatform(
  ui: ReactElement,
  options: RenderOptions & {
    platform?: Platform;
    contribute?: (scope: ModuleScope) => void;
  } = {},
): PlatformRenderResult {
  const { platform: given, contribute, ...renderOptions } = options;
  const platform = given ?? bootTestPlatform(contribute);
  const result = render(ui, {
    ...renderOptions,
    wrapper: ({ children }: { children: ReactNode }) => (
      <PlatformProvider platform={platform}>{children}</PlatformProvider>
    ),
  });
  return Object.assign(result, { platform });
}
