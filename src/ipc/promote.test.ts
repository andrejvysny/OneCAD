/*
 * The shared promote lane's degradation contract (HISTORY-HARDEN H4).
 *
 * Promotion is snapshot-scoped on both sides of the wire now, so a refusal is an
 * ORDINARY outcome (pick → edit → act on the old pick), not a worker fault. Every
 * call site depends on `promoteOne` turning that into a status hint plus `null` —
 * never a throw, and never a guessed element.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { promoteOne, STALE_PICK_HINT } from "./promote";
import type { CadClient } from "./client";
import type { PromotedElement, PromotePick } from "./types";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import { documentStore } from "@/stores/documentStore";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { makeBoxMesh } from "./mockMeshes";
import * as registry from "@/viewport/mesh/meshRegistry";

const PICK: PromotePick = { topoKey: "f:22", anchor: { worldPoint: [1, 2, 3] } };

/** A `CadClient` stub whose ONLY live method is `promoteSelection`. */
function clientWith(promoteSelection: CadClient["promoteSelection"]): CadClient {
  return { promoteSelection } as unknown as CadClient;
}

function hint(): string | undefined {
  return viewportStore.getState().statusHint?.message;
}

describe("promoteOne", () => {
  beforeEach(() => {
    resetStores();
    registry.disposeAll();
  });

  it("fails closed before IPC when an installed candidate is no longer current", async () => {
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()),
      "body_1",
      1,
      undefined,
      undefined,
      { documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 7, generation: 3 },
    );
    registry.swap("body_1", entry);
    registry.setCurrentMeshPublication({ documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 8, generation: 4 });
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
      bodies: { body_1: { id: "body_1", name: "Body", visible: true } },
    });
    const promoteSelection = vi.fn();
    const out = await promoteOne(
      clientWith(promoteSelection),
      "body_1",
      { topoKey: "f:0", kind: "face" },
      8,
      { entry, kind: "face", topoKey: "f:0" },
    );
    expect(out).toBeNull();
    expect(promoteSelection).not.toHaveBeenCalled();
  });

  it("treats an isolation-hidden installed candidate as nonactionable", async () => {
    const provenance = { documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 7, generation: 3 } as const;
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body_1", 1, undefined, undefined, provenance,
    );
    registry.swap("body_1", entry);
    registry.setCurrentMeshPublication(provenance);
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
      bodies: { body_1: { id: "body_1", name: "Body", visible: true } },
    });
    viewportStore.setState({ isolatedBodyIds: ["some-other-body"] });
    const promoteSelection = vi.fn();
    await expect(promoteOne(
      clientWith(promoteSelection), "body_1", { topoKey: "f:0", kind: "face" }, 7,
      { entry, kind: "face", topoKey: "f:0" },
    )).resolves.toBeNull();
    expect(promoteSelection).not.toHaveBeenCalled();
  });

  it("revalidates exact installed object identity after asynchronous promotion", async () => {
    const provenance = { documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 7, generation: 3 } as const;
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body_1", 1, undefined, undefined, provenance,
    );
    registry.swap("body_1", entry);
    registry.setCurrentMeshPublication(provenance);
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
      bodies: { body_1: { id: "body_1", name: "Body", visible: true } },
    });
    let resolve!: (value: PromotedElement[]) => void;
    const pending = new Promise<PromotedElement[]>((done) => { resolve = done; });
    const invoke = vi.fn(() => pending);
    const result = promoteOne(
      clientWith(invoke),
      "body_1",
      { topoKey: "f:0", kind: "face" },
      7,
      { entry, kind: "face", topoKey: "f:0" },
    );
    expect(invoke).toHaveBeenCalledOnce();
    registry.swap(
      "body_1",
      registry.buildBodyObjects(
        parseMeshPayload(makeBoxMesh()), "body_1", 2, undefined, undefined, provenance,
      ),
    );
    resolve([{ topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "body_1" }]);
    await expect(result).resolves.toBeNull();
  });

  it("invokes IPC for a current proof and validates the returned identity", async () => {
    const provenance = { documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 7, generation: 3 } as const;
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body_1", 1, undefined, undefined, provenance,
    );
    registry.swap("body_1", entry);
    registry.setCurrentMeshPublication(provenance);
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
      bodies: { body_1: { id: "body_1", name: "Body", visible: true } },
    });
    const promoted = { topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "body_1" } as const;
    const invoke = vi.fn(async () => [promoted]);
    await expect(promoteOne(
      clientWith(invoke), "body_1", { topoKey: "f:0", kind: "face" }, 7,
      { entry, kind: "face", topoKey: "f:0" },
    )).resolves.toEqual(promoted);
    expect(invoke).toHaveBeenCalledWith(
      "body_1", [{ topoKey: "f:0", kind: "face" }], 7, "runtime-1",
    );
  });

  it("rejects a proof whose kind disagrees with the pick before IPC", async () => {
    const provenance = { documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 7, generation: 3 } as const;
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body_1", 1, undefined, undefined, provenance,
    );
    registry.swap("body_1", entry);
    registry.setCurrentMeshPublication(provenance);
    documentStore.setState({ documentId: "doc-1", runtimeSession: "runtime-1", geometrySource: "live", bodies: { body_1: { id: "body_1", name: "Body", visible: true } } });
    const invoke = vi.fn();
    await expect(promoteOne(
      clientWith(invoke), "body_1", { topoKey: "f:0", kind: "edge" }, 7,
      { entry, kind: "face", topoKey: "f:0" },
    )).resolves.toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("returns the promoted element and leaves the status bar alone on success", async () => {
    const el: PromotedElement = {
      topoKey: "f:22",
      elementId: "el_1",
      kind: "face",
      bodyId: "body_1",
    };
    const out = await promoteOne(clientWith(async () => [el]), "body_1", PICK);
    expect(out).toEqual(el);
    expect(hint()).toBeUndefined();
  });

  it("keeps legacy snapshot-scoped promotion at three arguments", async () => {
    const promoted = { topoKey: "f:22", elementId: "el_1", bodyId: "body_1" };
    const invoke = vi.fn(async () => [promoted]);
    await expect(promoteOne(clientWith(invoke), "body_1", PICK, 7)).resolves.toEqual(promoted);
    expect(invoke).toHaveBeenCalledWith("body_1", [PICK], 7);
  });

  it("keeps strict returned identity checks on proof-bearing promotion", async () => {
    const provenance = { documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 7, generation: 3 } as const;
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body_1", 1, undefined, undefined, provenance,
    );
    registry.swap("body_1", entry);
    registry.setCurrentMeshPublication(provenance);
    documentStore.setState({ documentId: "doc-1", runtimeSession: "runtime-1", geometrySource: "live", bodies: { body_1: { id: "body_1", name: "Body", visible: true } } });
    const proof = { entry, kind: "face" as const, topoKey: "f:0" };

    await expect(promoteOne(
      clientWith(async () => [{ topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "other" }]),
      "body_1", { topoKey: "f:0", kind: "face" }, 7, proof,
    )).resolves.toBeNull();
    await expect(promoteOne(
      clientWith(async () => [{ topoKey: "f:0", elementId: "el_1", kind: "edge", bodyId: "body_1" }]),
      "body_1", { topoKey: "f:0", kind: "face" }, 7, proof,
    )).resolves.toBeNull();
  });

  it("hints and returns null when the backend resolved nothing", async () => {
    // An unresolvable pick is OMITTED from the reply — an empty array IS the
    // signal, and it must not read as "success with no id".
    const out = await promoteOne(clientWith(async () => []), "body_1", PICK);
    expect(out).toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("hints and returns null when the backend REFUSES a stale pick", async () => {
    // The shape the Rust gate produces: a recoverable OpFailed naming both ids.
    const out = await promoteOne(
      clientWith(async () => {
        throw new Error("pick was taken against snapshot 4; head is 7 — re-pick");
      }),
      "body_1",
      PICK,
    );
    expect(out).toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("does not throw out of the tool when promotion rejects", async () => {
    await expect(
      promoteOne(
        clientWith(async () => {
          throw new Error("worker gone");
        }),
        "body_1",
        PICK,
      ),
    ).resolves.toBeNull();
  });

  it("degrades the same way before the first publish (snapshotId 0)", async () => {
    // `tauriClient` starts at currentSnapshotId 0 and only adopts a POSITIVE
    // published id, so a pick made before any `document-changed` addresses
    // snapshot 0 against a positive head and comes back refused.
    let sent = -1;
    const client = clientWith(async (_bodyId, picks) => {
      sent = picks.length;
      throw new Error("pick was taken against snapshot 0; head is 3 — re-pick");
    });
    const out = await promoteOne(client, "body_1", PICK);
    expect(sent).toBe(1);
    expect(out).toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("short-circuits a pick that is ALREADY an ElementId — no wire call, no hint", async () => {
    // The MESH1 id table is two-namespace: the worker substitutes a minted
    // ElementId for any element with a live binding, and `Picker.ts` hands that
    // label back verbatim as the pick's topoKey. `AcquireElementIds` parses only
    // `f:N`/`e:N`/`v:N` and would drop it — "Selection is out of date" on a fresh
    // pick. There is nothing to promote: the label IS the persistent handle.
    const promoteSelection = vi.fn();
    const out = await promoteOne(clientWith(promoteSelection), "body_1", {
      topoKey: "el_4f2a",
      anchor: { worldPoint: [1, 2, 3] },
    });
    expect(promoteSelection).not.toHaveBeenCalled();
    expect(out?.elementId).toBe("el_4f2a");
    expect(out?.topoKey).toBe("el_4f2a");
    expect(out?.bodyId).toBe("body_1");
    expect(hint()).toBeUndefined();
  });

  it("carries the CALLER's kind on the short-circuit, and omits it when unknown", async () => {
    // The mesh label carries no kind, so the short-circuit has none of its own.
    // `""` is outside `PromotedElement.kind`'s documented `face|edge|vertex`
    // domain; the caller's own pick knows what it clicked, so it supplies it.
    const promoteSelection = vi.fn();
    const typed = await promoteOne(clientWith(promoteSelection), "body_1", {
      topoKey: "el_4f2a",
      kind: "edge",
    });
    expect(typed?.kind).toBe("edge");

    const untyped = await promoteOne(clientWith(promoteSelection), "body_1", {
      topoKey: "el_4f2a",
    });
    expect(untyped?.kind).toBeUndefined();
    expect(promoteSelection).not.toHaveBeenCalled();
  });

  it("keeps the hint sticky so it survives the next repaint", async () => {
    await promoteOne(clientWith(async () => []), "body_1", PICK);
    expect(viewportStore.getState().statusHint?.sticky).toBe(true);
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });
});
