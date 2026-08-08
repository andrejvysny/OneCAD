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
import { createPlatform, PlatformProvider, type Platform } from "@/platform";
import { registerModelingModule } from "@/modules/modeling/register";

export interface PlatformRenderResult extends RenderResult {
  readonly platform: Platform;
}

/** A booted platform with the modeling module active. */
export function bootTestPlatform(): Platform {
  const platform = createPlatform();
  registerModelingModule(platform);
  platform.initializeSync();
  return platform;
}

export function renderWithPlatform(
  ui: ReactElement,
  options: RenderOptions & { platform?: Platform } = {},
): PlatformRenderResult {
  const { platform = bootTestPlatform(), ...renderOptions } = options;
  const result = render(ui, {
    ...renderOptions,
    wrapper: ({ children }: { children: ReactNode }) => (
      <PlatformProvider platform={platform}>{children}</PlatformProvider>
    ),
  });
  return Object.assign(result, { platform });
}
