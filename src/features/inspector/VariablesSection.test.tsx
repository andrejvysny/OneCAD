/*
 * WP-VE.2 — the Variables inspector section, against the REAL mock lane.
 *
 * Deliberately not a `vi.mock` of the client: the mock client keeps a real
 * in-memory table with the same validation the backend applies, so exercising it
 * proves the section against the rules it will actually meet. A stubbed client
 * would only prove the section agrees with the stub.
 */
import { beforeEach, describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { VariablesSection } from "./VariablesSection";
import { mockClient, resetMockDocument, setMockLatency } from "@/ipc/mockClient";
import { viewportStore } from "@/stores/viewportStore";

describe("VariablesSection (WP-VE.2)", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  /** Adds a variable through the draft row exactly as a user would. */
  async function addVariable(name: string, value: string) {
    fireEvent.change(screen.getByTestId("variable-new-name"), { target: { value: name } });
    fireEvent.change(screen.getByTestId("variable-new-value"), { target: { value } });
    fireEvent.keyDown(screen.getByTestId("variable-new-value"), { key: "Enter" });
  }

  it("starts empty and says so", async () => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByText("Variables")).toBeInTheDocument());
    expect(screen.getByTestId("variables-empty")).toBeInTheDocument();
  });

  it("adds a variable on Enter and clears the draft row", async () => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());

    await addVariable("height", "25");

    await waitFor(() => expect(screen.getByTestId("variable-row-height")).toBeInTheDocument());
    expect(screen.getByTestId("variable-value-height")).toHaveValue("25");
    expect(screen.queryByTestId("variables-empty")).not.toBeInTheDocument();
    // The draft row resets, so a second add does not silently re-submit the first.
    expect(screen.getByTestId("variable-new-name")).toHaveValue("");
    expect(await mockClient.listVariables()).toEqual([
      { id: expect.any(String), name: "height", value: 25, resolvedValue: 25, dimension: "scalar" },
    ]);
  });

  it("edits an existing value on blur", async () => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());
    await addVariable("width", "10");
    await waitFor(() => expect(screen.getByTestId("variable-row-width")).toBeInTheDocument());

    const field = screen.getByTestId("variable-value-width");
    fireEvent.change(field, { target: { value: "42" } });
    fireEvent.blur(field);

    await waitFor(async () =>
      expect(await mockClient.listVariables()).toEqual([
        { id: expect.any(String), name: "width", value: 42, resolvedValue: 42, dimension: "scalar" },
      ]),
    );
  });

  it("deletes a variable", async () => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());
    await addVariable("gone", "1");
    await waitFor(() => expect(screen.getByTestId("variable-row-gone")).toBeInTheDocument());

    fireEvent.click(screen.getByTestId("variable-delete-gone"));

    await waitFor(() => expect(screen.queryByTestId("variable-row-gone")).not.toBeInTheDocument());
    expect(await mockClient.listVariables()).toEqual([]);
  });

  /*
   * A malformed name is caught BEFORE the round trip so the message lands next
   * to the field — but the grammar it checks is the backend's own
   * (`VARIABLE_NAME_RE` === `regen::variables::is_bare_name`), so the two cannot
   * drift into a UI that accepts what the document refuses.
   */
  it.each([
    ["2wide", "a leading digit"],
    ["my-var", "punctuation"],
    ["w * 2", "an arithmetic expression"],
    ["", "an empty name"],
  ])("refuses %s (%s) with an inline message", async (name) => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());

    await addVariable(name, "5");

    await waitFor(() => expect(screen.getByTestId("variables-error")).toBeInTheDocument());
    expect(screen.getByTestId("variables-error").textContent).toContain("Invalid variable name");
    expect(await mockClient.listVariables()).toEqual([]);
  });

  it("refuses a name with no number", async () => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());

    fireEvent.change(screen.getByTestId("variable-new-name"), { target: { value: "height" } });
    fireEvent.keyDown(screen.getByTestId("variable-new-name"), { key: "Enter" });

    await waitFor(() =>
      expect(screen.getByTestId("variables-error")).toHaveTextContent("needs a number"),
    );
    expect(await mockClient.listVariables()).toEqual([]);
  });

  /** A duplicate is a RE-VALUE, not a second row — matching `VariableTable::upsert`. */
  it("re-values rather than duplicating an existing name", async () => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());
    await addVariable("depth", "3");
    await waitFor(() => expect(screen.getByTestId("variable-row-depth")).toBeInTheDocument());

    await addVariable("depth", "9");

    await waitFor(async () =>
      expect(await mockClient.listVariables()).toEqual([
        { id: expect.any(String), name: "depth", value: 9, resolvedValue: 9, dimension: "scalar" },
      ]),
    );
    expect(screen.getAllByTestId("variable-row-depth")).toHaveLength(1);
  });

  /** Case-SENSITIVE, because that is what an `expr` lookup sees. */
  it("treats Width and width as two variables", async () => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());
    await addVariable("width", "1");
    await waitFor(() => expect(screen.getByTestId("variable-row-width")).toBeInTheDocument());
    await addVariable("Width", "2");
    await waitFor(() => expect(screen.getByTestId("variable-row-Width")).toBeInTheDocument());
    expect(await mockClient.listVariables()).toHaveLength(2);
  });

  it("re-reads the table when the document changes underneath it (undo)", async () => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());
    await addVariable("held", "7");
    await waitFor(() => expect(screen.getByTestId("variable-row-held")).toBeInTheDocument());

    // A change this component did NOT make — exactly what an undo looks like from
    // here. The section must follow the document, not its own last write.
    await mockClient.removeVariable("held");

    await waitFor(() => expect(screen.queryByTestId("variable-row-held")).not.toBeInTheDocument());
  });
});

