/*
 * DimensionInput — commit-time validation for dimensional constraint kinds
 * (ConstraintBadgeLayer wiring). Reject must NOT call onCommit and must flash
 * an error style on the input; back-compat (no `kind`) stays finite-only.
 */
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import { settingsStore } from "@/stores/settingsStore";
import { DEFAULT_LENGTH_UNIT, type LengthUnitId } from "@/units/lengthUnits";
import { mockClient, resetMockDocument, setMockLatency } from "@/ipc/mockClient";
import type { DimensionInputProps } from "./DimensionInput";
import { DimensionInput, parseExprInput } from "./DimensionInput";

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

// ── WP-VE.2: the OPT-IN variable-binding lane ────────────────────────────────

describe("parseExprInput — the `=` syntax", () => {
  it("returns the expression text after `=`, with or without surrounding space", () => {
    expect(parseExprInput("=height")).toBe("height");
    expect(parseExprInput("  = height ")).toBe("height");
    expect(parseExprInput("=_w2")).toBe("_w2");
  });

  /*
   * It is SYNTAX, not validation. `=w * 2` and `=2w` are now handed to the
   * evaluator, which is the only thing entitled to say whether they resolve —
   * a second grammar here would drift from `onecad-core`'s.
   */
  it("passes arithmetic and unit suffixes straight through to the evaluator", () => {
    expect(parseExprInput("=w * 2")).toBe("w * 2");
    expect(parseExprInput("=a-b")).toBe("a-b");
    expect(parseExprInput("=5mm + depth")).toBe("5mm + depth");
    expect(parseExprInput("=2w")).toBe("2w");
  });

  it("rejects text that is not an `=` entry at all, and a bare `=`", () => {
    for (const bad of ["=", "= ", "25", ""]) {
      expect(parseExprInput(bad)).toBeNull();
    }
  });
});

/*
 * The `=` EXPRESSION lane, against the REAL mock client — which resolves
 * through the shared TS port of `onecad-core`'s evaluator, so these prove the
 * field against the semantics the backend applies rather than against a stub.
 */
describe("DimensionInput — expression mode", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  function renderBindable(props: Partial<DimensionInputProps> = {}) {
    const onCommit = vi.fn();
    const onCommitExpr = vi.fn();
    render(
      <DimensionInput
        value={25}
        suffix="mm"
        onCommit={onCommit}
        onCommitExpr={onCommitExpr}
        {...props}
      />,
    );
    return { onCommit, onCommitExpr };
  }

  function preview() {
    return screen.getByTestId("dimension-expr-preview");
  }

  it("without onCommitExpr, `=name` is unparseable text and commits nothing", () => {
    const onCommit = vi.fn();
    render(<DimensionInput value={25} suffix="mm" onCommit={onCommit} />);
    typeAndEnter("=height");
    // The sketch-badge case: those values have no `Scalar` behind them, so a
    // binding would be a promise the backend has nowhere to keep.
    expect(onCommit).not.toHaveBeenCalled();
    expect(input().value).toBe("25");
    expect(screen.queryByTestId("dimension-expr-preview")).not.toBeInTheDocument();
  });

  it("renders an existing binding as `=<expr>` rather than its resolved number", () => {
    render(
      <DimensionInput value={45} suffix="mm" expr="w*2+5mm" onCommit={vi.fn()} onCommitExpr={vi.fn()} />,
    );
    expect(input().value).toBe("=w*2+5mm");
  });

  it("previews the resolved value while typing, then commits THAT number", async () => {
    await mockClient.upsertVariable("w", 20);
    const { onCommit, onCommitExpr } = renderBindable();

    fireEvent.change(input(), { target: { value: "=w*2+5mm" } });
    await waitFor(() => expect(preview()).toHaveTextContent("= 45 mm"));

    fireEvent.keyDown(input(), { key: "Enter" });
    // The EVALUATOR's number, not the field's own parse and not the `value`
    // prop it was seeded with.
    await waitFor(() => expect(onCommitExpr).toHaveBeenCalledWith("w*2+5mm", 45));
    expect(onCommit).not.toHaveBeenCalled();
  });

  /* Enter can beat the 80 ms debounce, and the commit must still be the
   * evaluator's number — never the stale one behind the field. */
  it("flushes the pending evaluation when Enter arrives before the debounce", async () => {
    await mockClient.upsertVariable("w", 20);
    const { onCommitExpr } = renderBindable();
    typeAndEnter("=w*2");
    await waitFor(() => expect(onCommitExpr).toHaveBeenCalledWith("w*2", 40));
  });

  it("shows the evaluator's refusal inline and commits nothing", async () => {
    const { onCommit, onCommitExpr } = renderBindable();
    fireEvent.change(input(), { target: { value: "=gone*2" } });
    await waitFor(() => expect(preview()).toHaveTextContent(/undefined variable/));

    fireEvent.keyDown(input(), { key: "Enter" });
    await waitFor(() => expect(input()).toHaveAttribute("aria-invalid", "true"));
    expect(onCommit).not.toHaveBeenCalled();
    expect(onCommitExpr).not.toHaveBeenCalled();
  });

  /* The site dimension is what makes this a refusal instead of a silent 45 mm. */
  it("refuses an ANGLE expression in a length field, inline", async () => {
    const { onCommitExpr } = renderBindable();
    fireEvent.change(input(), { target: { value: "=45deg" } });
    await waitFor(() => expect(preview()).toHaveTextContent(/expected length/));
    fireEvent.keyDown(input(), { key: "Enter" });
    await waitFor(() => expect(input()).toHaveAttribute("aria-invalid", "true"));
    expect(onCommitExpr).not.toHaveBeenCalled();
  });

  it("an ANGLE field evaluates at the angle site and previews degrees", async () => {
    const onCommitExpr = vi.fn();
    render(
      <DimensionInput value={30} suffix="°" onCommit={vi.fn()} onCommitExpr={onCommitExpr} />,
    );
    fireEvent.change(input(), { target: { value: "=15deg*3" } });
    await waitFor(() => expect(preview()).toHaveTextContent("= 45°"));
    fireEvent.keyDown(input(), { key: "Enter" });
    await waitFor(() => expect(onCommitExpr).toHaveBeenCalledWith("15deg*3", 45));
  });

  it("clears the binding when a plain number is typed over it", () => {
    const { onCommit, onCommitExpr } = renderBindable({ expr: "height" });
    typeAndEnter("40");
    expect(onCommitExpr).toHaveBeenCalledWith(null, 40);
    expect(onCommit).not.toHaveBeenCalled();
  });

  /** Unbinding IS the edit, even when the number itself does not move. */
  it("clears the binding even when the typed number is unchanged", () => {
    const { onCommitExpr } = renderBindable({ expr: "height" });
    typeAndEnter("25");
    expect(onCommitExpr).toHaveBeenCalledWith(null, 25);
  });

  it("a bare `=` is refused rather than silently re-committing the old number", () => {
    const { onCommit, onCommitExpr } = renderBindable();
    typeAndEnter("=");
    expect(onCommit).not.toHaveBeenCalled();
    expect(onCommitExpr).not.toHaveBeenCalled();
    expect(input()).toHaveAttribute("aria-invalid", "true");
  });

  it("re-typing the SAME binding is a no-op, not a redundant edit", () => {
    const { onCommitExpr } = renderBindable({ expr: "height" });
    typeAndEnter("=height");
    expect(onCommitExpr).not.toHaveBeenCalled();
  });
});

