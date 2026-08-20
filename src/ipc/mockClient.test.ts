/*
 * mockClient — the new file/worker seam methods (F-WP "make it a real app").
 *
 * The mock keeps vitest deterministic with no backend: save is a no-op, Save As /
 * Export return fake paths, and worker-status never fires (the mock has no worker).
 */
import { describe, it, expect } from "vitest";
import { mockClient, setMockRecovery } from "./mockClient";

describe("mockClient file seam", () => {
  it("saveDocument returns an authoritative outcome with or without a path", async () => {
    await expect(mockClient.saveDocument()).resolves.toMatchObject({ clean: true });
    await expect(mockClient.saveDocument("/tmp/x.onecad")).resolves.toMatchObject({
      path: "/tmp/x.onecad",
      title: "x",
    });
  });

  it("saveDocument accepts (and ignores) a previewPng — no container to write it to", async () => {
    await expect(
      mockClient.saveDocument("/tmp/x.onecad", "data:image/png;base64,AAAA"),
    ).resolves.toMatchObject({ clean: true });
    await expect(mockClient.saveDocument(undefined, null)).resolves.toMatchObject({ clean: true });
  });

  it("saveDocumentAs returns a fake .onecad path", async () => {
    const outcome = await mockClient.saveDocumentAs();
    expect(outcome?.path).toMatch(/\.onecad$/);
  });

  it("saveDocumentAs accepts (and ignores) a previewPng", async () => {
    const outcome = await mockClient.saveDocumentAs("data:image/png;base64,AAAA");
    expect(outcome?.path).toMatch(/\.onecad$/);
  });

  it("exportStep returns a fake .step path", async () => {
    const path = await mockClient.exportStep();
    expect(path).toMatch(/\.step$/);
  });

  it("exportStl returns a fake .stl path", async () => {
    const path = await mockClient.exportStl();
    expect(path).toMatch(/\.stl$/);
  });

  it("exportObj returns a fake .obj path", async () => {
    const path = await mockClient.exportObj();
    expect(path).toMatch(/\.obj$/);
  });

  it("export3mf returns a fake .3mf path", async () => {
    const path = await mockClient.export3mf();
    expect(path).toMatch(/\.3mf$/);
  });

  it("onWorkerStatus never fires and returns a no-op unsubscribe", () => {
    let fired = false;
    const unsub = mockClient.onWorkerStatus(() => {
      fired = true;
    });
    expect(typeof unsub).toBe("function");
    unsub();
    expect(fired).toBe(false);
  });
});

describe("mockClient crash recovery", () => {
  it("checkRecovery is empty by default (no banner unless a test opts in)", async () => {
    setMockRecovery(null);
    expect(await mockClient.checkRecovery()).toEqual([]);
  });

  it("checkRecovery reports the seeded info; recoverDocument(true) restores + clears", async () => {
    const info = {
      documentId: "11111111-1111-1111-1111-111111111111",
      autosavePath: "/x/autosave/foo.onecad",
      originalPath: "/docs/Bracket.onecad",
      modifiedMs: 1_700_000_000_000,
    };
    setMockRecovery(info);
    expect(await mockClient.checkRecovery()).toEqual([info]);

    const snap = await mockClient.recoverDocument(info.documentId, true);
    expect(snap).not.toBeNull();
    expect(snap?.title).toBe("Bracket"); // derived from originalPath basename

    // Consumed: a follow-up check sees nothing.
    expect(await mockClient.checkRecovery()).toEqual([]);
  });

  it("recoverDocument(false) discards the offer and resolves null", async () => {
    const id = "22222222-2222-2222-2222-222222222222";
    setMockRecovery({ documentId: id, autosavePath: "/x/autosave/foo.onecad", modifiedMs: 1 });
    expect(await mockClient.recoverDocument(id, false)).toBeNull();
    expect(await mockClient.checkRecovery()).toEqual([]);
  });

  it("checkRecovery reports every offer, newest first", async () => {
    setMockRecovery([
      { documentId: "a", autosavePath: "/x/a.onecad", modifiedMs: 100 },
      { documentId: "c", autosavePath: "/x/c.onecad", modifiedMs: 300 },
      { documentId: "b", autosavePath: "/x/b.onecad", modifiedMs: 200 },
    ]);
    expect((await mockClient.checkRecovery()).map((o) => o.documentId)).toEqual(["c", "b", "a"]);
  });

  it("openDocument refuses a path an unresolved offer names, unless told what to do", async () => {
    setMockRecovery({
      documentId: "d",
      autosavePath: "/x/d.onecad",
      originalPath: "/docs/Bracket.onecad",
      modifiedMs: 1,
    });
    await expect(mockClient.openDocument("/docs/Bracket.onecad")).rejects.toMatchObject({
      kind: "recoveryPending",
    });
    // The refusal did NOT consume the offer — the user has not decided yet.
    expect(await mockClient.checkRecovery()).toHaveLength(1);

    // An explicit discard opens, and drops the offer with it.
    await expect(mockClient.openDocument("/docs/Bracket.onecad", "openSaved")).resolves.toBeTruthy();
    expect(await mockClient.checkRecovery()).toEqual([]);
  });
});