/*
 * W5 — "saved + loud failure banner".
 *
 * A variable edit has TWO truths when its downstream regen fails, and the section
 * used to state only the first. The list keeps the value the document really holds
 * (nothing is ever reverted), and the failure rides the SAME sticky status-bar hint
 * `featureValueEdit` / `treeActions` / `ComponentParametersSection` already use —
 * not a new banner, and not the section's own `variables-error` alert, which means
 * "the write was REFUSED" and must stay distinguishable.
 */
describe("VariablesSection — result truth (W5)", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
    viewportStore.getState().setStatusHint(null);
  });

  /** Bind the seeded f2 extrude to `=height`, as the `=name` gesture does. */
  async function bindF2ToHeight() {
    await mockClient.applyEditCommand({
      cmd: "updateOperationParams",
      record: "f2",
      op: {
        opType: "Extrude",
        params: {
          profile: { sketchId: "sketch1", regionId: "r0" },
          draftAngleDeg: { value: 0 },
          distance: { value: 30, expr: "height" },
        },
      },
    } as unknown as Parameters<typeof mockClient.applyEditCommand>[0]);
  }

  it("keeps the SAVED value on screen and raises the failure hint when the rebuild fails", async () => {
    await mockClient.upsertVariable("height", 25);
    await bindF2ToHeight();

    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-value-height")).toBeInTheDocument());

    // 0 drives the bound extrude below the kernel's distance floor.
    const field = screen.getByTestId("variable-value-height");
    fireEvent.change(field, { target: { value: "0" } });
    fireEvent.blur(field);

    // Truth 1 — the save is REAL: the document holds it and the row shows it.
    await waitFor(async () =>
      expect(await mockClient.listVariables()).toEqual([
        { id: expect.any(String), name: "height", value: 0, resolvedValue: 0, dimension: "scalar" },
      ]),
    );
    await waitFor(() => expect(screen.getByTestId("variable-value-height")).toHaveValue("0"));
    // Truth 2 — the failure is LOUD, on the existing status-bar affordance…
    await waitFor(() => {
      const hint = viewportStore.getState().statusHint;
      expect(hint?.severity).toBe("error");
      expect(hint?.sticky).toBe(true);
      expect(hint?.message).toMatch(/Variable saved, but the rebuild failed: .*distance too small/);
    });
    // …and NOT as a refusal: nothing was rejected, so the inline error stays absent.
    expect(screen.queryByTestId("variables-error")).not.toBeInTheDocument();
  });

  it("deleting a variable a record still binds removes the row AND says why the rebuild failed", async () => {
    await mockClient.upsertVariable("height", 25);
    await bindF2ToHeight();

    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-row-height")).toBeInTheDocument());

    fireEvent.click(screen.getByTestId("variable-delete-height"));

    await waitFor(() => expect(screen.queryByTestId("variable-row-height")).not.toBeInTheDocument());
    expect(await mockClient.listVariables()).toEqual([]);
    await waitFor(() =>
      expect(viewportStore.getState().statusHint?.message).toMatch(
        /Variable saved, but the rebuild failed: .*undefined variable `height`/,
      ),
    );
  });

  it("raises NO hint for a healthy edit", async () => {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());
    await addVariableRow("clean", "5");
    await waitFor(() => expect(screen.getByTestId("variable-row-clean")).toBeInTheDocument());
    expect(viewportStore.getState().statusHint).toBeNull();
  });
});

