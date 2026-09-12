/*
 * NavPill — the isolate button's three states (idle / pressed / disabled) and
 * that it drives the SAME store action ⇧I does. Zoom-to-fit + home are covered
 * through the store (viewportStore.test.ts); what matters here is that the
 * button cannot lie about isolation state.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { NavPill } from "./NavPill";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import { resetStores } from "@/test/resetStores";

const isolateButton = () => screen.getByRole("button", { name: /solat/ });

describe("NavPill isolate button", () => {
  beforeEach(() => {
    resetStores();
    setViewportEngine(null); // no engine → the preview guard reports "not hidden"
  });

  it("is disabled with no body-shaped selection and nothing to leave", () => {
    selectionStore.getState().set([{ kind: "sketch", id: "sketch2" }]);
    render(<NavPill />);
    expect(isolateButton()).toBeDisabled();
    expect(isolateButton()).toHaveAttribute("aria-pressed", "false");
  });

  it("enables on a body selection and isolates on click", async () => {
    const user = userEvent.setup();
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    render(<NavPill />);
    expect(isolateButton()).toBeEnabled();

    await user.click(isolateButton());
    expect(viewportStore.getState().isolatedBodyIds).toEqual(["body1"]);
    expect(isolateButton()).toHaveAttribute("aria-pressed", "true");
    expect(isolateButton()).toHaveAccessibleName("Exit isolation (⇧I)");
  });

  it("stays enabled while isolated even after the selection is cleared", () => {
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    render(<NavPill />);
    act(() => viewportStore.getState().isolateSelection());
    act(() => selectionStore.getState().clear());
    expect(isolateButton()).toBeEnabled(); // the way OUT must never disappear
  });

  it("clicking again leaves isolation", async () => {
    const user = userEvent.setup();
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    render(<NavPill />);
    await user.click(isolateButton());
    await user.click(isolateButton());
    expect(viewportStore.getState().isolatedBodyIds).toBeNull();
  });

  it("shares section state between its activation and adjacent controls", async () => {
    const user = userEvent.setup();
    render(<NavPill />);

    await user.click(screen.getByRole("button", { name: "Section controls" }));
    expect(screen.getByTestId("section-controls-popover")).toBeInTheDocument();
    expect(screen.getByTestId("section-plane-yz")).toBeDisabled();

    await user.click(screen.getByTestId("nav-section"));
    await user.click(screen.getByRole("button", { name: "Section controls" }));
    await user.click(screen.getByTestId("section-plane-yz"));
    expect(viewportStore.getState().section).toMatchObject({ enabled: true, plane: "YZ" });
  });

  it("opens a labelled Section controls dialog from the keyboard and returns focus", async () => {
    const user = userEvent.setup();
    render(<NavPill />);
    const trigger = screen.getByRole("button", { name: "Section controls" });
    trigger.focus();
    await user.keyboard("{Enter}");

    expect(screen.getByRole("dialog", { name: "Section controls" })).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "Section view" })).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Section controls" })).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it("shows the controller's mouse and trackpad mappings in navigation help", async () => {
    const user = userEvent.setup();
    render(<NavPill />);
    await user.click(screen.getByRole("button", { name: "Navigation help" }));

    expect(screen.getByRole("dialog", { name: "Navigation help" })).toBeInTheDocument();
    const help = screen.getByTestId("navigation-help");
    expect(help).toHaveTextContent("Middle or right drag: pan");
    expect(help).toHaveTextContent("Shift + right drag: orbit");
    expect(help).toHaveTextContent("Two-finger scroll: pan");
    expect(help).toHaveTextContent("Shift + two-finger scroll: orbit");
    expect(help).toHaveTextContent("Pinch: zoom");
  });
});
