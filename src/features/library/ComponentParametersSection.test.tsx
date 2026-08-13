/*
 * Component Library WP-2.4 — the configurator's role/domain-aware rendering
 * and its role=free-only commit path.
 */
import { beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ComponentParametersSection, formatDesignation } from "./ComponentParametersSection";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import type { ComponentUpgrade, LibraryComponent, ReplaceComponentReport } from "@/ipc/types";

const getOperationParams = vi.fn();
const listLibraryComponents = vi.fn<() => Promise<LibraryComponent[]>>();
const setComponentParams = vi.fn(() => Promise.resolve());
const componentUpgradeAvailable = vi.fn<() => Promise<ComponentUpgrade | null>>();
const replaceComponent = vi.fn<() => Promise<ReplaceComponentReport>>();
vi.mock("@/ipc/client", () => ({
  createClient: () => ({
    getOperationParams,
    listLibraryComponents,
    setComponentParams,
    componentUpgradeAvailable,
    replaceComponent,
  }),
}));

const SHCS: LibraryComponent = {
  id: "onecad.std.iso4762",
  version: "1.0.0",
  name: "Socket Head Cap Screw",
  category: [],
  tags: [],
  sourceKind: "generator",
  revision: `sha256:${"0".repeat(64)}`,
  generatorId: "iso4762",
  generatorVersion: 1,
  attachments: {},
  parameters: {
    thread: { role: "free", key: "M6", domain: ["M3", "M4", "M5", "M6", "M8"] },
    length: { role: "free", value: 20, min: 4 },
    head_d: { role: "table", from: "iso4762.dk" },
  },
  // No literal "M" before `{thread}`: the stored thread VALUE is already the
  // full designation ("M6"), matching the worker/library table convention
  // (WP-2.1/2.2) — unlike the spec's own doc example, which pairs a literal
  // "M" with a bare-number thread value that isn't what this codebase stores.
  designation: "ISO 4762 {thread}x{length}",
};

function selectPlaceComponentFeature(overrideParams: Record<string, unknown> = {}) {
  documentStore.setState({
    features: [
      {
        id: "comp1",
        kind: "boolean",
        opType: "PlaceComponent",
        label: "Place Component",
        valueText: "",
        status: "ok",
      },
    ],
  });
  selectionStore.getState().set([{ kind: "feature", id: "comp1" }]);
  getOperationParams.mockResolvedValue({
    componentId: SHCS.id,
    componentVersion: SHCS.version,
    params: overrideParams,
  });
  listLibraryComponents.mockResolvedValue([SHCS]);
  componentUpgradeAvailable.mockResolvedValue(null);
  replaceComponent.mockResolvedValue({});
}

beforeEach(() => {
  getOperationParams.mockReset();
  listLibraryComponents.mockReset();
  setComponentParams.mockClear();
  componentUpgradeAvailable.mockReset();
  replaceComponent.mockReset();
  documentStore.setState({ features: [] });
  selectionStore.setState({ selected: [], hover: null });
});

describe("formatDesignation", () => {
  it("substitutes known keys and leaves unresolved ones verbatim", () => {
    expect(formatDesignation("ISO 4762 {thread}x{length}", { thread: "M8", length: 25 })).toBe(
      "ISO 4762 M8x25",
    );
    expect(formatDesignation("{thread} {head_d}", { thread: "M6" })).toBe("M6 {head_d}");
  });
});

