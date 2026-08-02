/*
 * fileActions — the File-menu / shortcut bridge. `createClient()` returns the
 * shared `mockClient` under vitest (no Tauri bridge), and fileActions captured
 * that same object, so `vi.spyOn(mockClient, …)` controls its behaviour directly.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mockClient } from "@/ipc/mockClient";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import { saveDocument, saveDocumentAs, exportStep, exportStl, exportObj, insertStep } from "./fileActions";
import type { ApplyOperationResult } from "@/ipc/types";

beforeEach(() => {
  viewportStore.getState().setStatusHint(null);
  documentStore.setState(seedMockDocument()); // title "Bracket v2"
});
afterEach(() => vi.restoreAllMocks());

const hint = () => viewportStore.getState().statusHint?.message ?? null;

describe("fileActions", () => {
  it("Save shows a transient 'Saved ⟨name⟩' hint on success", async () => {
    await saveDocument();
    expect(hint()).toBe("Saved Bracket v2");
  });

  it("Save with no known path falls back to Save As", async () => {
    vi.spyOn(mockClient, "saveDocument").mockRejectedValueOnce(
      new Error("io error: no save path; provide one"),
    );
    const saveAs = vi
      .spyOn(mockClient, "saveDocumentAs")
      .mockResolvedValue("/Users/andrej/CAD/Foo.onecad");

    await saveDocument();

    expect(saveAs).toHaveBeenCalledTimes(1);
    expect(hint()).toBe("Saved Foo");
  });

  it("Save surfaces a non-path failure as an error hint", async () => {
    vi.spyOn(mockClient, "saveDocument").mockRejectedValueOnce(new Error("disk full"));
    await saveDocument();
    expect(hint()).toBe("Save failed: disk full");
  });

  it("Save As is a no-op (no hint) when the dialog is cancelled", async () => {
    vi.spyOn(mockClient, "saveDocumentAs").mockResolvedValue(null);
    await saveDocumentAs();
    expect(hint()).toBeNull();
  });

  it("Export STEP shows 'Exported ⟨name⟩' on success", async () => {
    vi.spyOn(mockClient, "exportStep").mockResolvedValue("/Users/andrej/CAD/Part.step");
    await exportStep();
    expect(hint()).toBe("Exported Part");
  });

  it("Export STEP is a no-op when the dialog is cancelled", async () => {
    vi.spyOn(mockClient, "exportStep").mockResolvedValue(null);
    await exportStep();
    expect(hint()).toBeNull();
  });

  it("Export STL shows 'Exported ⟨name⟩' on success", async () => {
    vi.spyOn(mockClient, "exportStl").mockResolvedValue("/Users/andrej/CAD/Part.stl");
    await exportStl();
    expect(hint()).toBe("Exported Part");
  });

  it("Export STL is a no-op when the dialog is cancelled", async () => {
    vi.spyOn(mockClient, "exportStl").mockResolvedValue(null);
    await exportStl();
    expect(hint()).toBeNull();
  });

  // ── Import STEP (in-editor lane; Rust owns the dialog) ────────────────────

  /** An `insert_step` result carrying `count` imported bodies. */
  const importResult = (count: number): ApplyOperationResult => ({
    revision: 9,
    changedBodies: Array.from({ length: count }, (_, i) => ({
      bodyId: `body${i + 10}`,
      meshKey: `body${i + 10}:coarse:9`,
    })),
    removedBodies: [],
    features: [],
    opLabel: "Import",
  });

  it("Import STEP reports the imported body count", async () => {
    vi.spyOn(mockClient, "insertStep").mockResolvedValue(importResult(3));
    await insertStep();
    expect(hint()).toBe("Imported 3 bodies");
  });

  it("Import STEP singularizes a one-body import", async () => {
    vi.spyOn(mockClient, "insertStep").mockResolvedValue(importResult(1));
    await insertStep();
    expect(hint()).toBe("Imported 1 body");
  });

  it("Import STEP is a no-op when the dialog is cancelled", async () => {
    vi.spyOn(mockClient, "insertStep").mockResolvedValue(null);
    await insertStep();
    expect(hint()).toBeNull();
  });

  it("Import STEP surfaces a thrown failure with its reason", async () => {
    vi.spyOn(mockClient, "insertStep").mockRejectedValueOnce(new Error("not a STEP file"));
    await insertStep();
    expect(hint()).toBe("Import failed: not a STEP file");
  });

  /** A CORRELATED regen failure comes back as a resolved result carrying
   *  `errorMessage`, not a rejection — it must not read as a success. */
  it("Import STEP surfaces a correlated regen failure", async () => {
    vi.spyOn(mockClient, "insertStep").mockResolvedValue({
      ...importResult(0),
      errorMessage: "STEP entity 42 unsupported",
    });
    await insertStep();
    expect(hint()).toBe("Import failed: STEP entity 42 unsupported");
  });

  it("Export OBJ shows 'Exported ⟨name⟩' on success", async () => {
    vi.spyOn(mockClient, "exportObj").mockResolvedValue("/Users/andrej/CAD/Part.obj");
    await exportObj();
    expect(hint()).toBe("Exported Part");
  });

  it("Export OBJ is a no-op when the dialog is cancelled", async () => {
    vi.spyOn(mockClient, "exportObj").mockResolvedValue(null);
    await exportObj();
    expect(hint()).toBeNull();
  });
});