/*
 * WP4 — the mock's `analyzeEdgeOpRange`. The contract this pins is that the mock
 * does NOT invent a range. There is no kernel here, and the only honest source of
 * a feasible bound is a build that ran, so it reports that nothing was measured
 * and the clamp helper reads that as "do not clamp" — the pre-WP4 behaviour.
 *
 * A plausible fabricated range would be worse than useless: the mock lane drives
 * the whole UI and every Playwright spec, so a made-up ceiling would forbid
 * values on the e2e model that the real kernel accepts.
 */
describe("mockClient analyzeEdgeOpRange", () => {
  it("reports that nothing was measured, rather than a fabricated range", async () => {
    const res = await mockClient.analyzeEdgeOpRange({
      mode: "Fillet",
      chainTangentEdges: true,
      pickedEdges: [{ bodyId: "b1", topoKey: "e:4" }],
    });
    expect(res.confidence).toBe("none");
    expect(res.lowerBound).toBeNull();
    expect(res.bestKnownMax).toBeNull();
    expect(res.provenUpperBound).toBeNull();
    expect(res.feasibleIntervals).toEqual([]);
    expect(res.limitingEntities).toEqual([]);
    // Zero probes for a budget that cannot buy one — said out loud, not implied.
    expect(res.probesUsed).toBe(0);
    expect(res.budgetExhausted).toBe(true);
    expect(res.stoppedReason).toBe("budgetExhausted");
    expect(res.refusal).toBeNull();
    expect(res.mode).toBe("Fillet");
    expect(res.targetBodyId).toBe("b1");
    expect(res.edges).toEqual(["e:4"]);
  });

  it("refuses a cross-body pick exactly as prepareEdgeOp does", async () => {
    // This one IS a real answer: it is a fact about the PICKS, and the mock can
    // see those without a kernel. Both verbs must refuse one gesture identically.
    const picks = [
      { bodyId: "b1", topoKey: "e:4" },
      { bodyId: "b2", topoKey: "e:7" },
    ];
    const prepared = await mockClient.prepareEdgeOp({
      mode: "Chamfer",
      chainTangentEdges: true,
      pickedEdges: picks,
    });
    const range = await mockClient.analyzeEdgeOpRange({
      mode: "Chamfer",
      chainTangentEdges: true,
      pickedEdges: picks,
    });
    expect(prepared.refusal?.code).toBe("crossBody");
    expect(range.refusal?.code).toBe("crossBody");
    expect(range.refusal?.edges).toEqual(prepared.refusal?.edges);
    expect(range.targetBodyId).toBe("");
    expect(range.edges).toEqual([]);
    expect(range.probesUsed).toBe(0);
  });
});