describe("ComponentParametersSection", () => {
  it("renders nothing when the selected feature isn't PlaceComponent", async () => {
    documentStore.setState({
      features: [
        { id: "f1", kind: "boolean", opType: "Extrude", label: "Extrude", valueText: "", status: "ok" },
      ],
    });
    selectionStore.getState().set([{ kind: "feature", id: "f1" }]);
    const { container } = render(<ComponentParametersSection />);
    await new Promise((r) => setTimeout(r, 0));
    expect(container.firstChild).toBeNull();
    expect(getOperationParams).not.toHaveBeenCalled();
  });

  it("renders a domain select + a numeric field, seeded from library defaults", async () => {
    selectPlaceComponentFeature();
    render(<ComponentParametersSection />);

    await waitFor(() => expect(screen.getByText("Component Parameters")).toBeInTheDocument());
    expect(screen.getByText("ISO 4762 M6x20")).toBeInTheDocument();
    expect(screen.getByLabelText("thread value")).toHaveValue("M6");
    expect(screen.getByLabelText("length value")).toHaveValue(20);
    // role: table is display-only, never an editable control.
    expect(screen.queryByLabelText("head_d value")).toBeNull();
  });

  it("seeds from a stored override rather than the package default", async () => {
    selectPlaceComponentFeature({ thread: "M8" });
    render(<ComponentParametersSection />);
    await waitFor(() => expect(screen.getByLabelText("thread value")).toHaveValue("M8"));
    expect(screen.getByText("ISO 4762 M8x20")).toBeInTheDocument();
  });

  it("commits a domain change through setComponentParams", async () => {
    selectPlaceComponentFeature();
    render(<ComponentParametersSection />);
    await waitFor(() => expect(screen.getByLabelText("thread value")).toBeInTheDocument());

    fireEvent.change(screen.getByLabelText("thread value"), { target: { value: "M8" } });

    await waitFor(() =>
      expect(setComponentParams).toHaveBeenCalledWith("comp1", { thread: "M8" }),
    );
  });

  it("rejects a numeric value below the declared minimum without committing", async () => {
    selectPlaceComponentFeature();
    render(<ComponentParametersSection />);
    await waitFor(() => expect(screen.getByLabelText("length value")).toBeInTheDocument());

    const input = screen.getByLabelText("length value");
    fireEvent.change(input, { target: { value: "2" } });
    fireEvent.blur(input);

    await new Promise((r) => setTimeout(r, 0));
    expect(setComponentParams).not.toHaveBeenCalled();
  });
});

// ── WP-B4: opt-in upgrade + replace-in-place ────────────────────────────────

// A SECOND component to replace the placed one with. Deliberately under an
// authored (`me.`) id rather than `onecad.std.*`: the shipped catalog is one
// package, and a fixture wearing a std id the app does not ship reads as a
// claim about the seed catalog instead of what it is — an arbitrary other
// component the picker can offer.
const OTHER_SCREW: LibraryComponent = {
  ...SHCS,
  id: "me.custom.button-head",
  name: "Button Head Screw",
};

describe("upgrade and replace", () => {
  it("offers an upgrade only when one is reported, and never applies it on its own", async () => {
    selectPlaceComponentFeature();
    render(<ComponentParametersSection />);
    await screen.findByText("Component Parameters");
    // No offer ⇒ no row at all, and nothing was swapped just by rendering.
    expect(screen.queryByTestId("component-upgrade")).toBeNull();
    expect(replaceComponent).not.toHaveBeenCalled();
  });

  it("upgrades to the reported version only when the user asks", async () => {
    selectPlaceComponentFeature();
    componentUpgradeAvailable.mockResolvedValue({
      componentId: SHCS.id,
      currentVersion: "1.0.0",
      latestVersion: "1.1.0",
      latestRevision: `sha256:${"b".repeat(64)}`,
    });
    render(<ComponentParametersSection />);

    const apply = await screen.findByTestId("component-upgrade-apply");
    expect(replaceComponent).not.toHaveBeenCalled();
    fireEvent.click(apply);

    await waitFor(() => expect(replaceComponent).toHaveBeenCalledTimes(1));
    // Same id, newer version — an upgrade is a replace onto oneself.
    expect(replaceComponent).toHaveBeenCalledWith("comp1", SHCS.id, "1.1.0");
  });

  it("replaces with the picked component and reports a dropped mate", async () => {
    selectPlaceComponentFeature();
    listLibraryComponents.mockResolvedValue([SHCS, OTHER_SCREW]);
    replaceComponent.mockResolvedValue({ droppedMateAttachment: "shank_axis" });
    render(<ComponentParametersSection />);

    const pick = await screen.findByTestId("component-replace-pick");
    fireEvent.change(pick, { target: { value: `${OTHER_SCREW.id}@${OTHER_SCREW.version}` } });
    fireEvent.click(screen.getByTestId("component-replace-apply"));

    await waitFor(() => expect(replaceComponent).toHaveBeenCalledTimes(1));
    expect(replaceComponent).toHaveBeenCalledWith("comp1", OTHER_SCREW.id, OTHER_SCREW.version);
    // The dropped mate is SAID, not swallowed: the part quietly stopping
    // following its target is exactly the failure this reports.
    await waitFor(() =>
      expect(viewportStore.getState().statusHint?.message ?? "").toContain("shank_axis"),
    );
  });

  it("offers no replace target when the library holds only this component", async () => {
    selectPlaceComponentFeature();
    render(<ComponentParametersSection />);
    await screen.findByText("Component Parameters");
    expect(screen.queryByTestId("component-replace-pick")).toBeNull();
  });
});