/** The draft-row add, duplicated here because the suite above scopes its own. */
async function addVariableRow(name: string, value: string) {
  fireEvent.change(screen.getByTestId("variable-new-name"), { target: { value: name } });
  fireEvent.change(screen.getByTestId("variable-new-value"), { target: { value } });
  fireEvent.keyDown(screen.getByTestId("variable-new-value"), { key: "Enter" });
}

/*
 * Text authoring — one field takes `25`, `45deg` and `=w*2`, and the routing to
 * the two write commands is the section's ONLY decision. Whether an expression
 * is any good is the backend's verdict, surfaced verbatim.
 */
describe("VariablesSection — expression authoring", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
    viewportStore.getState().setStatusHint(null);
  });

  async function ready() {
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-new-name")).toBeInTheDocument());
  }

  it("authors an expression from the draft row and shows what it resolves to", async () => {
    await ready();
    await addVariableRow("w", "20");
    await waitFor(() => expect(screen.getByTestId("variable-row-w")).toBeInTheDocument());

    await addVariableRow("plate", "=w*2 + 5mm");

    await waitFor(() => expect(screen.getByTestId("variable-row-plate")).toBeInTheDocument());
    // The field holds what was AUTHORED…
    expect(screen.getByTestId("variable-value-plate")).toHaveValue("=w*2 + 5mm");
    // …and the resolved number is on screen beside it, with its unit.
    expect(screen.getByTestId("variable-resolved-plate")).toHaveTextContent("= 45 mm");
  });

  /** A unit suffix with no `=` is still an expression — the `=` is a
   *  convenience, not the grammar. */
  it("routes a unit-suffixed value through the expression command", async () => {
    await ready();
    await addVariableRow("tilt", "45deg");
    await waitFor(() => expect(screen.getByTestId("variable-row-tilt")).toBeInTheDocument());
    expect(screen.getByTestId("variable-resolved-tilt")).toHaveTextContent("= 45°");
    expect((await mockClient.listVariables())[0]).toMatchObject({
      expr: "45deg",
      dimension: "angle",
    });
  });

  /** A plain number is DIMENSIONLESS, so it gets no unit read-out at all —
   *  claiming one would claim a dimension it does not have. */
  it("shows no resolved line for a plain literal", async () => {
    await ready();
    await addVariableRow("depth", "10");
    await waitFor(() => expect(screen.getByTestId("variable-row-depth")).toBeInTheDocument());
    expect(screen.queryByTestId("variable-resolved-depth")).not.toBeInTheDocument();
  });

  it("typing a plain number over an expression goes back to a literal", async () => {
    await ready();
    await addVariableRow("w", "=8mm");
    await waitFor(() => expect(screen.getByTestId("variable-row-w")).toBeInTheDocument());

    const field = screen.getByTestId("variable-value-w");
    fireEvent.change(field, { target: { value: "3" } });
    fireEvent.blur(field);

    await waitFor(async () =>
      expect((await mockClient.listVariables())[0]).toEqual({
        id: expect.any(String),
        name: "w",
        value: 3,
        resolvedValue: 3,
        dimension: "scalar",
      }),
    );
  });

  it("surfaces the backend's refusal of a bad expression inline, and adds nothing", async () => {
    await ready();
    await addVariableRow("plate", "=gone * 2");
    await waitFor(() =>
      expect(screen.getByTestId("variables-error")).toHaveTextContent(/undefined variable `gone`/),
    );
    expect(await mockClient.listVariables()).toEqual([]);
    // The draft row KEEPS the text so it can be corrected in place.
    expect(screen.getByTestId("variable-new-value")).toHaveValue("=gone * 2");
  });

  /**
   * A variable broken by a LATER edit is a per-row diagnostic, not a refusal —
   * and it is shown where the expression was authored, which is the only place
   * the user can fix it.
   */
  it("badges a row whose expression stopped resolving", async () => {
    await mockClient.upsertVariable("w", 4);
    await mockClient.upsertVariableExpr("plate", "w * 2");
    await ready();
    await waitFor(() => expect(screen.getByTestId("variable-row-plate")).toBeInTheDocument());
    expect(screen.queryByTestId("variable-error-plate")).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId("variable-delete-w"));

    await waitFor(() =>
      expect(screen.getByTestId("variable-error-plate")).toHaveTextContent(
        /undefined variable `w`/,
      ),
    );
    // The stale number is NOT presented as a resolved value beside the reason.
    expect(screen.queryByTestId("variable-resolved-plate")).not.toBeInTheDocument();
  });
});

