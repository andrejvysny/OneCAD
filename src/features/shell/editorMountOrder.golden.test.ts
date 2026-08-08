/*
 * Editor mount-order GOLDEN probe.
 *
 * The editor's children are absolutely-positioned siblings over the viewport, so
 * DOM order decides stacking inside a z-index band — a past defect had the tool
 * chips render under the side panels and become unclickable. That makes mount
 * order a real invariant to carry across the refactor.
 *
 * W0 read the shipped JSX. Now that the shell is slot-hosted, the same frozen
 * contract is asserted against what the REGISTRY produces: slot regions in the
 * shell's declared order, contributions inside each region in registry order.
 * The contract in `src/test/contracts/` did not move — only the probe did.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { createPlatform, Slots, type Platform } from "@/platform";
import { EDITOR_MOUNT_ORDER_CONTRACT } from "@/test/contracts/shellContract";
import { EDITOR_REGIONS } from "@/app/shell/EditorShell";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";
import { contributeModelingUi } from "@/modules/modeling/ui";
import { SHELL_MODULE_ID } from "@/modules/shell/panelIds";
import { contributeShellChrome } from "@/modules/shell/register";

/**
 * What the shell renders, in order. `component.name` is the component's function
 * name, which is exactly what the frozen contract lists.
 */
function mountOrder(platform: Platform): string[] {
  const inSlot = (slot: string): string[] =>
    platform.panels
      .entries()
      .filter((p) => p.slot === slot)
      .map((p) => p.component.name);

  return [
    ...inSlot(Slots.ShellTop),
    ...EDITOR_REGIONS.flatMap((region) =>
      region === "viewport" ? ["ViewportRoot"] : inSlot(region),
    ),
  ];
}

describe("editor mount order", () => {
  let platform: Platform;

  beforeEach(() => {
    platform = createPlatform();
    contributeShellChrome(platform.createScope(SHELL_MODULE_ID));
    contributeModelingUi(platform.createScope(MODELING_MODULE_ID));
  });

  it("mounts exactly the frozen contribution set, in the frozen order", () => {
    expect(mountOrder(platform)).toEqual([...EDITOR_MOUNT_ORDER_CONTRACT]);
  });

  it("places every registered panel in a region the shell actually renders", () => {
    // A contribution in an unrendered slot is invisible with no error — the one
    // failure mode slot hosting introduces that the order check cannot see.
    const rendered = new Set<string>([Slots.ShellTop, ...EDITOR_REGIONS]);
    for (const panel of platform.panels.entries()) {
      expect(rendered.has(panel.slot), `${panel.id} is in unrendered slot ${panel.slot}`).toBe(
        true,
      );
    }
  });

  it("survives a remount without duplicate-id collisions", () => {
    // React StrictMode double-invokes mount effects, so registration must be
    // dispose-clean and repeatable.
    const scope = platform.createScope(MODELING_MODULE_ID);
    scope.dispose();
    const modeling = platform.createScope(MODELING_MODULE_ID);
    const shell = platform.createScope(SHELL_MODULE_ID);
    platform.disposeOwner(MODELING_MODULE_ID);
    platform.disposeOwner(SHELL_MODULE_ID);
    contributeShellChrome(shell);
    contributeModelingUi(modeling);
    expect(mountOrder(platform)).toEqual([...EDITOR_MOUNT_ORDER_CONTRACT]);
  });
});