/*
 * A `document`-source component (WP-F1.3). Its free params are indistinguishable
 * from a generator's HERE — the section keys on `role`, never on `sourceKind` —
 * and that is the property under test: the re-bake happens behind
 * `setComponentParams`, so the configurator needs no source-kind branch at all.
 *
 * The one thing that IS different is where the default lives: a document param
 * spells the SOURCE VARIABLE NAME in `key` and the number in `value`, so a
 * `key`-first read would show the field as the literal word "depth".
 */
describe("ComponentParametersSection over a document-source component", () => {
  const BRACKET: LibraryComponent = {
    id: "mine.bracket",
    version: "1.0.0",
    name: "Bracket",
    category: ["mine"],
    tags: [],
    sourceKind: "document",
    revision: `sha256:${"c".repeat(64)}`,
    attachments: {},
    parameters: {
      depth: { role: "free", key: "depth", value: 10 },
    },
    designation: "Bracket {depth} deep",
  };

  function selectBracket(overrideParams: Record<string, unknown> = {}) {
    documentStore.setState({
      features: [
        {
          id: "comp1",
          kind: "boolean",
          opType: "PlaceComponent",
          label: "Place Component",
          valueText: "",
          status: "ok",
        },
      ],
    });
    selectionStore.getState().set([{ kind: "feature", id: "comp1" }]);
    getOperationParams.mockResolvedValue({
      componentId: BRACKET.id,
      componentVersion: BRACKET.version,
      params: overrideParams,
    });
    listLibraryComponents.mockResolvedValue([BRACKET]);
    componentUpgradeAvailable.mockResolvedValue(null);
  }

  it("edits a variable-bound free param exactly like a generator's", async () => {
    selectBracket();
    render(<ComponentParametersSection />);

    // The NUMBER, not the variable name the binding spells in `key`.
    await waitFor(() => expect(screen.getByLabelText("depth value")).toHaveValue(10));
    expect(screen.getByText("Bracket 10 deep")).toBeInTheDocument();

    const field = screen.getByLabelText("depth value");
    fireEvent.change(field, { target: { value: "25" } });
    fireEvent.blur(field);

    await waitFor(() => expect(setComponentParams).toHaveBeenCalledTimes(1));
    expect(setComponentParams).toHaveBeenCalledWith("comp1", { depth: 25 });
  });

  it("seeds from the instance's stored override, not the package default", async () => {
    selectBracket({ depth: 25 });
    render(<ComponentParametersSection />);
    await waitFor(() => expect(screen.getByLabelText("depth value")).toHaveValue(25));
    expect(screen.getByText("Bracket 25 deep")).toBeInTheDocument();
  });
});
