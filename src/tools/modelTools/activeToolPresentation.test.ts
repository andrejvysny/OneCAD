import { beforeEach, describe, expect, expectTypeOf, it, vi } from "vitest";
import { toolChipStore } from "@/stores/toolChipStore";
import { activeToolPresentation } from "./activeToolPresentation";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import type { ActiveToolContextFor } from "./activeToolContext";

describe("activeToolPresentation", () => {
  beforeEach(() => {
    toolChipStore.getState().clear();
    documentStore.setState(seedMockDocument());
  });

  it("keeps each grouped family individually narrowable", () => {
    expectTypeOf<ActiveToolContextFor<"extrudeDepth">>().not.toBeNever();
    expectTypeOf<ActiveToolContextFor<"revolveAngle">>().not.toBeNever();
    expectTypeOf<ActiveToolContextFor<"revolveAxisPick">>().not.toBeNever();
    expectTypeOf<ActiveToolContextFor<"shellThickness">>().not.toBeNever();
    expectTypeOf<ActiveToolContextFor<"offsetFace">>().not.toBeNever();
    expectTypeOf<ActiveToolContextFor<"hole">>().not.toBeNever();
    expectTypeOf<ActiveToolContextFor<"linearPattern">>().not.toBeNever();
    expectTypeOf<ActiveToolContextFor<"circularPattern">>().not.toBeNever();
    expectTypeOf<ActiveToolContextFor<"mirror">>().not.toBeNever();
    expectTypeOf<ActiveToolContextFor<"transform">>().not.toBeNever();
  });

  it("keeps the tool and target-context discriminants paired", () => {
    const fn = vi.fn();
    toolChipStore.getState().showLinearPattern("X", 3, 20, [0, 0, 0], {
      onAxis: fn,
      onCount: fn,
      onSpacing: fn,
      onConfirm: fn,
    });
    toolChipStore.getState().setContext("linearPattern", {
      tool: "linearPattern",
      kind: "bodies",
      bodies: [{ bodyId: "body1" }],
    });
    const presentation = activeToolPresentation(toolChipStore.getState());
    if (presentation?.tool !== "linearPattern") throw new Error("wrong presentation tool");
    expect(presentation.targets).toMatchObject({
      tool: "linearPattern",
      kind: "bodies",
      bodies: [{ bodyId: "body1", label: "Body 1", resolution: "current" }],
    });
  });

  it("derives labels and missing state from the current projection", () => {
    const fn = vi.fn();
    toolChipStore.getState().showBoolean("Cut", [0, 0, 0], { onOp: fn, onConfirm: fn });
    toolChipStore.getState().setContext("booleanOp", {
      tool: "booleanOp",
      kind: "boolean",
      target: { bodyId: "body1" },
      toolBody: { bodyId: "retired" },
    });
    documentStore.setState({ bodies: { body1: { id: "body1", name: "Renamed body", visible: true } } });

    const presentation = activeToolPresentation(toolChipStore.getState());
    if (presentation?.tool !== "booleanOp") throw new Error("wrong presentation tool");
    expect(presentation.targets?.target).toMatchObject({ label: "Renamed body", resolution: "current" });
    expect(presentation.targets?.toolBody).toMatchObject({ resolution: "missing" });
    expect(presentation.canConfirm).toBe(false);
  });

  it("does not treat topology evidence as persistent element identity", () => {
    const fn = vi.fn();
    toolChipStore.getState().showFillet(2, [0, 0, 0], fn, { onConfirm: fn });
    toolChipStore.getState().setContext("filletRadius", {
      tool: "filletRadius",
      kind: "edgeOperation",
      affectedBodies: [{ bodyId: "body1" }],
      edges: [{ kind: "edge", bodyId: "body1", topoKey: "edge:7" }],
      referenceFaces: [{
        a: { kind: "face", bodyId: "body1", elementId: "face-a" },
        b: { kind: "face", bodyId: "body1", topoKey: "face:8" },
      }],
    });

    const presentation = activeToolPresentation(toolChipStore.getState());
    if (presentation?.tool !== "filletRadius") throw new Error("wrong presentation tool");
    expect(presentation.targets?.edges[0]).toMatchObject({ elementId: null, resolution: "unresolved" });
    expect(presentation.targets?.referenceFaces[0]).toMatchObject({
      a: { elementId: "face-a", resolution: "identity-only" },
      b: { elementId: null, resolution: "unresolved" },
    });
  });

  it("projects one hole state into primary, mode and conditional secondary fields", () => {
    const fn = vi.fn();
    toolChipStore.getState().showHole(8, [1, 2, 3], {
      onValue: fn,
      onHoleType: fn,
      onDepth: fn,
      onCbDiameter: fn,
      onCbDepth: fn,
      onCsDiameter: fn,
      onCsAngle: fn,
      onStandard: fn,
      onConfirm: fn,
      onCancel: fn,
    }, { holeType: "counterbore", depth: 20, cbDiameter: 16, cbDepth: 4 });
    toolChipStore.getState().setContext("hole", {
      tool: "hole",
      kind: "faces",
      affectedBodies: [{ bodyId: "body1" }],
      faces: [{ kind: "face", bodyId: "body1", elementId: "face-1" }],
    });

    const p = activeToolPresentation(toolChipStore.getState());
    expect(p?.primary).toMatchObject({ label: "Hole diameter", value: 8, unit: "length" });
    expect(p?.modes).toContainEqual({ id: "holeType", label: "Hole type", value: "counterbore" });
    expect(p?.secondaries.map((f) => f.id)).toEqual(["depth", "cbDiameter", "cbDepth"]);
    expect(p?.canConfirm).toBe(true);
  });

  it("blocks confirmation while a draft is pending or invalid", () => {
    const fn = vi.fn();
    toolChipStore.getState().showFillet(2, [0, 0, 0], fn, { onConfirm: fn });
    toolChipStore.getState().setValidation({ status: "pending", draft: 100, message: "Checking…" });
    expect(activeToolPresentation(toolChipStore.getState())?.canConfirm).toBe(false);
    toolChipStore.getState().setValidation({
      status: "invalid",
      draft: 100,
      message: "Too large",
      suggestedValue: 12,
      suggestedLabel: "Use maximum",
    });
    expect(activeToolPresentation(toolChipStore.getState())?.validation).toMatchObject({ draft: 100 });
    expect(activeToolPresentation(toolChipStore.getState())?.canConfirm).toBe(false);
  });

  it("fails closed when a required tool context was not published", () => {
    const fn = vi.fn();
    toolChipStore.getState().showShell(2, [0, 0, 0], fn, { onConfirm: fn });

    expect(activeToolPresentation(toolChipStore.getState())).toMatchObject({
      tool: "shellThickness",
      targets: null,
      canConfirm: false,
      validation: { status: "invalid", message: expect.stringContaining("unavailable") },
    });
  });

  it("explains a missing target without overwriting authored validation", () => {
    const fn = vi.fn();
    toolChipStore.getState().showShell(2, [0, 0, 0], fn, { onConfirm: fn });
    toolChipStore.getState().setContext("shellThickness", {
      tool: "shellThickness",
      kind: "faces",
      affectedBodies: [{ bodyId: "gone" }],
      faces: [{ kind: "face", bodyId: "gone", elementId: "face-1" }],
    });
    expect(activeToolPresentation(toolChipStore.getState())).toMatchObject({
      canConfirm: false,
      validation: { status: "invalid", draft: 2, message: "Affected body is no longer available" },
    });
    expect(toolChipStore.getState().validation).toEqual({ status: "valid" });

    documentStore.setState({
      bodies: { ...documentStore.getState().bodies, gone: { id: "gone", name: "Restored", visible: true } },
    });
    expect(activeToolPresentation(toolChipStore.getState())).toMatchObject({
      canConfirm: true,
      validation: { status: "valid" },
    });
    expect(toolChipStore.getState().value).toBe(2);
  });

  it("does not invent preview validity from input validity", () => {
    const fn = vi.fn();
    toolChipStore.getState().showShell(2, [0, 0, 0], fn, { onConfirm: fn });
    expect(activeToolPresentation(toolChipStore.getState())?.preview).toBe("none");
  });

  it("keeps a retained-commit failure blocking across ordinary validation updates", () => {
    const fn = vi.fn();
    toolChipStore.getState().showFillet(2, [0, 0, 0], fn, { onConfirm: fn, onCancel: fn });
    toolChipStore.getState().setRetainedCommitFailure("Rollback refused; the failed change remains in history");
    toolChipStore.getState().setValue(3);
    toolChipStore.getState().setValidation({ status: "valid" });
    toolChipStore.getState().setPreviewLifecycle({ status: "valid", arm: 1, request: 2 });

    expect(activeToolPresentation(toolChipStore.getState())).toMatchObject({
      canConfirm: false,
      canCancel: true,
      validation: { status: "invalid", message: expect.stringContaining("remains in history") },
    });
  });

  it("queues confirmation behind pending preview but blocks while applying", () => {
    const fn = vi.fn();
    toolChipStore.getState().showShell(2, [0, 0, 0], fn, { onConfirm: fn });
    toolChipStore.getState().setContext("shellThickness", {
      tool: "shellThickness",
      kind: "faces",
      affectedBodies: [{ bodyId: "body1" }],
      faces: [{ kind: "face", bodyId: "body1", elementId: "face-1" }],
    });
    toolChipStore.getState().setPreviewLifecycle({ status: "pending", arm: 4, request: 2 });
    toolChipStore.getState().setPreviewLifecycle({ status: "valid", arm: 3, request: 99 });
    expect(activeToolPresentation(toolChipStore.getState())).toMatchObject({
      preview: "pending",
      canConfirm: true,
    });
    toolChipStore.getState().setPreviewLifecycle({ status: "applying", arm: 4, request: 2 });
    toolChipStore.getState().setPreviewLifecycle({ status: "valid", arm: 4, request: 2 });
    expect(activeToolPresentation(toolChipStore.getState())).toMatchObject({
      phase: "applying",
      preview: "applying",
      canConfirm: false,
      canCancel: false,
    });
  });
});
