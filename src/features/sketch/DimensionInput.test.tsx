/*
 * DimensionInput — commit-time validation for dimensional constraint kinds
 * (ConstraintBadgeLayer wiring). Reject must NOT call onCommit and must flash
 * an error style on the input; back-compat (no `kind`) stays finite-only.
 */
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { settingsStore } from "@/stores/settingsStore";
import { DEFAULT_LENGTH_UNIT, type LengthUnitId } from "@/units/lengthUnits";
import { DimensionInput } from "./DimensionInput";

function input() {
  return screen.getByLabelText("Dimension value") as HTMLInputElement;
}

function setUnit(unit: LengthUnitId) {
  act(() => settingsStore.getState().setDisplayUnit(unit));
}

afterEach(() => {
  act(() => settingsStore.setState({ displayUnit: DEFAULT_LENGTH_UNIT }));
  localStorage.clear();
});

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

describe("DimensionInput — formatted no-op commit gate", () => {
  it("untouched blur of 12.345 does not call onCommit (no-op gate)", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={12.345} kind="Distance" onCommit={onCommit} />);
    fireEvent.blur(input());
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("untouched blur when the display value is rounded (12.3456 → \"12.346\") does not spuriously re-commit the rounded string as a new value", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={12.3456} kind="Distance" onCommit={onCommit} />);
    expect(input().value).toBe("12.346");
    fireEvent.blur(input());
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("a real edit that only changes precision beyond 3 decimals still commits the typed value", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={12.345} kind="Distance" onCommit={onCommit} />);
    typeAndEnter("12.7");
    expect(onCommit).toHaveBeenCalledWith(12.7);
  });
});

/*
 * WP-C2 — display unit.
 *
 * `value` is ALWAYS millimetres and `onCommit` ALWAYS receives millimetres.
 * The preference moves the FIELD only. The pin that matters most is the last
 * one: switching units must not dirty a document.
 */
describe("DimensionInput — display unit", () => {
  it("shows the stored millimetres in the current unit, with that unit's symbol", () => {
    setUnit("in");
    render(<DimensionInput value={25.4} suffix="mm" kind="Distance" onCommit={() => {}} />);
    expect(input().value).toBe("1");
    expect(screen.getByText("in")).toBeInTheDocument();
    expect(screen.queryByText("mm")).toBeNull();
  });

  it("re-renders the SAME value when the unit changes mid-life", () => {
    render(<DimensionInput value={25.4} suffix="mm" kind="Distance" onCommit={() => {}} />);
    expect(input().value).toBe("25.4");
    setUnit("in");
    expect(input().value).toBe("1");
    setUnit("cm");
    expect(input().value).toBe("2.54");
    setUnit("mm");
    expect(input().value).toBe("25.4");
  });

  it("a BARE typed number is read in the display unit and commits MILLIMETRES", () => {
    const onCommit = vi.fn();
    setUnit("in");
    render(<DimensionInput value={10} kind="Distance" onCommit={onCommit} />);
    typeAndEnter("2");
    expect(onCommit).toHaveBeenCalledWith(50.8);
  });

  it("an explicit suffix overrides the display unit (still millimetres out)", () => {
    const onCommit = vi.fn();
    setUnit("in");
    render(<DimensionInput value={10} kind="Distance" onCommit={onCommit} />);
    typeAndEnter("2 mm");
    expect(onCommit).toHaveBeenCalledWith(2);
  });

  it("the SAME typed suffixed text commits the SAME millimetres in every unit", () => {
    for (const unit of ["mm", "cm", "m", "in"] as LengthUnitId[]) {
      const onCommit = vi.fn();
      setUnit(unit);
      const { unmount } = render(
        <DimensionInput value={999} kind="Distance" onCommit={onCommit} />,
      );
      typeAndEnter("25 mm");
      expect(onCommit).toHaveBeenCalledWith(25);
      unmount();
    }
  });

  it("an ANGLE chip ignores the display unit entirely (degrees stay degrees)", () => {
    const onCommit = vi.fn();
    setUnit("in");
    render(<DimensionInput value={45} kind="Angle" onCommit={onCommit} />);
    expect(input().value).toBe("45");
    typeAndEnter("90");
    expect(onCommit).toHaveBeenCalledWith(90);
  });

  it("an angle chip marked by its ° suffix keeps that suffix", () => {
    setUnit("in");
    render(<DimensionInput value={45} suffix="°" onCommit={() => {}} />);
    expect(input().value).toBe("45");
    expect(screen.getByText("°")).toBeInTheDocument();
  });

  /*
   * THE PIN. Changing only the display unit is a re-display, never an edit: no
   * onCommit, in either direction, including a blur while the field shows the
   * converted value. If this ever fires, a user idly switching units would
   * rewrite every dimension in the document.
   */
  it("switching the unit never commits — not on switch, not on a following blur", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={25.4} suffix="mm" kind="Distance" onCommit={onCommit} />);

    setUnit("in");
    expect(onCommit).not.toHaveBeenCalled();

    fireEvent.blur(input());
    expect(onCommit).not.toHaveBeenCalled();

    setUnit("mm");
    fireEvent.blur(input());
    expect(onCommit).not.toHaveBeenCalled();
    expect(input().value).toBe("25.4");
  });

  it("an untouched blur does not re-commit even when the display rounds the value", () => {
    const onCommit = vi.fn();
    setUnit("in");
    // 1 mm is 0.0394 in at four decimals — a lossy display. The no-op gate
    // compares FORMATTED strings, so the rounding cannot look like an edit.
    render(<DimensionInput value={1} suffix="mm" kind="Distance" onCommit={onCommit} />);
    expect(input().value).toBe("0.0394");
    fireEvent.blur(input());
    expect(onCommit).not.toHaveBeenCalled();
  });
});