/*
 * Rename — one transaction that rewrites the table AND every binding, so a
 * bound feature follows the name instead of breaking.
 */
describe("VariablesSection — rename", () => {
  beforeEach(() => {
    resetMockDocument();
    setMockLatency(0);
  });

  async function renameTo(from: string, to: string) {
    const field = screen.getByTestId(`variable-name-${from}`);
    fireEvent.change(field, { target: { value: to } });
    fireEvent.blur(field);
  }

  it("renames the variable and rewrites every reference to it", async () => {
    await mockClient.upsertVariable("w", 5);
    await mockClient.upsertVariableExpr("plate", "w * 2");
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-row-w")).toBeInTheDocument());

    await renameTo("w", "width");

    await waitFor(() => expect(screen.getByTestId("variable-row-width")).toBeInTheDocument());
    expect(screen.queryByTestId("variable-row-w")).not.toBeInTheDocument();
    // The reference followed — and it still resolves.
    expect(screen.getByTestId("variable-value-plate")).toHaveValue("=width * 2");
    expect(screen.getByTestId("variable-resolved-plate")).toHaveTextContent("= 10");
  });

  it("refuses a duplicate name and leaves the table untouched", async () => {
    await mockClient.upsertVariable("w", 1);
    await mockClient.upsertVariable("h", 2);
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-row-w")).toBeInTheDocument());

    await renameTo("w", "h");

    await waitFor(() =>
      expect(screen.getByTestId("variables-error")).toHaveTextContent(/duplicate variable name/),
    );
    expect((await mockClient.listVariables()).map((v) => v.name)).toEqual(["w", "h"]);
  });

  /** Caught before the round trip so the message lands next to the field —
   *  against the BACKEND's own grammar, never a second copy of it. */
  it("refuses an illegal new name inline", async () => {
    await mockClient.upsertVariable("w", 1);
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-row-w")).toBeInTheDocument());

    await renameTo("w", "2wide");

    await waitFor(() =>
      expect(screen.getByTestId("variables-error")).toHaveTextContent("Invalid variable name"),
    );
    expect((await mockClient.listVariables()).map((v) => v.name)).toEqual(["w"]);
  });

  it("an unchanged or emptied name field writes nothing", async () => {
    await mockClient.upsertVariable("w", 1);
    render(<VariablesSection />);
    await waitFor(() => expect(screen.getByTestId("variable-row-w")).toBeInTheDocument());

    await renameTo("w", "w");
    await renameTo("w", "   ");

    expect(screen.queryByTestId("variables-error")).not.toBeInTheDocument();
    expect((await mockClient.listVariables()).map((v) => v.name)).toEqual(["w"]);
  });
});
