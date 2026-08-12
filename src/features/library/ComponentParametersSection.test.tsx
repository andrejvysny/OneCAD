/*
 * Component Library WP-2.4 — the configurator's role/domain-aware rendering
 * and its role=free-only commit path.
 */
import { beforeEach, describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ComponentParametersSection, formatDesignation } from "./ComponentParametersSection";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import type { LibraryComponent } from "@/ipc/types";

const getOperationParams = vi.fn();
const listLibraryComponents = vi.fn<() => Promise<LibraryComponent[]>>();
const setComponentParams = vi.fn(() => Promise.resolve());
vi.mock("@/ipc/client", () => ({
  createClient: () => ({ getOperationParams, listLibraryComponents, setComponentParams }),
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
}

beforeEach(() => {
  getOperationParams.mockReset();
  listLibraryComponents.mockReset();
  setComponentParams.mockClear();
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
