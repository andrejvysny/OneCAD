import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { HistoryList, type HistoryRowActions, type HistoryValueEdit } from "./HistoryList";
import type { FeatureMeta } from "@/stores/documentStore";
import { ICONS, type IconName } from "@/icons/paths";
import { settingsStore } from "@/stores/settingsStore";
import { documentStore } from "@/stores/documentStore";
import { mockClient } from "@/ipc/mockClient";
import { planeFor } from "@/ipc/mockSketch";
import { resetStores } from "@/test/resetStores";

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
  // Icons are multi-element (two-tone), so identity is the full ordered list of
  // `d` strings, not a single path.
  function iconPathOf(testId: string): (string | null)[] {
    return [...screen.getByTestId(testId).querySelectorAll("path")].map((p) =>
      p.getAttribute("d"),
    );
  }

  function iconDef(name: IconName): string[] {
    return ICONS[name].map((el) => el.d);
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
    expect(iconPathOf("history-row-c1")).toEqual(iconDef("chamfer"));
    expect(iconPathOf("history-row-c1")).not.toEqual(iconDef("fillet"));
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
    expect(iconPathOf("history-row-m1")).toEqual(iconDef("mirrorBody"));
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
    expect(iconPathOf("history-row-t1")).toEqual(iconDef("move"));
    expect(iconPathOf("history-row-t1")).not.toEqual(iconDef("boolean"));
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
    expect(iconPathOf("history-row-n1")).toEqual(iconDef("fillet"));
  });
});

/*
 * H7b — the ROLLBACK CURSOR is visible: rows past the bar read as drafts, the bar
 * itself is drawn, and a persistent banner offers the way back. `appliedOps` is
 * POSITIONAL over `items`, so only the full-lineage view may pass it.
 */