/*
 * The two authoring guardrails around "a bare number inside `=` is
 * millimetres". They exist because that rule DISAGREES with the field's other
 * rule — a bare number typed on its own is read in the display unit — and the
 * disagreement is invisible while millimetres are displayed.
 */
describe("DimensionInput — expression unit guardrails", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  it("commits `=2` through the PLAIN path, in the display unit, with no binding", async () => {
    setUnit("in");
    const onCommit = vi.fn();
    const onCommitExpr = vi.fn();
    render(
      <DimensionInput value={25} suffix="mm" onCommit={onCommit} onCommitExpr={onCommitExpr} />,
    );
    typeAndEnter("=2");
    // 2 INCHES — identical to typing `2`. A pure literal is never recorded as
    // an expression, so the two spellings cannot mean different numbers.
    expect(onCommit).toHaveBeenCalledWith(50.8);
    expect(onCommitExpr).not.toHaveBeenCalled();
    // …and it never opened the expression lane at all.
    expect(screen.queryByTestId("dimension-expr-preview")).not.toBeInTheDocument();
  });

  it("a pure literal over a BOUND field unbinds it, in the display unit", () => {
    setUnit("in");
    const onCommitExpr = vi.fn();
    render(
      <DimensionInput value={25} suffix="mm" expr="height" onCommit={vi.fn()} onCommitExpr={onCommitExpr} />,
    );
    typeAndEnter("=2");
    expect(onCommitExpr).toHaveBeenCalledWith(null, 50.8);
  });

  it("previews a length in millimetres AND the display unit, and says bare numbers are mm", async () => {
    setUnit("in");
    await mockClient.upsertVariable("w", 20);
    render(<DimensionInput value={25} suffix="mm" onCommit={vi.fn()} onCommitExpr={vi.fn()} />);

    fireEvent.change(input(), { target: { value: "=w/2" } });
    await waitFor(() =>
      expect(screen.getByTestId("dimension-expr-preview")).toHaveTextContent("= 10 mm (0.3937 in)"),
    );
    expect(screen.getByTestId("dimension-expr-hint")).toHaveTextContent(
      "bare numbers inside = are millimetres",
    );
  });

  it("shows no unit hint while millimetres are displayed — the two rules agree there", async () => {
    await mockClient.upsertVariable("w", 20);
    render(<DimensionInput value={25} suffix="mm" onCommit={vi.fn()} onCommitExpr={vi.fn()} />);
    fireEvent.change(input(), { target: { value: "=w/2" } });
    await waitFor(() =>
      expect(screen.getByTestId("dimension-expr-preview")).toHaveTextContent("= 10 mm"),
    );
    expect(screen.getByTestId("dimension-expr-preview")).not.toHaveTextContent("(");
    expect(screen.queryByTestId("dimension-expr-hint")).not.toBeInTheDocument();
  });

  /* An ANGLE field has no display unit to disagree with, so it never nudges. */
  it("shows no unit hint on an angle field", async () => {
    setUnit("in");
    render(<DimensionInput value={30} suffix="°" onCommit={vi.fn()} onCommitExpr={vi.fn()} />);
    fireEvent.change(input(), { target: { value: "=30deg*1.5" } });
    await waitFor(() =>
      expect(screen.getByTestId("dimension-expr-preview")).toHaveTextContent("= 45°"),
    );
    expect(screen.queryByTestId("dimension-expr-hint")).not.toBeInTheDocument();
  });
});
