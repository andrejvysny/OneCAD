/*
 * The library card's picture (WP-B6): shows the rendered component when there
 * is one, and the generic icon when there is not.
 *
 * The fallback is the point. A component the kernel cannot build, a machine
 * with no WebGL, a worker that is not up — none of those may leave a hole in
 * the grid, because the card is still how a user searches for, selects and
 * places that component.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { ComponentThumbnail } from "./ComponentThumbnail";

const componentThumbnail = vi.fn();

vi.mock("./componentPreviewScene", () => ({
  componentThumbnail: (...args: unknown[]) => componentThumbnail(...args),
}));

describe("ComponentThumbnail", () => {
  beforeEach(() => {
    componentThumbnail.mockReset();
  });

  it("renders the rasterized preview once it resolves", async () => {
    componentThumbnail.mockResolvedValue("data:image/png;base64,AAAA");
    render(<ComponentThumbnail componentId="a.b" componentVersion="1.0.0" />);

    await waitFor(() =>
      expect(screen.getByTestId("component-thumbnail")).toHaveAttribute(
        "src",
        "data:image/png;base64,AAAA",
      ),
    );
  });

  it("falls back to the icon when there is no preview to show", async () => {
    componentThumbnail.mockResolvedValue(null);
    render(<ComponentThumbnail componentId="a.b" componentVersion="1.0.0" />);

    await waitFor(() =>
      expect(screen.getByTestId("component-thumbnail-fallback")).toBeInTheDocument(),
    );
    expect(screen.queryByTestId("component-thumbnail")).toBeNull();
  });

  it("asks for the component it was given, at the size it was given", async () => {
    componentThumbnail.mockResolvedValue(null);
    render(<ComponentThumbnail componentId="mine.bracket" componentVersion="2.1.0" size={192} />);
    await waitFor(() =>
      expect(componentThumbnail).toHaveBeenCalledWith("mine.bracket", "2.1.0", undefined, 192),
    );
  });
});
