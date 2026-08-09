import { describe, it, expect, beforeEach } from "vitest";
import { screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FloatingToolbar } from "./FloatingToolbar";
import { ICON_MONO } from "@/icons/Icon";
import { resetStores } from "@/test/resetStores";
import { bootTestPlatform, renderWithPlatform } from "@/test/renderWithPlatform";
import { selectionStore } from "@/stores/selectionStore";
import {
  addonId,
  contributionId,
  type ModuleScope,
  type Platform,
  type ToolDefinition,
  type ToolId,
} from "@/platform";
import { ModelingScopes } from "@/modules/modeling/manifest";

describe("FloatingToolbar", () => {
  beforeEach(() => resetStores());

  it("renders the model tool set with Select active by default", () => {
    renderWithPlatform(<FloatingToolbar />);
    for (const name of ["Select", "New sketch", "Extrude", "Revolve", "Fillet / Chamfer", "Combine"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    }
    // Sketch-only tools are absent in model mode.
    expect(screen.queryByRole("button", { name: "Line" })).toBeNull();
    expect(screen.getByRole("button", { name: "Select" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("collapses the icon to monochrome on the ACTIVE tool only", () => {
    // The active button paints its glyph `text-accent`. Without the override
    // the icon's own accent tone stays a second, near-identical blue and the
    // two-tone split turns to mush. Idle buttons are neutral ink and must keep
    // their accent, so the override must NOT be there.
    renderWithPlatform(<FloatingToolbar />);
    expect(screen.getByRole("button", { name: "Select" }).className).toContain(
      ICON_MONO,
    );
    expect(screen.getByRole("button", { name: "Extrude" }).className).not.toContain(
      ICON_MONO,
    );
  });

  it("has no separate Chamfer button — the unified tool covers both (FILLET-CHAMFER-UNIFY W2)", () => {
    renderWithPlatform(<FloatingToolbar />);
    expect(screen.queryByRole("button", { name: "Chamfer" })).toBeNull();
  });

  it("toggles the active tool on click", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);
    await user.click(screen.getByRole("button", { name: "Extrude" }));
    expect(screen.getByRole("button", { name: "Extrude" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Select" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("swaps to the sketch tool set when entering sketch mode", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);
    // The Model "New sketch" tool enters sketch mode.
    await user.click(screen.getByRole("button", { name: "New sketch" }));

    for (const name of ["Line", "Rectangle", "Circle", "Arc", "Dimension", "Trim", "Extend", "Mirror"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    }
    // Family variants are folded behind their flyout's chevron, not buttons.
    for (const name of ["Center rectangle", "Ellipse", "3-point arc"]) {
      expect(screen.queryByRole("button", { name })).toBeNull();
    }
    expect(screen.queryByRole("button", { name: "Extrude" })).toBeNull();
    // Sketch mode defaults to the Line tool.
    expect(screen.getByRole("button", { name: "Line" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    // Active tool toggles within sketch mode too.
    await user.click(screen.getByRole("button", { name: "Circle" }));
    expect(screen.getByRole("button", { name: "Circle" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Line" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("grays out gated tools by default (only sketch2 selected) but keeps Extrude/Revolve enabled via the sketch fallback", () => {
    renderWithPlatform(<FloatingToolbar />);
    for (const name of ["Extrude", "Revolve", "Select", "New sketch", "Datum plane", "Hole", "Measure"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "false");
    }
    for (const name of ["Fillet / Chamfer", "Combine", "Shell", "Offset face", "Linear pattern", "Mirror", "Move"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "true");
    }
  });

  it("selecting a body enables the body-gated tools but not the edge/face-gated ones", () => {
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    renderWithPlatform(<FloatingToolbar />);
    for (const name of ["Combine", "Linear pattern", "Mirror", "Move"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "false");
    }
    for (const name of ["Fillet / Chamfer", "Shell", "Offset face"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "true");
    }
  });

  it("selecting an edge enables only Fillet among the gated tools", () => {
    selectionStore.getState().set([{ kind: "edge", id: "e1", bodyId: "body1" }]);
    renderWithPlatform(<FloatingToolbar />);
    expect(screen.getByRole("button", { name: "Fillet / Chamfer" })).toHaveAttribute("aria-disabled", "false");
    for (const name of ["Combine", "Shell", "Offset face", "Linear pattern", "Mirror", "Move"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "true");
    }
  });

  it("sketch tools are never gated on the model applicability matrix", async () => {
    // `mirror` is a member of BOTH unions and means different things: the model
    // one needs a body selected, the sketch one is a pointer gesture. Asking the
    // matrix about the sketch tool disabled it with nothing selected, which is
    // what `e2e/sketch-mirror.spec.ts` caught.
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);
    await user.click(screen.getByRole("button", { name: "New sketch" }));

    for (const name of ["Mirror", "Line", "Trim", "Select"]) {
      expect(screen.getByRole("button", { name }), name).toHaveAttribute(
        "aria-disabled",
        "false",
      );
    }
  });

  it("clicking a disabled button does not change the active tool", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<FloatingToolbar />);
    await user.click(screen.getByRole("button", { name: "Shell" }));
    expect(screen.getByRole("button", { name: "Select" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Shell" })).toHaveAttribute("aria-pressed", "false");
  });

  /*
   * The point of WP1. Every case below uses a tool from an owner modeling has
   * never heard of — the exact shape that used to be dropped by
   * `registryToolbar`'s reverse-map lookup and vanish from the screen while
   * sitting in the registry.
   */
  describe("a tool from a foreign owner", () => {
    const FOREIGN = addonId("com.example.foo");
    const FOREIGN_TOOL = contributionId<ToolId>(FOREIGN, "com.example.foo.tool.inspect");

    function contributeForeign(
      platform: Platform,
      overrides: Partial<ToolDefinition> = {},
    ): { activated: string[]; scope: ModuleScope } {
      const activated: string[] = [];
      const scope = platform.createScope(FOREIGN);
      scope.registerTool({
        id: FOREIGN_TOOL,
        title: "Inspect",
        icon: "select",
        shortcutLabel: "I",
        scopes: [ModelingScopes.Model],
        activate: () => {
          activated.push("activate");
        },
        deactivate: () => {},
        ...overrides,
      });
      return { activated, scope };
    }

    it("reaches the toolbar, activates on click, and highlights", async () => {
      const user = userEvent.setup();
      const platform = bootTestPlatform();
      const { activated } = contributeForeign(platform);
      renderWithPlatform(<FloatingToolbar />, { platform });

      const button = screen.getByRole("button", { name: "Inspect" });
      await user.click(button);

      expect(activated).toEqual(["activate"]);
      expect(screen.getByRole("button", { name: "Inspect" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
      // ...and modeling's own tool gave up the highlight, without either side
      // knowing about the other.
      expect(screen.getByRole("button", { name: "Select" })).toHaveAttribute(
        "aria-pressed",
        "false",
      );
    });

    it("leaves the toolbar when its registration is disposed", async () => {
      const platform = bootTestPlatform();
      const { scope } = contributeForeign(platform);
      renderWithPlatform(<FloatingToolbar />, { platform });
      expect(screen.getByRole("button", { name: "Inspect" })).toBeInTheDocument();

      await act(async () => scope.dispose());

      expect(screen.queryByRole("button", { name: "Inspect" })).toBeNull();
    });

    it("declaring NO scopes shows it in both modes", async () => {
      const user = userEvent.setup();
      const platform = bootTestPlatform();
      contributeForeign(platform, { scopes: undefined });
      renderWithPlatform(<FloatingToolbar />, { platform });

      expect(screen.getByRole("button", { name: "Inspect" })).toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "New sketch" }));
      expect(screen.getByRole("button", { name: "Inspect" })).toBeInTheDocument();
    });

    it("is grayed out with ITS OWN reason when canActivate says no", () => {
      const platform = bootTestPlatform();
      contributeForeign(platform, {
        canActivate: () => ({ enabled: false, reason: "Pick something first" }),
      });
      renderWithPlatform(<FloatingToolbar />, { platform });

      expect(screen.getByRole("button", { name: "Inspect" })).toHaveAttribute(
        "aria-disabled",
        "true",
      );
    });

    it("renders a placeholder for an icon the registry does not have", () => {
      const platform = bootTestPlatform();
      contributeForeign(platform, { icon: "no-such-glyph" });
      // The old cast (`def.icon as IconName`) made this a crash inside `Icon`.
      renderWithPlatform(<FloatingToolbar />, { platform });
      expect(screen.getByRole("button", { name: "Inspect" })).toBeInTheDocument();
    });
  });

  it("the ACTIVE tool is exempt from its own gray-out (must not disable mid-gesture)", async () => {
    const user = userEvent.setup();
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    renderWithPlatform(<FloatingToolbar />);
    await user.click(screen.getByRole("button", { name: "Combine" }));
    expect(screen.getByRole("button", { name: "Combine" })).toHaveAttribute("aria-pressed", "true");
    // Clearing the selection mid-gesture would normally gray Combine out —
    // but it's the active tool, so it stays enabled.
    act(() => selectionStore.getState().clear());
    expect(screen.getByRole("button", { name: "Combine" })).toHaveAttribute("aria-disabled", "false");
  });
});
