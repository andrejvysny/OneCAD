/*
 * The manager's honesty rules.
 *
 * "Installed" describes the REAL build (modules + what each contributed);
 * "Browse" and "Updates" have no registry behind them and must say so rather
 * than show a mock catalog. A fake listing here would outlive the mock and
 * teach users that install buttons do nothing.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { missingModules } from "@/platform";
import { extensionsStore } from "@/stores/extensionsStore";
import { renderWithPlatform } from "@/test/renderWithPlatform";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";
import { ExtensionsManager } from "./ExtensionsManager";

describe("ExtensionsManager", () => {
  beforeEach(() => {
    extensionsStore.setState({
      managerOpen: true,
      tab: "installed",
      missing: [],
      dismissed: false,
      detailsFor: null,
    });
  });

  it("renders nothing while closed", () => {
    extensionsStore.setState({ managerOpen: false });
    renderWithPlatform(<ExtensionsManager />);
    expect(screen.queryByTestId("extensions-manager")).not.toBeInTheDocument();
  });

  it("lists the modules this build actually registered", () => {
    renderWithPlatform(<ExtensionsManager />);
    expect(screen.getByTestId(`extension-card-${MODELING_MODULE_ID}`)).toBeInTheDocument();
  });

  it("describes a module by what it contributed", () => {
    renderWithPlatform(<ExtensionsManager />);
    // The modeling module registers tools and commands at bootstrap; the card
    // counts registrations rather than reciting a manifest.
    expect(screen.getByTestId(`extension-card-${MODELING_MODULE_ID}`)).toHaveTextContent(/Adds:/);
  });

  it("says there is no registry instead of showing a fake catalog", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<ExtensionsManager />);
    await user.click(screen.getByTestId("extensions-tab-browse"));
    expect(screen.getByTestId("extensions-empty")).toHaveTextContent(
      "No extension registry configured",
    );
    // No install affordance at all — a dead button is worse than none.
    expect(screen.queryByRole("button", { name: /install/i })).not.toBeInTheDocument();
  });

  it("says the same about updates", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<ExtensionsManager />);
    await user.click(screen.getByTestId("extensions-tab-updates"));
    expect(screen.getByTestId("extensions-empty")).toHaveTextContent("No updates to check");
  });

  it("surfaces the document's missing modules alongside the installed ones", () => {
    extensionsStore.setState({
      missing: missingModules([{ moduleId: "com.vendor.robot", schemaVersion: 2 }], []),
    });
    renderWithPlatform(<ExtensionsManager />);
    expect(screen.getByTestId("extension-missing-com.vendor.robot")).toHaveTextContent(
      "preserved and saved back unchanged",
    );
  });

  it("filters the installed list", async () => {
    const user = userEvent.setup();
    renderWithPlatform(<ExtensionsManager />);
    await user.type(screen.getByLabelText("Search extensions"), "nothing-matches-this");
    expect(screen.getByTestId("extensions-empty")).toHaveTextContent("No matching extensions");
  });
});
