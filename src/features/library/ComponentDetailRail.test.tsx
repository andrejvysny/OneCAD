/*
 * The library detail panel's configurator (LGU-1 WP-A, canvas D2-A).
 *
 * What is worth pinning here is the PROMISE the panel makes: what you configure
 * is what gets placed, and what it says about the component is true. The 3D
 * preview is mocked — it needs a WebGL context jsdom does not have, and the
 * thing under test is the copy and the carry, not the renderer.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ComponentDetailRail } from "./ComponentDetailRail";
import { componentConfigStore } from "@/modules/library/componentConfigStore";
import type { LibraryComponent } from "@/ipc/types";

vi.mock("./ComponentPreview3D", () => ({
  ComponentPreview3D: () => <div data-testid="preview" />,
}));

const SHCS: LibraryComponent = {
  id: "onecad.std.iso4762",
  version: "1.0.0",
  name: "Socket Head Cap Screw",
  category: ["fastener"],
  tags: ["metric", "shcs"],
  sourceKind: "generator",
  revision: "sha256:0",
  attachments: {
    head_seat: { on: "face:head_underside", accepts: ["plane"] },
    shank_axis: { on: "cylinder:shank", accepts: ["cylinder", "hole"] },
  },
  parameters: {
    thread: { role: "free", key: "M6", domain: ["M2", "M4", "M6", "M8", "M12"] },
    length: { role: "free", value: 20 },
    head_d: { role: "table", from: "head_diameter" },
  },
  designation: "ISO 4762 {thread}x{length}",
};

function renderRail(over: Partial<LibraryComponent> = {}, canPlace = true) {
  const onPlace = vi.fn();
  render(<ComponentDetailRail component={{ ...SHCS, ...over }} canPlace={canPlace} onPlace={onPlace} />);
  return onPlace;
}

describe("ComponentDetailRail", () => {
  beforeEach(() => componentConfigStore.setState({ byComponent: {} }));

  it("opens at the package's own defaults", () => {
    renderRail();
    expect(screen.getByTestId("detail-designation")).toHaveTextContent("ISO 4762 M6x20");
  });

  it("re-renders the designation live as the configuration changes", () => {
    renderRail();
    fireEvent.change(screen.getByLabelText("thread value"), { target: { value: "M8" } });
    expect(screen.getByTestId("detail-designation")).toHaveTextContent("ISO 4762 M8x20");

    fireEvent.change(screen.getByLabelText("length value"), { target: { value: "25" } });
    expect(screen.getByTestId("detail-designation")).toHaveTextContent("ISO 4762 M8x25");
  });

  /*
   * The panel's whole point: configure, THEN place. Handing the placement the
   * package defaults instead would make every choice made here a lie.
   */
  it("hands the configured values to the placement, not the defaults", () => {
    const onPlace = renderRail();
    fireEvent.change(screen.getByLabelText("thread value"), { target: { value: "M8" } });
    fireEvent.click(screen.getByText("Place in scene"));

    expect(onPlace).toHaveBeenCalledTimes(1);
    expect(onPlace.mock.calls[0][1]).toMatchObject({ thread: "M8", length: 20 });
  });

  it("carries a configuration so it survives closing and reopening the panel", () => {
    renderRail();
    fireEvent.change(screen.getByLabelText("thread value"), { target: { value: "M12" } });
    expect(componentConfigStore.getState().get("onecad.std.iso4762", "1.0.0")).toMatchObject({
      thread: "M12",
    });
  });

  it("offers no control for a table-driven param — only free ones are the user's", () => {
    renderRail();
    expect(screen.getByLabelText("thread value")).toBeInTheDocument();
    expect(screen.queryByLabelText("head_d value")).toBeNull();
  });

  it("says what the component attaches to in prose, not in manifest keys", () => {
    renderRail();
    const rows = screen.getAllByTestId("detail-attachment").map((r) => r.textContent);
    expect(rows).toEqual(["Head seat → flat faces", "Shank axis → holes & cylinders"]);
  });

  /* The standing rule — see `componentMeta.test.ts`. */
  it("never calls the component a generator", () => {
    renderRail();
    expect(screen.getByText(/Parametric, generated on demand/)).toBeInTheDocument();
    expect(document.body.textContent?.toLowerCase()).not.toContain("generator");
  });

  it("says fixed geometry has nothing to configure rather than showing an empty section", () => {
    renderRail({ parameters: {}, designation: undefined });
    expect(screen.getByText("Fixed geometry — nothing to configure.")).toBeInTheDocument();
    expect(screen.queryByTestId("component-config-row")).toBeNull();
  });
});
