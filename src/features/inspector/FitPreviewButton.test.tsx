import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FitPreviewButton } from "./FitPreviewButton";

const { engine, viewport } = vi.hoisted(() => {
  const engine = { fitPreview: vi.fn() };
  return { engine, viewport: { current: engine as { fitPreview: typeof engine.fitPreview } | null } };
});
vi.mock("@/viewport/engineBridge", () => ({ useViewportEngine: () => viewport.current }));

describe("FitPreviewButton", () => {
  beforeEach(() => {
    engine.fitPreview.mockReset();
    viewport.current = engine;
  });

  it("does not fit automatically and fits once after an explicit valid click", () => {
    engine.fitPreview.mockReturnValue(true);
    render(<FitPreviewButton tool="extrudeDepth" phase="armed" preview="valid" validationStatus="valid" />);
    expect(engine.fitPreview).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Fit preview" }));
    expect(engine.fitPreview).toHaveBeenCalledTimes(1);
  });

  it("disables with an honest reason while preview is pending", () => {
    render(<FitPreviewButton tool="filletRadius" phase="armed" preview="pending" validationStatus="valid" />);
    const button = screen.getByRole("button", { name: "Fit preview" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("title", "Preview is still updating");
    fireEvent.click(button);
    expect(engine.fitPreview).not.toHaveBeenCalled();
    expect(button).toHaveAttribute("aria-describedby");
    expect(document.getElementById(button.getAttribute("aria-describedby")!)).toHaveTextContent(
      "Preview is still updating",
    );
  });

  it("disables invalid and idle previews with associated reasons", () => {
    const { rerender } = render(
      <FitPreviewButton tool="hole" phase="armed" preview="invalid" validationStatus="invalid" />,
    );
    expect(screen.getByRole("button", { name: "Fit preview" })).toHaveAttribute("title", "Preview is invalid");
    rerender(<FitPreviewButton tool="hole" phase="armed" preview="none" validationStatus="valid" />);
    expect(screen.getByRole("button", { name: "Fit preview" })).toHaveAttribute(
      "title",
      "No current preview geometry to frame",
    );
  });

  it("disables honestly when the viewport engine is unavailable", () => {
    viewport.current = null;
    render(<FitPreviewButton tool="hole" phase="armed" preview="valid" validationStatus="valid" />);
    const button = screen.getByRole("button", { name: "Fit preview" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("title", "Viewport unavailable");
    expect(button).toHaveAttribute("aria-describedby");
  });

  it("reports a refused fit without falling back to fit-all", () => {
    engine.fitPreview.mockReturnValue(false);
    render(<FitPreviewButton tool="booleanOp" phase="armed" preview="valid" validationStatus="valid" />);
    fireEvent.click(screen.getByRole("button", { name: "Fit preview" }));
    expect(screen.getByRole("status")).toHaveTextContent("No current preview geometry to frame");
    expect(engine.fitPreview).toHaveBeenCalledTimes(1);
  });

  it("clears refusal feedback when the preview state changes", () => {
    engine.fitPreview.mockReturnValue(false);
    const { rerender } = render(
      <FitPreviewButton tool="hole" phase="armed" preview="valid" validationStatus="valid" />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Fit preview" }));
    expect(screen.getByRole("status")).toBeInTheDocument();
    rerender(<FitPreviewButton tool="hole" phase="armed" preview="pending" validationStatus="pending" />);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("omits unsupported tools and disables when applying", () => {
    expect(render(<FitPreviewButton tool="dimension" phase="armed" preview="valid" validationStatus="valid" />).container).toBeEmptyDOMElement();
    render(<FitPreviewButton tool="hole" phase="applying" preview="valid" validationStatus="valid" />);
    expect(screen.getByRole("button", { name: "Fit preview" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Fit preview" })).toHaveAttribute("title", "Applying operation");
  });

  it("omits terminal controls explicitly", () => {
    expect(
      render(<FitPreviewButton tool="hole" phase="armed" preview="valid" validationStatus="valid" terminal />).container,
    ).toBeEmptyDOMElement();
  });
});
