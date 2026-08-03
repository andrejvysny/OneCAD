import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StatusBar } from "./StatusBar";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
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

  /*
   * WP-C2: the store holds mm; the row is display-only. The unit is named once
   * at the end of the row rather than on every axis, and the columns keep their
   * fixed width so the read-out does not jitter as the pointer moves.
   */
  it("renders the cursor read-out in the display unit, named once", () => {
    render(<StatusBar />);
    // resetStores() seeds cursor = (273, 210, 0) mm at the mm default.
    expect(screen.getByText(/X\s+273\.00\s+Y\s+210\.00\s+Z\s+0\.00\s+mm/)).toBeInTheDocument();

    act(() => settingsStore.getState().setDisplayUnit("cm"));
    expect(screen.getByText(/X\s+27\.300\s+Y\s+21\.000\s+Z\s+0\.000\s+cm/)).toBeInTheDocument();

    act(() => settingsStore.getState().setDisplayUnit("in"));
    expect(screen.getByText(/10\.748.*in/)).toBeInTheDocument();

    act(() => settingsStore.getState().setDisplayUnit("mm"));
    expect(screen.getByText(/273\.00\s.*mm/)).toBeInTheDocument();
  });
});
