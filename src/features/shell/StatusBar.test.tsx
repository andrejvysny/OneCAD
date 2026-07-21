import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StatusBar } from "./StatusBar";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

describe("StatusBar", () => {
  beforeEach(() => resetStores());

  it("shows DOF for a selected sketch, — for a body, and the mono read-out", () => {
    render(<StatusBar />);
    expect(screen.getByText("DOF: 0")).toBeInTheDocument();
    expect(screen.getByText(/273\.00/)).toBeInTheDocument();

    act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));
    expect(screen.getByText("DOF: —")).toBeInTheDocument();
  });

  it("styles an error hint red and an info hint dimmed", () => {
    render(<StatusBar />);

    act(() =>
      viewportStore.getState().setStatusHint("Extrude failed: boom", { severity: "error", sticky: true }),
    );
    expect(screen.getByText("Extrude failed: boom")).toHaveClass("text-traffic-close");

    act(() => viewportStore.getState().setStatusHint("Select a plane to start", { sticky: true }));
    const info = screen.getByText("Select a plane to start");
    expect(info).toHaveClass("text-ink-5");
    expect(info).not.toHaveClass("text-traffic-close");
  });

  it("toggles projection and dims FOV in ortho", async () => {
    const user = userEvent.setup();
    render(<StatusBar />);

    expect(screen.getByTestId("fov")).toHaveStyle({ opacity: "1" });
    await user.click(screen.getByRole("tab", { name: "Ortho" }));
    expect(screen.getByTestId("fov")).toHaveStyle({ opacity: "0.35" });
    await user.click(screen.getByRole("tab", { name: "Persp" }));
    expect(screen.getByTestId("fov")).toHaveStyle({ opacity: "1" });
  });
});
