/*
 * The component-preview cache and its refusals (WP-B6).
 *
 * The behaviour worth pinning is not the pixels — jsdom has no WebGL, so
 * rasterization always declines here, which is itself the case that matters:
 * every failure path has to end in `null` and a card that still renders. What
 * this owns is the REQUEST discipline: one fetch per component no matter how
 * many cards mount, and a result (including a null one) that is never fetched
 * twice.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  componentThumbnail,
  previewKey,
  resetComponentPreviews,
} from "./componentPreviewScene";

const componentPreviewMesh = vi.fn();

vi.mock("@/ipc/client", () => ({
  createClient: () => ({ componentPreviewMesh: (...a: unknown[]) => componentPreviewMesh(...a) }),
}));

/** A MESH1-shaped ArrayBuffer is unnecessary: nothing parses it in jsdom. */
function fakeBodies() {
  return [{ bodyId: "preview_1", mesh: new ArrayBuffer(8) }];
}

describe("previewKey", () => {
  it("keys on identity, and on params when they are given", () => {
    expect(previewKey("mine.bracket", "1.0.0")).toBe("mine.bracket@1.0.0");
    expect(previewKey("a.b", "1.0.0", { thread: "M6", length: 20 })).toBe(
      "a.b@1.0.0#length=20,thread=M6",
    );
  });

  it("is stable under param ORDER — two callers must share one cache entry", () => {
    expect(previewKey("a.b", "1.0.0", { thread: "M6", length: 20 })).toBe(
      previewKey("a.b", "1.0.0", { length: 20, thread: "M6" }),
    );
  });

  it("treats an empty param map as no params at all", () => {
    expect(previewKey("a.b", "1.0.0", {})).toBe(previewKey("a.b", "1.0.0"));
  });
});

describe("componentThumbnail", () => {
  beforeEach(() => {
    resetComponentPreviews();
    componentPreviewMesh.mockReset();
    componentPreviewMesh.mockResolvedValue(fakeBodies());
  });

  it("fetches once for concurrent callers — a grid mounts every card at once", async () => {
    const [a, b, c] = await Promise.all([
      componentThumbnail("a.b", "1.0.0"),
      componentThumbnail("a.b", "1.0.0"),
      componentThumbnail("a.b", "1.0.0"),
    ]);
    expect(componentPreviewMesh).toHaveBeenCalledTimes(1);
    expect([a, b, c]).toEqual([null, null, null]); // no WebGL under jsdom
  });

  it("remembers a null result rather than re-asking on every remount", async () => {
    await componentThumbnail("a.b", "1.0.0");
    await componentThumbnail("a.b", "1.0.0");
    expect(componentPreviewMesh).toHaveBeenCalledTimes(1);
  });

  it("keeps different versions and different params apart", async () => {
    await componentThumbnail("a.b", "1.0.0");
    await componentThumbnail("a.b", "1.1.0");
    await componentThumbnail("a.b", "1.0.0", { thread: "M8" });
    expect(componentPreviewMesh).toHaveBeenCalledTimes(3);
  });

  it("returns null when the backend refuses, instead of throwing into a render", async () => {
    componentPreviewMesh.mockRejectedValue(new Error("no worker is running"));
    await expect(componentThumbnail("a.b", "1.0.0")).resolves.toBeNull();
  });

  it("returns null when the component meshes to nothing", async () => {
    componentPreviewMesh.mockResolvedValue([]);
    await expect(componentThumbnail("a.b", "1.0.0")).resolves.toBeNull();
  });
});
