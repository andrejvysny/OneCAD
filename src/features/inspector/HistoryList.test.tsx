import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { HistoryList, type HistoryRowActions, type HistoryValueEdit } from "./HistoryList";
import type { FeatureMeta } from "@/stores/documentStore";
import { ICON_PATHS } from "@/icons/paths";
import { settingsStore } from "@/stores/settingsStore";

const items: FeatureMeta[] = [
  { id: "f1", kind: "sketch", label: "Sketch 1", valueText: "", status: "ok" },
  { id: "f2", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "25.0 mm", primaryValue: 25, primaryValueKind: "length", status: "ok" },
  { id: "f3", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", primaryValue: 2, primaryValueKind: "length", status: "ok" },
];

describe("HistoryList", () => {
  it("renders one interactive chip per feature with its value", () => {
    render(<HistoryList items={items} onSelect={() => {}} />);
    expect(screen.getByTestId("history-row-f1")).toBeInTheDocument();
    // Rendered from `primaryValue` through the display formatter (trailing zeros
    // trimmed), NOT from the millimetre-fixed `valueText`.
    expect(screen.getByText("25 mm")).toBeInTheDocument();
    expect(screen.getByTestId("history-row-f2")).toHaveAttribute("role", "button");
  });

  it("calls onSelect with the feature id on click", () => {
    const onSelect = vi.fn();
    render(<HistoryList items={items} onSelect={onSelect} />);
    fireEvent.click(screen.getByTestId("history-row-f2"));
    expect(onSelect).toHaveBeenCalledWith("f2");
  });

  it("calls onEdit with the feature on double-click", () => {
    const onEdit = vi.fn();
    render(<HistoryList items={items} onEdit={onEdit} />);
    fireEvent.doubleClick(screen.getByTestId("history-row-f2"));
    expect(onEdit).toHaveBeenCalledWith(items[1]);
  });

  it("highlights the selected feature", () => {
    render(<HistoryList items={items} selectedId="f2" />);
    expect(screen.getByTestId("history-row-f2").className).toContain("bg-sel-bg");
    expect(screen.getByTestId("history-row-f1").className).not.toContain("bg-sel-bg");
  });

  it("is non-interactive (no button role) without handlers", () => {
    render(<HistoryList items={items} />);
    expect(screen.getByTestId("history-row-f1")).not.toHaveAttribute("role");
  });
});

describe("HistoryList row affordances (M4b)", () => {
  const actions = (over: Partial<HistoryRowActions> = {}): ((i: FeatureMeta) => HistoryRowActions) => {
    const base: HistoryRowActions = {
      suppressed: false,
      onToggleSuppress: vi.fn(),
      onRoll: vi.fn(),
      onDelete: vi.fn(),
      ...over,
    };
    return () => base;
  };

  it("shows suppress / roll / delete affordances only with rowActions", () => {
    const { rerender } = render(<HistoryList items={items} onSelect={() => {}} />);
    expect(screen.queryByTestId("history-suppress-f2")).toBeNull();
    rerender(<HistoryList items={items} onSelect={() => {}} rowActions={actions()} />);
    expect(screen.getByTestId("history-suppress-f2")).toBeInTheDocument();
    expect(screen.getByTestId("history-roll-f2")).toBeInTheDocument();
    expect(screen.getByTestId("history-delete-f2")).toBeInTheDocument();
  });

  it("calls onToggleSuppress / onRoll with the item", () => {
    const onToggleSuppress = vi.fn();
    const onRoll = vi.fn();
    render(<HistoryList items={items} rowActions={actions({ onToggleSuppress, onRoll })} />);
    fireEvent.click(screen.getByTestId("history-suppress-f2"));
    expect(onToggleSuppress).toHaveBeenCalledWith(items[1]);
    fireEvent.click(screen.getByTestId("history-roll-f2"));
    expect(onRoll).toHaveBeenCalledWith(items[1]);
  });

  it("delete requires a second confirm click (no browser confirm)", () => {
    const onDelete = vi.fn();
    render(<HistoryList items={items} rowActions={actions({ onDelete })} />);
    fireEvent.click(screen.getByTestId("history-delete-f2")); // arms confirm
    expect(onDelete).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("history-delete-confirm-f2")); // confirms
    expect(onDelete).toHaveBeenCalledWith(items[1]);
  });

  it("a suppressed row dims (line-through) and keeps its icon visible", () => {
    render(<HistoryList items={items} rowActions={actions({ suppressed: true })} />);
    expect(screen.getByTestId("history-row-f2").className).toContain("opacity-60");
    // The label carries a strike-through as the suppressed signal.
    expect(screen.getByText("Extrude").className).toContain("line-through");
  });

  it("affordance clicks do not also select the row", () => {
    const onSelect = vi.fn();
    render(<HistoryList items={items} onSelect={onSelect} rowActions={actions()} />);
    fireEvent.click(screen.getByTestId("history-roll-f2"));
    expect(onSelect).not.toHaveBeenCalled();
  });
});

