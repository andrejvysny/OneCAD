/*
 * treeActions — the model tree's backend-backed visibility + rename dispatch
 * (TRUST wave). Verifies the EXACT `EditCommand` payloads (the Rust serde shapes:
 * `EditCommand` internally tagged on `cmd`; `VisibilityTarget` externally tagged),
 * the bare-uuid body normalization, and the optimistic-write / revert contract.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  deleteSketch,
  renameBody,
  renameSketch,
  setBodyColor,
  setFaceColor,
  setBodyVisible,
  setSketchVisible,
} from "./treeActions";
import { mockClient } from "@/ipc/mockClient";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

beforeEach(() => resetStores());

const body = (id: string) => documentStore.getState().bodies[id];
const sketch = (id: string) => documentStore.getState().sketches[id];

describe("treeActions — wire payloads", () => {
  it("setBodyVisible sends SetVisibility with an externally-tagged body target", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await setBodyVisible("body1", false);
    expect(apply.mock.calls[0][0]).toEqual({
      cmd: "setVisibility",
      target: { body: "body1" },
      visible: false,
    });
    apply.mockRestore();
  });

  it("setBodyVisible strips the `body_` worker-wire prefix (BodyId is a bare uuid on the core)", async () => {
    const uuid = "11111111-2222-3333-4444-555555555555";
    documentStore.setState({
      bodies: { [`body_${uuid}`]: { id: `body_${uuid}`, name: "Body 1", visible: true } },
    });
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await setBodyVisible(`body_${uuid}`, false);
    expect(apply.mock.calls[0][0]).toEqual({
      cmd: "setVisibility",
      target: { body: uuid },
      visible: false,
    });
    apply.mockRestore();
  });

  it("setSketchVisible sends SetVisibility with a sketch target (sketch ids are already bare)", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await setSketchVisible("sketch4", false);
    expect(apply.mock.calls[0][0]).toEqual({
      cmd: "setVisibility",
      target: { sketch: "sketch4" },
      visible: false,
    });
    apply.mockRestore();
  });

  it("renameBody sends RenameBody with the trimmed name + a bare body id", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await renameBody("body1", "  Housing  ");
    expect(apply.mock.calls[0][0]).toEqual({ cmd: "renameBody", body: "body1", name: "Housing" });
    expect(body("body1").name).toBe("Housing");
    apply.mockRestore();
  });

  it("renameSketch sends RenameSketch", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await renameSketch("sketch4", "Profile");
    expect(apply.mock.calls[0][0]).toEqual({
      cmd: "renameSketch",
      sketch: "sketch4",
      name: "Profile",
    });
    expect(sketch("sketch4").name).toBe("Profile");
    apply.mockRestore();
  });

  it("setBodyColor sends SetBodyColor with a bare body id + rgba", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await setBodyColor("body1", [255, 128, 64, 200]);
    expect(apply.mock.calls[0][0]).toEqual({
      cmd: "setBodyColor",
      body: "body1",
      color: [255, 128, 64, 200],
    });
    expect(body("body1").color).toEqual([255, 128, 64, 200]);
    apply.mockRestore();
  });

  it("setBodyColor clears color with null", async () => {
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true, color: [1, 2, 3, 4] },
      },
    });
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await setBodyColor("body1", null);
    expect(apply.mock.calls[0][0]).toEqual({
      cmd: "setBodyColor",
      body: "body1",
      color: null,
    });
    expect(body("body1").color).toBeUndefined();
    apply.mockRestore();
  });

  it("setFaceColor sends SetFaceColor with a bare body id + element id + rgba", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await setFaceColor("body1", "el_abc", [255, 128, 64, 200]);
    expect(apply.mock.calls[0][0]).toEqual({
      cmd: "setFaceColor",
      body: "body1",
      elementId: "el_abc",
      color: [255, 128, 64, 200],
    });
    expect(body("body1").faceColors).toEqual({ el_abc: [255, 128, 64, 200] });
    apply.mockRestore();
  });

  it("setFaceColor clears a face color with null", async () => {
    documentStore.setState({
      bodies: {
        body1: {
          id: "body1",
          name: "Body 1",
          visible: true,
          faceColors: { el_abc: [1, 2, 3, 4] },
        },
      },
    });
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await setFaceColor("body1", "el_abc", null);
    expect(apply.mock.calls[0][0]).toEqual({
      cmd: "setFaceColor",
      body: "body1",
      elementId: "el_abc",
      color: null,
    });
    expect(body("body1").faceColors).toBeUndefined();
    apply.mockRestore();
  });
});

describe("treeActions — refusals", () => {
  it("refuses an empty/whitespace rename without sending anything", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    expect(await renameBody("body1", "   ")).toBe(false);
    expect(apply).not.toHaveBeenCalled();
    expect(body("body1").name).toBe("Body 1");
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
    apply.mockRestore();
  });

  it("spends no undo step on a rename to the SAME name", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    expect(await renameBody("body1", "Body 1")).toBe(true);
    expect(apply).not.toHaveBeenCalled();
    apply.mockRestore();
  });

  it("refuses to delete the sketch currently open for editing", async () => {
    viewportStore.setState({ activeSketchId: "sketch2" });
    const del = vi.spyOn(mockClient, "deleteSketch");
    expect(await deleteSketch("sketch2")).toBe(false);
    expect(del).not.toHaveBeenCalled();
    expect(sketch("sketch2")).toBeDefined();
    del.mockRestore();
  });
});

describe("treeActions — optimistic write + revert", () => {
  it("flips visibility optimistically BEFORE the command resolves", async () => {
    let release: () => void = () => {};
    const apply = vi.spyOn(mockClient, "applyEditCommand").mockImplementation(
      () =>
        new Promise((resolve) => {
          release = () =>
            resolve({ revision: 9, changedBodies: [], removedBodies: [], features: [] });
        }),
    );
    const pending = setBodyVisible("body1", false);
    expect(body("body1").visible).toBe(false); // already flipped, command still in flight
    release();
    await pending;
    expect(body("body1").visible).toBe(false);
    apply.mockRestore();
  });

  it("reverts the visibility flip and hints when the backend rejects", async () => {
    const apply = vi
      .spyOn(mockClient, "applyEditCommand")
      .mockRejectedValue(new Error("body not found"));
    expect(await setBodyVisible("body1", false)).toBe(false);
    expect(body("body1").visible).toBe(true); // reverted
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("body not found");
    apply.mockRestore();
  });

  /*
   * U0 red evidence. `dispatch` (treeActions.ts:98-113) catches a REJECTION only.
   * A regen that RESOLVES carrying `terminal:"failed"` + `errorMessage` is the
   * documented failure shape (`ApplyOperationResult.errorMessage`, types.ts:1362),
   * and `classifyRegen` exists to read it — but this family never adopted it, so
   * the optimistic flip stands, no hint appears, and the caller is told `true`.
   * RED until U1 routes every family through the classifier.
   */
  it("reverts the visibility flip when the backend RESOLVES a failure", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand").mockResolvedValue({
      revision: 3,
      features: [],
      changedBodies: [],
      removedBodies: [],
      terminal: "failed",
      errorMessage: "body is suppressed",
    } as unknown as Awaited<ReturnType<typeof mockClient.applyEditCommand>>);

    expect(await setBodyVisible("body1", false)).toBe(false);
    expect(body("body1").visible).toBe(true); // reverted
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("body is suppressed");
    apply.mockRestore();
  });

  it("reverts the name and hints when a rename is rejected", async () => {
    const apply = vi
      .spyOn(mockClient, "applyEditCommand")
      .mockRejectedValue(new Error("rename boom"));
    expect(await renameSketch("sketch4", "Profile")).toBe(false);
    expect(sketch("sketch4").name).toBe("Sketch 4"); // reverted
    expect(viewportStore.getState().statusHint?.message).toContain("rename boom");
    apply.mockRestore();
  });

  it("hydrates the feature timeline from the command's projection", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand").mockResolvedValue({
      revision: 42,
      changedBodies: [],
      removedBodies: [],
      features: [
        { id: "fx", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "5.0 mm", status: "ok" },
      ],
    });
    await setBodyVisible("body1", false);
    expect(documentStore.getState().revision).toBe(42);
    expect(documentStore.getState().features.map((f) => f.id)).toEqual(["fx"]);
    apply.mockRestore();
  });
});

describe("treeActions — mock lane authority (the honest-e2e prerequisite)", () => {
  it("a command-backed hide survives a subsequent op commit; a local-only flip does not", async () => {
    // Local-only (the pre-TRUST tree behaviour): the mock re-asserts its own
    // metadata on every commit, exactly as `projection-updated` does in the real
    // lane, so the flip is silently reverted.
    documentStore.getState().setVisibility("body1", false);
    await mockClient.applyOperation({
      opType: "Shell",
      params: { thickness: 1, openFaces: [], targetBodyId: "body1" },
    });
    expect(body("body1").visible).toBe(true);

    // Command-backed: the backend owns it, so it survives.
    await setBodyVisible("body1", false);
    await mockClient.applyOperation({
      opType: "Shell",
      params: { thickness: 1, openFaces: [], targetBodyId: "body1" },
    });
    expect(body("body1").visible).toBe(false);
  });
});
