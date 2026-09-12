/*
 * The native menu must answer with the SAME code the ⌘-chords do — that is the
 * whole point of routing Rust's `menu-action` verb through here rather than
 * letting the backend act on the document. These tests pin the six verbs onto
 * the six bridges, so a menu item can never grow a second, divergent meaning.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { openDocumentDialog, saveDocument, saveDocumentAs } from "./fileActions";
import { runRedo, runUndo } from "./undoActions";
import { appStore } from "@/stores/appStore";
import { isMenuAction, runMenuAction } from "./menuActions";

vi.mock("./undoActions", () => ({
  runUndo: vi.fn(() => Promise.resolve()),
  runRedo: vi.fn(() => Promise.resolve()),
}));

vi.mock("./fileActions", async (importActual) => {
  const actual = await importActual<typeof import("./fileActions")>();
  return {
    ...actual,
    saveDocument: vi.fn(() => Promise.resolve(null)),
    saveDocumentAs: vi.fn(() => Promise.resolve(null)),
    openDocumentDialog: vi.fn(() => Promise.resolve()),
  };
});

let newProject: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.clearAllMocks();
  newProject = vi.fn(() => Promise.resolve());
  appStore.setState({ newProject } as never);
});

describe("menu router — Edit", () => {
  it("Undo and Redo go through the one undo router, not the client directly", async () => {
    await runMenuAction("undo");
    expect(runUndo).toHaveBeenCalledTimes(1);
    expect(runRedo).not.toHaveBeenCalled();

    await runMenuAction("redo");
    expect(runRedo).toHaveBeenCalledTimes(1);
  });
});

describe("menu router — File", () => {
  it("reuses the same bridges the ⌘ chords call", async () => {
    await runMenuAction("save");
    expect(saveDocument).toHaveBeenCalledTimes(1);

    await runMenuAction("saveAs");
    expect(saveDocumentAs).toHaveBeenCalledTimes(1);
    expect(saveDocument).toHaveBeenCalledTimes(1); // Save As is not a second Save

    await runMenuAction("open");
    expect(openDocumentDialog).toHaveBeenCalledTimes(1);

    await runMenuAction("new");
    expect(newProject).toHaveBeenCalledTimes(1);
  });
});

describe("menu verb guard", () => {
  it("accepts exactly the six verbs Rust emits", () => {
    for (const verb of ["new", "open", "save", "saveAs", "undo", "redo"]) {
      expect(isMenuAction(verb)).toBe(true);
    }
  });

  it("rejects anything else, so a newer shell cannot be guessed at", () => {
    expect(isMenuAction("quit")).toBe(false);
    expect(isMenuAction("Undo")).toBe(false);
    expect(isMenuAction("save_as")).toBe(false);
    expect(isMenuAction(undefined)).toBe(false);
    expect(isMenuAction(7)).toBe(false);
  });
});