describe("HistoryList inline value editing (H3)", () => {
  const rowActions = (): ((i: FeatureMeta) => HistoryRowActions) => () => ({
    suppressed: false,
    onToggleSuppress: vi.fn(),
    onRoll: vi.fn(),
    onDelete: vi.fn(),
  });
  const valueEdit =
    (editable: boolean, onCommit = vi.fn()) =>
    (): HistoryValueEdit => ({ editable, onCommit });

  afterEach(() => settingsStore.setState({ displayUnit: "mm" }));

  it("keeps the value VISIBLE when the affordance cluster is present", () => {
    // The regression this pins: `valueText && !actions` hid the one number the row
    // is about on exactly the view where a user goes to change it.
    render(<HistoryList items={items} rowActions={rowActions()} />);
    expect(screen.getByTestId("history-value-f2")).toHaveTextContent("25 mm");
    expect(screen.getByTestId("history-suppress-f2")).toBeInTheDocument();
  });

  it("renders the value in the DISPLAY unit, and an angle in degrees regardless", () => {
    const rows: FeatureMeta[] = [
      items[1],
      { id: "r1", kind: "revolve", opType: "Revolve", label: "Revolve", valueText: "90.0°", primaryValue: 90, primaryValueKind: "angle", status: "ok" },
      { id: "h1", kind: "boolean", opType: "Hole", label: "Hole", valueText: "Ø6.0", primaryValue: 6, primaryValueKind: "diameter", status: "ok" },
    ];
    settingsStore.setState({ displayUnit: "in" });
    render(<HistoryList items={rows} />);
    expect(screen.getByTestId("history-value-f2")).toHaveTextContent("0.9843 in");
    expect(screen.getByTestId("history-value-r1")).toHaveTextContent("90°");
    // A hole's row stays Ø-prefixed (dto.rs) so it cannot read as a length.
    expect(screen.getByTestId("history-value-h1")).toHaveTextContent("Ø0.2362");
  });

  it("falls back to the raw valueText for a row with no primary dimension", () => {
    const boolRow: FeatureMeta = {
      id: "b1", kind: "boolean", opType: "Boolean", label: "Cut", valueText: "Cut", status: "ok",
    };
    render(<HistoryList items={[boolRow]} rowActions={rowActions()} />);
    expect(screen.getByTestId("history-value-b1")).toHaveTextContent("Cut");
  });

  it("clicking the value opens an editor and commits the typed number", async () => {
    const onCommit = vi.fn();
    const onSelect = vi.fn();
    render(
      <HistoryList
        items={items}
        onSelect={onSelect}
        rowActions={rowActions()}
        valueEdit={valueEdit(true, onCommit)}
      />,
    );
    fireEvent.click(screen.getByTestId("history-value-f2"));
    // The click opens the editor AND still selects the row: swallowing it would
    // leave a dead zone in the middle of the row where clicking selects nothing.
    expect(onSelect).toHaveBeenCalledWith("f2");

    // The field appears one double-click threshold later — see VALUE_EDIT_OPEN_MS.
    const input = await screen.findByLabelText("Dimension value");
    fireEvent.change(input, { target: { value: "40" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onCommit).toHaveBeenCalledWith(items[1], 40);
  });

  it("Escape closes the editor without committing", async () => {
    const onCommit = vi.fn();
    render(<HistoryList items={items} valueEdit={valueEdit(true, onCommit)} />);
    fireEvent.click(screen.getByTestId("history-value-f2"));
    const input = await screen.findByLabelText("Dimension value");
    fireEvent.change(input, { target: { value: "40" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(onCommit).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("Dimension value")).toBeNull();
  });

  it("Enter on the focused chip opens the editor immediately (no dblclick ambiguity)", async () => {
    render(<HistoryList items={items} valueEdit={valueEdit(true)} />);
    fireEvent.keyDown(screen.getByTestId("history-value-f2"), { key: "Enter" });
    expect(await screen.findByLabelText("Dimension value")).toBeInTheDocument();
  });

  it("a NON-editable row (tool armed / suppressed / past the cursor) shows plain text", async () => {
    const onCommit = vi.fn();
    render(<HistoryList items={items} rowActions={rowActions()} valueEdit={valueEdit(false, onCommit)} />);
    expect(screen.getByTestId("history-value-f2")).toHaveTextContent("25 mm");
    fireEvent.click(screen.getByTestId("history-value-f2"));
    await new Promise((r) => setTimeout(r, 300));
    expect(screen.queryByLabelText("Dimension value")).toBeNull();
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("with no valueEdit builder at all the value is read-only", async () => {
    render(<HistoryList items={items} rowActions={rowActions()} />);
    fireEvent.click(screen.getByTestId("history-value-f2"));
    await new Promise((r) => setTimeout(r, 300));
    expect(screen.queryByLabelText("Dimension value")).toBeNull();
  });

  /*
   * The row's DOUBLE-click (full re-edit) and the value's single click (inline
   * edit) share a target, and the browser only fires `dblclick` when both clicks
   * do. Opening the field on the FIRST click swapped the chip for an input, so the
   * second click landed on a different element and no `dblclick` was ever
   * dispatched — the hole / step-import / filletChamfer re-edit specs all died on
   * that. The open is therefore DEFERRED, and the second click cancels it.
   */
  it("a DOUBLE-click over the value runs the row's full re-edit and opens NO field", async () => {
    const onEdit = vi.fn();
    render(<HistoryList items={items} onEdit={onEdit} rowActions={rowActions()} valueEdit={valueEdit(true)} />);
    const value = screen.getByTestId("history-value-f2");
    fireEvent.click(value, { detail: 1 });
    fireEvent.click(value, { detail: 2 }); // cancels the pending open
    fireEvent.doubleClick(screen.getByTestId("history-row-f2"));
    expect(onEdit).toHaveBeenCalledWith(items[1]);
    await new Promise((r) => setTimeout(r, 300));
    expect(screen.queryByLabelText("Dimension value")).toBeNull();
  });

  it("the pen affordance runs the full re-edit (the double-click twin)", () => {
    const onEdit = vi.fn();
    const onSelect = vi.fn();
    render(<HistoryList items={items} onSelect={onSelect} onEdit={onEdit} rowActions={rowActions()} />);
    fireEvent.click(screen.getByTestId("history-edit-f2"));
    expect(onEdit).toHaveBeenCalledWith(items[1]);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("hides the pen affordance on a list with no re-edit handler", () => {
    render(<HistoryList items={items} rowActions={rowActions()} />);
    expect(screen.queryByTestId("history-edit-f2")).toBeNull();
  });
});

/*
 * M18 PIN. `valueText` is the MILLIMETRE-FIXED document string and must keep being
 * produced whatever the display preference — three re-edit seeds parse it back as
 * mm (`filletRadius.radiusFromValueText`, `shellThickness.thicknessFromValueText`,
 * `patternPreview.countFromValueText`). H3 renders `primaryValue` through the
 * display formatter INSTEAD of it; that must not tempt anyone into dropping the
 * field, or a re-edit under `displayUnit = "in"` would seed 0.079 mm for a 2 mm
 * fillet. The producers are pinned in Rust (`dto.rs primary_value_…`); this pins
 * the two consumers that must keep agreeing on the mm domain.
 */
describe("M18 — valueText stays millimetre-fixed and parseable", () => {
  afterEach(() => settingsStore.setState({ displayUnit: "mm" }));

  it("the parsers read valueText as mm no matter what unit is on display", async () => {
    const { radiusFromValueText } = await import("@/tools/preview/filletRadius");
    const { thicknessFromValueText } = await import("@/tools/preview/shellThickness");
    const { countFromValueText } = await import("@/tools/preview/patternPreview");

    for (const unit of ["mm", "cm", "m", "in"] as const) {
      settingsStore.setState({ displayUnit: unit });
      // The Rust-composed strings, verbatim (`dto.rs feature_value`).
      expect(radiusFromValueText("2.0 mm")).toBe(2);
      expect(radiusFromValueText("1.0×2.5 mm")).toBe(1); // chamfer d1 leads
      expect(thicknessFromValueText("1.5 mm")).toBe(1.5);
      expect(countFromValueText("×4")).toBe(4);
    }
  });

  it("a projection row carries BOTH the mm text and the primary number", () => {
    // The row displays the number; the parsers keep reading the text. Neither
    // field may be dropped in favour of the other.
    settingsStore.setState({ displayUnit: "in" });
    render(<HistoryList items={items} />);
    expect(screen.getByTestId("history-value-f3")).toHaveTextContent("0.0787 in");
    expect(items[2].valueText).toBe("2.0 mm");
    expect(items[2].primaryValue).toBe(2);
  });
});

describe("HistoryList icons — opType-first lookup (W0.5)", () => {
  // FeatureKind folds Chamfer/Shell into "fillet" and pattern/mirror ops into
  // "boolean" (dto.rs feature_kind) so the icon must key off the exact authored
  // `opType` first, falling back to the coarse `kind` bucket only when absent.
  function iconPathOf(testId: string): string | null {
    return screen.getByTestId(testId).querySelector("path")?.getAttribute("d") ?? null;
  }

  it("a folded Chamfer row (kind: fillet) shows the chamfer icon, not the fillet icon", () => {
    const chamferRow: FeatureMeta = {
      id: "c1",
      kind: "fillet",
      opType: "Chamfer",
      label: "Chamfer",
      valueText: "2.0 mm",
      status: "ok",
    };
    render(<HistoryList items={[chamferRow]} />);
    expect(iconPathOf("history-row-c1")).toBe(ICON_PATHS.chamfer);
    expect(iconPathOf("history-row-c1")).not.toBe(ICON_PATHS.fillet);
  });

  it("a folded MirrorBody row (kind: boolean) shows the mirror icon", () => {
    const mirrorRow: FeatureMeta = {
      id: "m1",
      kind: "boolean",
      opType: "MirrorBody",
      label: "Mirror",
      valueText: "",
      status: "ok",
    };
    render(<HistoryList items={[mirrorRow]} />);
    expect(iconPathOf("history-row-m1")).toBe(ICON_PATHS.mirrorBody);
  });

  it("a folded TransformBody row (kind: boolean) shows the move icon", () => {
    const moveRow: FeatureMeta = {
      id: "t1",
      kind: "boolean",
      opType: "TransformBody",
      label: "Move",
      valueText: "Δ(30.0, 0.0, 0.0)",
      status: "ok",
    };
    render(<HistoryList items={[moveRow]} />);
    expect(iconPathOf("history-row-t1")).toBe(ICON_PATHS.move);
    expect(iconPathOf("history-row-t1")).not.toBe(ICON_PATHS.boolean);
  });

  it("a row without opType falls back to the kind icon", () => {
    const noOpTypeRow: FeatureMeta = {
      id: "n1",
      kind: "fillet",
      label: "Fillet",
      valueText: "2.0 mm",
      status: "ok",
    };
    render(<HistoryList items={[noOpTypeRow]} />);
    expect(iconPathOf("history-row-n1")).toBe(ICON_PATHS.fillet);
  });
});
