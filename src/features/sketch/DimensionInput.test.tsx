/*
 * DimensionInput — commit-time validation for dimensional constraint kinds
 * (ConstraintBadgeLayer wiring). Reject must NOT call onCommit and must flash
 * an error style on the input; back-compat (no `kind`) stays finite-only.
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DimensionInput } from "./DimensionInput";

function input() {
  return screen.getByLabelText("Dimension value") as HTMLInputElement;
}

function typeAndEnter(text: string) {
  fireEvent.change(input(), { target: { value: text } });
  fireEvent.keyDown(input(), { key: "Enter" });
}

describe("DimensionInput — kind-aware validation", () => {
  it("Radius ≤0 is rejected: no onCommit, error style appears", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={5} kind="Radius" onCommit={onCommit} />);
    typeAndEnter("0");
    expect(onCommit).not.toHaveBeenCalled();
    expect(input()).toHaveClass("text-traffic-close");
    expect(input()).toHaveAttribute("aria-invalid", "true");
  });

  it("Radius negative is rejected too", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={5} kind="Radius" onCommit={onCommit} />);
    typeAndEnter("-3");
    expect(onCommit).not.toHaveBeenCalled();
    expect(input()).toHaveAttribute("aria-invalid", "true");
  });

  it("Radius positive commits", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={5} kind="Radius" onCommit={onCommit} />);
    typeAndEnter("12.5");
    expect(onCommit).toHaveBeenCalledWith(12.5);
    expect(input()).toHaveAttribute("aria-invalid", "false");
  });

  it("Angle 0 is rejected", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={45} kind="Angle" onCommit={onCommit} />);
    typeAndEnter("0");
    expect(onCommit).not.toHaveBeenCalled();
    expect(input()).toHaveAttribute("aria-invalid", "true");
  });

  it("Angle 360 is rejected (exclusive upper bound)", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={45} kind="Angle" onCommit={onCommit} />);
    typeAndEnter("360");
    expect(onCommit).not.toHaveBeenCalled();
    expect(input()).toHaveAttribute("aria-invalid", "true");
  });

  it("Angle 400 is rejected (out of range)", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={45} kind="Angle" onCommit={onCommit} />);
    typeAndEnter("400");
    expect(onCommit).not.toHaveBeenCalled();
    expect(input()).toHaveAttribute("aria-invalid", "true");
  });

  it("Angle 45 commits", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={90} kind="Angle" onCommit={onCommit} />);
    typeAndEnter("45");
    expect(onCommit).toHaveBeenCalledWith(45);
  });

  it("HorizontalDistance accepts a negative value (signed domain)", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={0} kind="HorizontalDistance" onCommit={onCommit} />);
    typeAndEnter("-12.5");
    expect(onCommit).toHaveBeenCalledWith(-12.5);
    expect(input()).toHaveAttribute("aria-invalid", "false");
  });

  it("VerticalDistance accepts zero (signed domain)", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={5} kind="VerticalDistance" onCommit={onCommit} />);
    typeAndEnter("0");
    expect(onCommit).toHaveBeenCalledWith(0);
  });

  it("no kind: any finite value commits (back-compat, model-tool chips)", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={5} onCommit={onCommit} />);
    typeAndEnter("-3");
    expect(onCommit).toHaveBeenCalledWith(-3);
  });

  it("no kind: a Radius-invalid value (e.g. 0) still commits — no constraint-specific validation", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={5} onCommit={onCommit} />);
    typeAndEnter("0");
    expect(onCommit).toHaveBeenCalledWith(0);
  });
});