describe("HistoryList rollback cursor (H7b)", () => {
  const rowActions = (): ((i: FeatureMeta) => HistoryRowActions) => () => ({
    suppressed: false,
    onToggleSuppress: vi.fn(),
    onRoll: vi.fn(),
    onDelete: vi.fn(),
  });

  it("grays + italicizes the rows at or past the cursor and titles them", () => {
    render(<HistoryList items={items} appliedOps={1} onRollToEnd={vi.fn()} />);
    expect(screen.getByTestId("history-row-f1")).toHaveAttribute("data-applied", "true");
    expect(screen.getByTestId("history-row-f2")).toHaveAttribute("data-applied", "false");
    expect(screen.getByTestId("history-row-f2").className).toContain("italic");
    expect(screen.getByTestId("history-row-f2")).toHaveAttribute(
      "title",
      "Not applied — beyond the rollback bar",
    );
  });

  it("draws the marker between the last applied row and the first draft", () => {
    render(<HistoryList items={items} appliedOps={2} onRollToEnd={vi.fn()} />);
    const marker = screen.getByTestId("history-rollback-marker");
    expect(marker).toHaveTextContent("rolled back");
    // It sits ABOVE the first unapplied row, i.e. immediately before f3.
    const rows = screen.getByTestId("history-row-f3");
    expect(marker.compareDocumentPosition(rows) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("banners the rolled-back count and rolls to end on click", () => {
    const onRollToEnd = vi.fn();
    render(<HistoryList items={items} appliedOps={1} onRollToEnd={onRollToEnd} />);
    expect(screen.getByTestId("history-rollback-banner")).toHaveTextContent(
      "2 operations rolled back",
    );
    fireEvent.click(screen.getByTestId("history-roll-to-end"));
    expect(onRollToEnd).toHaveBeenCalled();
  });

  it("says 'operation' (singular) for a single rolled-back op", () => {
    render(<HistoryList items={items} appliedOps={2} onRollToEnd={vi.fn()} />);
    expect(screen.getByTestId("history-rollback-banner")).toHaveTextContent(
      "1 operation rolled back",
    );
  });

  it("renders NO cursor chrome for a fully-applied timeline or a slice view", () => {
    const { rerender } = render(
      <HistoryList items={items} appliedOps={items.length} onRollToEnd={vi.fn()} />,
    );
    expect(screen.queryByTestId("history-rollback-banner")).toBeNull();
    expect(screen.queryByTestId("history-rollback-marker")).toBeNull();
    expect(screen.getByTestId("history-row-f3")).toHaveAttribute("data-applied", "true");

    // A SLICE view passes no cursor at all — indices there are not global, so a
    // bar drawn from them would point at the wrong op.
    rerender(<HistoryList items={items} rowActions={rowActions()} />);
    expect(screen.queryByTestId("history-rollback-banner")).toBeNull();
    expect(screen.getByTestId("history-row-f3")).toHaveAttribute("data-applied", "true");
  });
});

/*
 * H7b — failure visibility. The status→tone map is derived from the HALT POINT
 * (the first error/needsRepair row) so the row a user is sent to is the row that
 * is tinted, and a dirty row is only "blocked" when something ahead of it stopped.
 */
describe("HistoryList failure tones (H7b)", () => {
  const tinted = (over: Partial<FeatureMeta>[]): FeatureMeta[] =>
    items.map((f, i) => ({ ...f, ...(over[i] ?? {}) }));

  it("an errored row keeps the red tint + the worker reason as its tooltip", () => {
    render(<HistoryList items={tinted([{}, { status: "error", statusMessage: "boom" }, {}])} />);
    expect(screen.getByTestId("history-row-f2")).toHaveAttribute("data-tone", "error");
    expect(screen.getByTestId("history-row-f2")).toHaveAttribute("title", "boom");
    expect(screen.getByText("Extrude").className).toContain("text-traffic-close");
  });

  it("a needsRepair row is warn-toned and carries the repair glyph", () => {
    render(<HistoryList items={tinted([{}, { status: "needsRepair" }, {}])} />);
    expect(screen.getByTestId("history-row-f2")).toHaveAttribute("data-tone", "repair");
    expect(screen.getByTestId("history-repair-glyph-f2")).toBeInTheDocument();
    expect(screen.getByText("Extrude").className).toContain("text-warn");
  });

  it("an ok row with a warning diagnostic gets the diagnostic badge, titled with the message", () => {
    render(
      <HistoryList
        items={tinted([
          {},
          { diagnostics: [{ severity: "warning", code: "REGION_REBOUND_BY_ANCHOR", message: "re-bound via anchor" }] },
          {},
        ])}
      />,
    );
    expect(screen.getByTestId("feature-diagnostic-badge-f2")).toHaveAttribute("data-severity", "warning");
    expect(screen.getByTestId("feature-diagnostic-badge-f2")).toHaveAttribute("title", "re-bound via anchor");
    expect(screen.queryByTestId("feature-diagnostic-badge-f1")).toBeNull();
  });

  it("has no diagnostic badge for a plain ok row", () => {
    render(<HistoryList items={items} />);
    expect(screen.queryByTestId("feature-diagnostic-badge-f2")).toBeNull();
  });

  it("suppresses the diagnostic badge on an errored row (already red end to end)", () => {
    render(
      <HistoryList
        items={tinted([
          {},
          {
            status: "error",
            statusMessage: "boom",
            diagnostics: [{ severity: "error", code: "KERNEL_REFUSED", message: "boom" }],
          },
          {},
        ])}
      />,
    );
    expect(screen.getByTestId("history-row-f2")).toHaveAttribute("data-tone", "error");
    expect(screen.queryByTestId("feature-diagnostic-badge-f2")).toBeNull();
  });

  it("grays only the dirty rows AFTER the halt — a pre-halt dirty row is normal", () => {
    // f1 dirty (ordinary mid-regen), f2 errored (the halt), f3 dirty (blocked).
    render(
      <HistoryList items={tinted([{ status: "dirty" }, { status: "error" }, { status: "dirty" }])} />,
    );
    expect(screen.getByTestId("history-row-f1")).toHaveAttribute("data-tone", "normal");
    expect(screen.getByTestId("history-row-f3")).toHaveAttribute("data-tone", "stale");
    expect(screen.getByTestId("history-row-f3")).toHaveAttribute(
      "title",
      "Not rebuilt — the timeline stopped earlier",
    );
  });

  it("with nothing halted, a dirty row stays normal", () => {
    render(<HistoryList items={tinted([{}, { status: "dirty" }, {}])} />);
    expect(screen.getByTestId("history-row-f2")).toHaveAttribute("data-tone", "normal");
  });

  it("an errored row pins its suppress affordance visible, titled for recovery", () => {
    const onToggleSuppress = vi.fn();
    render(
      <HistoryList
        items={tinted([{}, { status: "error" }, {}])}
        rowActions={() => ({
          suppressed: false,
          onToggleSuppress,
          onRoll: vi.fn(),
          onDelete: vi.fn(),
        })}
      />,
    );
    const suppress = screen.getByTestId("history-suppress-f2");
    expect(suppress).toHaveAttribute("title", "Suppress to continue rebuild");
    // The cluster is not hover-gated on this row (opacity-0 would hide it).
    expect(suppress.parentElement?.className).toContain("opacity-100");
    fireEvent.click(suppress);
    expect(onToggleSuppress).toHaveBeenCalled();
  });

  it("the SELECTED row also keeps its cluster visible", () => {
    render(
      <HistoryList
        items={items}
        selectedId="f2"
        rowActions={() => ({
          suppressed: false,
          onToggleSuppress: vi.fn(),
          onRoll: vi.fn(),
          onDelete: vi.fn(),
        })}
      />,
    );
    expect(screen.getByTestId("history-suppress-f2").parentElement?.className).toContain(
      "opacity-100",
    );
    expect(screen.getByTestId("history-suppress-f1").parentElement?.className).toContain(
      "opacity-0",
    );
  });
});

/*
 * H7b — the row CONTEXT MENU (same Popover/MenuItem shape as the model tree's).
 * Right-click selects the row first so the menu can never act on a row other than
 * the one under the pointer.
 */
describe("HistoryList context menu (H7b)", () => {
  const acts = (over: Partial<HistoryRowActions> = {}) => {
    const base: HistoryRowActions = {
      suppressed: false,
      onToggleSuppress: vi.fn(),
      onRoll: vi.fn(),
      onDelete: vi.fn(),
      ...over,
    };
    return { base, build: () => base };
  };

  it("opens on right-click, selects the row, and offers the full set", () => {
    const onSelect = vi.fn();
    const { base, build } = acts();
    render(
      <HistoryList items={items} onSelect={onSelect} onEdit={vi.fn()} rowActions={build} />,
    );
    fireEvent.contextMenu(screen.getByTestId("history-row-f2"));
    expect(onSelect).toHaveBeenCalledWith("f2");
    expect(screen.getByTestId("history-menu-edit")).toBeInTheDocument();
    expect(screen.getByTestId("history-menu-roll-here")).toBeInTheDocument();
    expect(screen.getByTestId("history-menu-suppress")).toHaveTextContent("Suppress");
    expect(screen.getByTestId("history-menu-delete")).toBeInTheDocument();
    // No "Roll to end" while the timeline is fully applied — a dead affordance.
    expect(screen.queryByTestId("history-menu-roll-end")).toBeNull();
    fireEvent.click(screen.getByTestId("history-menu-roll-here"));
    expect(base.onRoll).toHaveBeenCalledWith(items[1]);
  });

  it("offers Roll to end only while rolled back", () => {
    const onRollToEnd = vi.fn();
    render(
      <HistoryList
        items={items}
        rowActions={acts().build}
        appliedOps={1}
        onRollToEnd={onRollToEnd}
      />,
    );
    fireEvent.contextMenu(screen.getByTestId("history-row-f1"));
    fireEvent.click(screen.getByTestId("history-menu-roll-end"));
    expect(onRollToEnd).toHaveBeenCalled();
  });

  it("labels the suppress item by the row's own flag", () => {
    const { base, build } = acts({ suppressed: true });
    render(<HistoryList items={items} rowActions={build} />);
    fireEvent.contextMenu(screen.getByTestId("history-row-f2"));
    expect(screen.getByTestId("history-menu-suppress")).toHaveTextContent("Unsuppress");
    fireEvent.click(screen.getByTestId("history-menu-suppress"));
    expect(base.onToggleSuppress).toHaveBeenCalledWith(items[1]);
  });

  it("delete is a two-click confirm (the house idiom)", () => {
    const { base, build } = acts();
    render(<HistoryList items={items} rowActions={build} />);
    fireEvent.contextMenu(screen.getByTestId("history-row-f2"));
    fireEvent.click(screen.getByTestId("history-menu-delete"));
    expect(base.onDelete).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("history-menu-delete-confirm"));
    expect(base.onDelete).toHaveBeenCalledWith(items[1]);
  });

  it("does not open at all on a list with no rowActions (read-only slice view)", () => {
    render(<HistoryList items={items} onSelect={vi.fn()} />);
    fireEvent.contextMenu(screen.getByTestId("history-row-f2"));
    expect(screen.queryByTestId("history-menu-roll-here")).toBeNull();
  });
});

/*
 * H10 — dependency-view dependent counts behind the suppress/delete affordances,
 * both on the row itself (× two-click confirm + the suppress button's title) and
 * on the context menu's Suppress/Delete items. `getDependents` is fetched ONCE
 * per open/hover (never per render) — pinned by the call-count assertions below.
 */
describe("HistoryList H10 dependent counts", () => {
  const depsOf = (downstream: string[]) =>
    vi.fn().mockResolvedValue({ upstream: [], downstream });
  const acts = (getDependents: HistoryRowActions["getDependents"]): HistoryRowActions => ({
    suppressed: false,
    onToggleSuppress: vi.fn(),
    onRoll: vi.fn(),
    onDelete: vi.fn(),
    getDependents,
  });

  it("does NOT fetch dependents on plain render — only on hover/open", () => {
    const getDependents = depsOf(["f5"]);
    render(<HistoryList items={items} rowActions={() => acts(getDependents)} />);
    expect(getDependents).not.toHaveBeenCalled();
  });

  it("hovering the row's affordance cluster fetches once and titles the Confirm-delete button with the count", async () => {
    const getDependents = depsOf(["f5", "f6"]);
    render(<HistoryList items={items} rowActions={() => acts(getDependents)} />);

    const cluster = screen.getByTestId("history-suppress-f2").parentElement!;
    fireEvent.mouseEnter(cluster);
    fireEvent.mouseEnter(cluster); // a second hover must NOT re-fetch
    expect(getDependents).toHaveBeenCalledTimes(1);
    expect(getDependents).toHaveBeenCalledWith(items[1]);

    // The suppress title picks up the count once the fetch resolves.
    await screen.findByTitle("Suppress — 2 dependents");

    fireEvent.click(screen.getByTestId("history-delete-f2")); // arms confirm
    expect(getDependents).toHaveBeenCalledTimes(1); // still cached, no re-fetch
    await screen.findByTitle("Confirm delete — 2 dependents");
  });

  it("a × click with no prior hover still fetches (fallback) and titles the confirm once resolved", async () => {
    const getDependents = depsOf(["f5"]);
    render(<HistoryList items={items} rowActions={() => acts(getDependents)} />);
    fireEvent.click(screen.getByTestId("history-delete-f2"));
    expect(getDependents).toHaveBeenCalledTimes(1);
    await screen.findByTitle("Confirm delete — 1 dependent");
  });

  it("a ZERO dependent count leaves the plain titles untouched", async () => {
    const getDependents = depsOf([]);
    render(<HistoryList items={items} rowActions={() => acts(getDependents)} />);
    fireEvent.mouseEnter(screen.getByTestId("history-suppress-f2").parentElement!);
    await vi.waitFor(() => expect(getDependents).toHaveBeenCalled());
    expect(screen.getByTestId("history-suppress-f2")).toHaveAttribute("title", "Suppress");
  });

  it("the context menu's Suppress/Delete items pick up the count on open, fetched once", async () => {
    const getDependents = depsOf(["f5", "f6", "f7"]);
    render(<HistoryList items={items} rowActions={() => acts(getDependents)} />);

    fireEvent.contextMenu(screen.getByTestId("history-row-f2"));
    expect(getDependents).toHaveBeenCalledTimes(1);
    expect(getDependents).toHaveBeenCalledWith(items[1]);
    expect(await screen.findByTestId("history-menu-suppress")).toHaveTextContent(
      "Suppress — 3 dependents",
    );
    expect(screen.getByTestId("history-menu-delete")).toHaveTextContent("Delete — 3 dependents");

    fireEvent.click(screen.getByTestId("history-menu-delete"));
    expect(screen.getByTestId("history-menu-delete-confirm")).toHaveTextContent(
      "Confirm delete — 3 dependents",
    );
    // Still just the one fetch from opening the menu.
    expect(getDependents).toHaveBeenCalledTimes(1);
  });

  it("re-opening the menu on a DIFFERENT row re-fetches for that row", async () => {
    const getDependents = vi.fn(async (item: FeatureMeta) =>
      item.id === "f2" ? { upstream: [], downstream: ["f5"] } : { upstream: [], downstream: [] },
    );
    render(<HistoryList items={items} rowActions={() => acts(getDependents)} />);

    fireEvent.contextMenu(screen.getByTestId("history-row-f2"));
    expect(await screen.findByTestId("history-menu-suppress")).toHaveTextContent(
      "Suppress — 1 dependent",
    );

    fireEvent.contextMenu(screen.getByTestId("history-row-f1"));
    expect(getDependents).toHaveBeenCalledTimes(2);
    await vi.waitFor(() =>
      expect(screen.getByTestId("history-menu-suppress")).toHaveTextContent("Suppress"),
    );
    expect(screen.getByTestId("history-menu-suppress")).not.toHaveTextContent("dependent");
  });

  it("omitting getDependents skips the hint entirely (no count, no crash)", () => {
    render(
      <HistoryList
        items={items}
        rowActions={() => ({
          suppressed: false,
          onToggleSuppress: vi.fn(),
          onRoll: vi.fn(),
          onDelete: vi.fn(),
        })}
      />,
    );
    fireEvent.mouseEnter(screen.getByTestId("history-suppress-f2").parentElement!);
    expect(screen.getByTestId("history-suppress-f2")).toHaveAttribute("title", "Suppress");
  });
});

// ── HISTORY-HARDEN H9: Reattach… ─────────────────────────────────────────────

describe("HistoryList — the Reattach affordance (H9)", () => {
  const acts = (): HistoryRowActions => ({
    suppressed: false,
    onToggleSuppress: vi.fn(),
    onRoll: vi.fn(),
    onDelete: vi.fn(),
  });

  afterEach(() => {
    vi.restoreAllMocks();
    resetStores();
  });

  it("is offered on a SKETCH row only", () => {
    render(<HistoryList items={items} rowActions={acts} />);
    fireEvent.contextMenu(screen.getByTestId("history-row-f1")); // Sketch 1
    expect(screen.getByTestId("history-menu-reattach")).toBeInTheDocument();
  });

  it("is NOT offered on a non-sketch row (an extrude has no attachment to move)", () => {
    render(<HistoryList items={items} rowActions={acts} />);
    fireEvent.contextMenu(screen.getByTestId("history-row-f2")); // Extrude
    expect(screen.queryByTestId("history-menu-reattach")).toBeNull();
  });

  it("resolves the row's SKETCH id (a feature id is a RECORD id) then opens the picker", async () => {
    // The projection carries no feature→sketch link, so the stored op params are
    // the only place it exists.
    const params = vi
      .spyOn(mockClient, "getOperationParams")
      .mockResolvedValue({ sketchId: "sketch2" });
    render(<HistoryList items={items} rowActions={acts} />);
    fireEvent.contextMenu(screen.getByTestId("history-row-f1"));
    fireEvent.click(screen.getByTestId("history-menu-reattach"));
    expect(params).toHaveBeenCalledWith("f1");

    // World planes are always offered; the seeded document has no datums.
    expect(await screen.findByTestId("reattach-world-XY")).toBeInTheDocument();
    expect(screen.getByTestId("reattach-world-XZ")).toBeInTheDocument();
    expect(screen.getByTestId("reattach-world-YZ")).toBeInTheDocument();

    const reattach = vi.spyOn(mockClient, "reattachSketch");
    fireEvent.click(screen.getByTestId("reattach-world-YZ"));
    await waitFor(() =>
      expect(reattach).toHaveBeenCalledWith("sketch2", { kind: "world", plane: "YZ" }),
    );
  });

  it("lists RESOLVED datums as targets and shows an unresolved one as inert", async () => {
    documentStore.getState().addDatum({
      id: "d1",
      name: "Datum 1",
      basePlaneId: "XY",
      offset: 10,
      plane: { ...planeFor("XY"), kind: "custom" },
      resolvedValid: true,
    });
    documentStore.getState().addDatum({
      id: "d2",
      name: "Broken",
      basePlaneId: "nope",
      offset: 1,
      plane: { ...planeFor("XY"), kind: "custom" },
      resolvedValid: false,
    });
    vi.spyOn(mockClient, "getOperationParams").mockResolvedValue({ sketchId: "sketch2" });
    const reattach = vi.spyOn(mockClient, "reattachSketch");
    render(<HistoryList items={items} rowActions={acts} />);
    fireEvent.contextMenu(screen.getByTestId("history-row-f1"));
    fireEvent.click(screen.getByTestId("history-menu-reattach"));

    expect(await screen.findByTestId("reattach-datum-d1")).toHaveTextContent("Datum 1");
    // Shown, not hidden — it exists in the tree — but it cannot be picked.
    expect(screen.getByTestId("reattach-datum-d2")).toHaveTextContent("(unresolved)");
    fireEvent.click(screen.getByTestId("reattach-datum-d2"));
    expect(reattach).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId("reattach-datum-d1"));
    await waitFor(() =>
      expect(reattach).toHaveBeenCalledWith("sketch2", { kind: "datum", datumId: "d1" }),
    );
  });
});

// ── WP-VE.2: a bound row reads as its binding ────────────────────────────────

describe("HistoryList — variable bindings", () => {
  const bound: FeatureMeta[] = [
    {
      id: "f2",
      kind: "extrude",
      opType: "Extrude",
      label: "Extrude",
      valueText: "25.0 mm",
      primaryValue: 25,
      primaryValueKind: "length",
      primaryExpr: "height",
      status: "ok",
    },
  ];

  /*
   * The number is DERIVED once a variable drives it, so showing it at rest would
   * invite an edit the next regen overwrites. It moves to the tooltip instead —
   * and `primaryExpr` is backend-minted, so this can never claim a binding the
   * document does not hold.
   */
  it("shows =name in place of the resolved number, with the number in the tooltip", () => {
    render(<HistoryList items={bound} />);
    const chip = screen.getByTestId("history-value-f2");
    expect(chip).toHaveTextContent("=height");
    expect(chip).toHaveAttribute("title", "height = 25 mm");
  });

  it("an UNBOUND row keeps the plain number and the plain tooltip", () => {
    render(
      <HistoryList
        items={items}
        valueEdit={() => ({ editable: true, bindable: true, onCommit: vi.fn(), onCommitExpr: vi.fn() })}
      />,
    );
    const chip = screen.getByTestId("history-value-f2");
    expect(chip).toHaveTextContent("25 mm");
    expect(chip).toHaveAttribute("title", "Edit value");
  });

  it("opens the editor seeded with the binding and commits a new one", async () => {
    // The new binding has to RESOLVE: the committed number is the evaluator's,
    // not the `primaryValue` the row was seeded with.
    await mockClient.upsertVariable("width", 40);
    const onCommitExpr = vi.fn();
    render(
      <HistoryList
        items={bound}
        valueEdit={() => ({ editable: true, bindable: true, onCommit: vi.fn(), onCommitExpr })}
      />,
    );
    fireEvent.click(screen.getByTestId("history-value-f2"));
    const field = (await screen.findByLabelText("Dimension value")) as HTMLInputElement;
    expect(field.value).toBe("=height");

    fireEvent.change(field, { target: { value: "=width" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(onCommitExpr).toHaveBeenCalledWith(bound[0], "width", 40));
    await mockClient.removeVariable("width");
  });

  /** `bindable: false` (a Hole) must not even offer the affordance. */
  it("a non-bindable row's editor refuses =name outright", async () => {
    const onCommit = vi.fn();
    const onCommitExpr = vi.fn();
    render(
      <HistoryList
        items={items}
        valueEdit={() => ({ editable: true, bindable: false, onCommit, onCommitExpr })}
      />,
    );
    fireEvent.click(screen.getByTestId("history-value-f2"));
    const field = await screen.findByLabelText("Dimension value");
    fireEvent.change(field, { target: { value: "=height" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(onCommitExpr).not.toHaveBeenCalled();
    expect(onCommit).not.toHaveBeenCalled();
  });
});
