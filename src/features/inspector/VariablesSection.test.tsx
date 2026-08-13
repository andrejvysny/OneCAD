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
    expect(screen.getByTestId("variable-value-height")).toHaveValue(25);
    expect(screen.queryByTestId("variables-empty")).not.toBeInTheDocument();
    // The draft row resets, so a second add does not silently re-submit the first.
    expect(screen.getByTestId("variable-new-name")).toHaveValue("");
    expect(await mockClient.listVariables()).toEqual([
      { id: expect.any(String), name: "height", value: 25 },
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
        { id: expect.any(String), name: "width", value: 42 },
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
        { id: expect.any(String), name: "depth", value: 9 },
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
