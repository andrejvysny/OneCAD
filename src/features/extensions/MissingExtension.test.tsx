/*
 * ADR-0005, made visible.
 *
 * The promise is that a document opens with its unknown module state intact and
 * that the user is TOLD. These pin the telling: the banner appears only when
 * something is missing, the details dialog names the module, and neither is
 * ever a blocker.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { missingModules } from "@/platform";
import { extensionsStore } from "@/stores/extensionsStore";
import { MissingExtensionBanner } from "./MissingExtensionBanner";
import { MissingExtensionDialog } from "./MissingExtensionDialog";

const MISSING = missingModules(
  [
    { moduleId: "com.vendor.robot-dynamics", schemaVersion: 3 },
    { moduleId: "onecad.modeling", schemaVersion: 1 },
  ],
  ["onecad.modeling"],
);

describe("missing-extension notice", () => {
  beforeEach(() => {
    extensionsStore.setState({
      missing: [],
      dismissed: false,
      detailsFor: null,
      managerOpen: false,
      tab: "installed",
    });
  });

  it("says nothing when every module is installed", () => {
    render(<MissingExtensionBanner />);
    expect(screen.queryByTestId("missing-extension-banner")).not.toBeInTheDocument();
  });

  it("counts only the modules this build does not have", () => {
    expect(MISSING.map((m) => m.moduleId)).toEqual(["com.vendor.robot-dynamics"]);
    extensionsStore.setState({ missing: MISSING });
    render(<MissingExtensionBanner />);
    expect(screen.getByTestId("missing-extension-banner")).toHaveTextContent(
      "This project uses 1 extension that is not installed. Its data has been preserved.",
    );
  });

  it("is dismissible — it is a notice, never a gate", async () => {
    extensionsStore.setState({ missing: MISSING });
    const user = userEvent.setup();
    render(<MissingExtensionBanner />);
    await user.click(screen.getByTestId("missing-extension-dismiss"));
    expect(screen.queryByTestId("missing-extension-banner")).not.toBeInTheDocument();
  });

  it("View details names the module and its schema version", async () => {
    extensionsStore.setState({ missing: MISSING });
    const user = userEvent.setup();
    render(
      <>
        <MissingExtensionBanner />
        <MissingExtensionDialog />
      </>,
    );
    await user.click(screen.getByTestId("missing-extension-details"));
    const dialog = screen.getByTestId("missing-extension-dialog");
    expect(dialog).toHaveTextContent("com.vendor.robot-dynamics");
    expect(dialog).toHaveTextContent("schema v3");
  });

  it("'Continue without it' just closes — nothing is discarded", async () => {
    extensionsStore.setState({ missing: MISSING, detailsFor: "com.vendor.robot-dynamics" });
    const user = userEvent.setup();
    render(<MissingExtensionDialog />);
    await user.click(screen.getByTestId("missing-extension-continue"));
    expect(screen.queryByTestId("missing-extension-dialog")).not.toBeInTheDocument();
    expect(extensionsStore.getState().missing).toHaveLength(1);
  });

  it("'Find extension' opens the manager's Browse tab", async () => {
    extensionsStore.setState({ missing: MISSING, detailsFor: "com.vendor.robot-dynamics" });
    const user = userEvent.setup();
    render(<MissingExtensionDialog />);
    await user.click(screen.getByTestId("missing-extension-find"));
    expect(extensionsStore.getState().managerOpen).toBe(true);
    expect(extensionsStore.getState().tab).toBe("browse");
  });
});
