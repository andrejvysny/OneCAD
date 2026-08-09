/*
 * The layers menu is a VIEW filter, never a document write. These pin that
 * boundary: flipping a layer moves `layersStore` (and, for the grid, the
 * viewport store it already shared), and nothing reaches per-entity visibility.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { layersStore } from "@/stores/layersStore";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore } from "@/stores/documentStore";
import { resetStores } from "@/test/resetStores";
import { LayersMenu } from "./LayersMenu";

function open() {
  const ref = createRef<HTMLButtonElement>();
  layersStore.setState({ open: true });
  render(
    <>
      <button ref={ref} type="button">
        anchor
      </button>
      <LayersMenu anchorRef={ref} />
    </>,
  );
}

describe("LayersMenu", () => {
  beforeEach(() => {
    localStorage.clear();
    resetStores();
    layersStore.setState({ open: false, visible: { bodies: true, sketches: true, datums: true } });
  });

  it("renders nothing until it is opened", () => {
    const ref = createRef<HTMLButtonElement>();
    render(<LayersMenu anchorRef={ref} />);
    expect(screen.queryByTestId("layers-menu")).not.toBeInTheDocument();
  });

  it("offers only layers the viewport can actually draw", () => {
    open();
    for (const key of ["bodies", "sketches", "datums", "grid"]) {
      expect(screen.getByTestId(`layer-${key}`)).toBeInTheDocument();
    }
    // No fabricated groups for modules that do not exist.
    expect(screen.queryByText("Simulation")).not.toBeInTheDocument();
    expect(screen.queryByText("Add-ons")).not.toBeInTheDocument();
  });

  it("flips a scene layer without touching document visibility", async () => {
    const user = userEvent.setup();
    const setVisibility = vi.spyOn(documentStore.getState(), "setVisibility");
    open();
    await user.click(screen.getByTestId("layer-sketches"));
    expect(layersStore.getState().visible.sketches).toBe(false);
    expect(setVisibility).not.toHaveBeenCalled();
  });

  it("drives the ONE grid flag, not a second copy of it", async () => {
    const user = userEvent.setup();
    open();
    const before = viewportStore.getState().gridVisible;
    await user.click(screen.getByTestId("layer-grid"));
    expect(viewportStore.getState().gridVisible).toBe(!before);
  });

  it("reports its state through aria-checked", async () => {
    const user = userEvent.setup();
    open();
    expect(screen.getByTestId("layer-bodies")).toHaveAttribute("aria-checked", "true");
    await user.click(screen.getByTestId("layer-bodies"));
    expect(screen.getByTestId("layer-bodies")).toHaveAttribute("aria-checked", "false");
  });
});
