/*
 * The layers menu is a VIEW filter, never a document write. These pin that
 * boundary: flipping a layer moves `layersStore` (and, for the grid, the
 * viewport store it already shared), and nothing reaches per-entity visibility.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createRef } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { layersStore } from "@/stores/layersStore";
import { SECTION_DEFAULT, viewportStore } from "@/stores/viewportStore";
import { documentStore } from "@/stores/documentStore";
import { settingsStore } from "@/stores/settingsStore";
import { mockClient } from "@/ipc/mockClient";
import { setViewportEngine } from "@/viewport/engineBridge";
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
    setViewportEngine(null);
    viewportStore.setState({ section: SECTION_DEFAULT });
    layersStore.setState({ open: false, visible: { bodies: true, sketches: true, datums: true } });
  });

  it("renders nothing until it is opened", () => {
    const ref = createRef<HTMLButtonElement>();
    render(<LayersMenu anchorRef={ref} />);
    expect(screen.queryByTestId("layers-menu")).not.toBeInTheDocument();
  });

  it("offers only layers the viewport can actually draw", () => {
    open();
    expect(screen.getByRole("dialog", { name: "Viewport layers" })).toBeInTheDocument();
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

  it("uses a native keyboard-reachable checkbox for the section toggle", async () => {
    const user = userEvent.setup();
    open();
    const section = screen.getByRole("checkbox", { name: "Section view" });
    expect(section.tagName).toBe("BUTTON");
    section.focus();
    await user.keyboard(" ");
    expect(viewportStore.getState().section.enabled).toBe(true);
    expect(section).toHaveAttribute("aria-checked", "true");
    expect(screen.getByTestId("section-offset")).toBeDisabled();
    expect(screen.getByTestId("section-offset")).not.toHaveAttribute("min");
  });

  it("keeps an invalid section offset draft visible without mutating the viewport", () => {
    open();
    act(() => viewportStore.getState().toggleSection());
    const field = screen.getByTestId("section-offset-input");
    fireEvent.change(field, { target: { value: "not a length" } });
    fireEvent.blur(field);

    expect(field).toHaveValue("not a length");
    expect(field).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByRole("alert")).toHaveTextContent("Enter a valid length.");
    expect(viewportStore.getState().section.offsetMm).toBe(0);
  });

  it("does not replace an untouched precise offset with its rounded display text", () => {
    open();
    act(() => viewportStore.setState({ section: { enabled: true, plane: "XY", offsetMm: 1.234567, flip: false } }));
    const field = screen.getByTestId("section-offset-input");
    fireEvent.focus(field);
    fireEvent.blur(field);

    expect(viewportStore.getState().section.offsetMm).toBe(1.234567);
  });

  it("parses a display-unit section offset and never calls a client mutation", async () => {
    const user = userEvent.setup();
    const apply = vi.spyOn(mockClient, "applyOperation");
    settingsStore.getState().setDisplayUnit("in");
    open();
    act(() => viewportStore.getState().toggleSection());
    const field = screen.getByTestId("section-offset-input");
    expect(field).toBeEnabled();
    await user.clear(field);
    await user.type(field, "2");
    await user.keyboard("{Enter}");
    await user.click(screen.getByTestId("section-flip"));

    expect(viewportStore.getState().section).toMatchObject({ offsetMm: 50.8, flip: true });
    expect(apply).not.toHaveBeenCalled();
  });
});
