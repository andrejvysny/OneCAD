/*
 * The shared promote lane's degradation contract (HISTORY-HARDEN H4), and the
 * proof contract VP-HARDENING PR-01 added on top of it.
 *
 * Promotion is snapshot-scoped on both sides of the wire, so a refusal is an
 * ORDINARY outcome (pick → edit → act on the old pick), not a worker fault. Every
 * call site depends on the lane turning that into a status hint plus `null` —
 * never a throw, and never a guessed element.
 *
 * PR-01: a VIEWPORT promotion now has no proof-free form at all. The installed
 * entry the pick's ordinal was read from is mandatory, is checked BEFORE the
 * already-an-ElementId short-circuit, and is checked AGAIN after the reply — so
 * neither a failed replacement nor a publication that lands mid-flight can mint
 * an id for a face the user is no longer looking at. The only proof-free entry
 * point, `promoteAuthoritativeRef`, is separately typed and fenced by an
 * explicit snapshot instead.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  promoteAuthoritativeRef,
  promoteRef,
  promoteViewportPick,
  STALE_PICK_HINT,
  type InstalledPickProof,
} from "./promote";
import type { CadClient } from "./client";
import type { PromotedElement, PromotePick } from "./types";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import { documentStore } from "@/stores/documentStore";
import type { EntityRef } from "@/stores/selectionStore";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { attachPickProof } from "@/viewport/mesh/pickProof";
import { encodeMesh1, makeBoxMesh } from "./mockMeshes";
import * as registry from "@/viewport/mesh/meshRegistry";

const PICK: PromotePick = { topoKey: "f:22", anchor: { worldPoint: [1, 2, 3] } };

const PUBLICATION = {
  documentId: "doc-1",
  runtimeSession: "runtime-1",
  snapshotId: 7,
  generation: 3,
} as const;

/**
 * A mesh whose face id table carries a MINTED ElementId, not a TopoKey — MESH1
 * `IDS_HAVE_ELEMENTIDS` (mesh_format.md §2). This is the label `Picker.ts` hands
 * back verbatim for an element the worker already has a live binding for.
 */
function elementIdMesh(): ArrayBuffer {
  return encodeMesh1({
    positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
    faces: [{ triangles: [[0, 1, 2]], id: "el_4f2a" }],
    idsHaveElementIds: true,
  });
}

/** A `CadClient` stub whose ONLY live method is `promoteSelection`. */
function clientWith(promoteSelection: CadClient["promoteSelection"]): CadClient {
  return { promoteSelection } as unknown as CadClient;
}

function hint(): string | undefined {
  return viewportStore.getState().statusHint?.message;
}

/**
 * Install one body as the CURRENT publication and return a face proof for it —
 * the state a fresh click produces.
 */
function installCurrentBody(
  provenance: registry.MeshProvenance = PUBLICATION,
  topoKey = "f:0",
  blob: ArrayBuffer = makeBoxMesh(),
): { entry: registry.MeshEntry; proof: InstalledPickProof } {
  const entry = registry.buildBodyObjects(
    parseMeshPayload(blob), "body_1", 1, undefined, undefined, provenance,
  );
  registry.swap("body_1", entry);
  registry.setCurrentMeshPublication(provenance);
  documentStore.setState({
    documentId: "doc-1",
    runtimeSession: "runtime-1",
    geometrySource: "live",
    bodies: { body_1: { id: "body_1", name: "Body", visible: true } },
  });
  return { entry, proof: { entry, kind: "face", topoKey } };
}

describe("promoteViewportPick", () => {
  beforeEach(() => {
    resetStores();
    registry.disposeAll();
  });

  it("fails closed before IPC when an installed candidate is no longer current", async () => {
    const { proof } = installCurrentBody();
    registry.setCurrentMeshPublication({ ...PUBLICATION, snapshotId: 8, generation: 4 });
    const promoteSelection = vi.fn();
    const out = await promoteViewportPick(clientWith(promoteSelection), proof, {
      topoKey: "f:0",
      kind: "face",
    });
    expect(out).toBeNull();
    expect(promoteSelection).not.toHaveBeenCalled();
  });

  it("TEST-MESH-05: fails closed before IPC when the installed entry is stale inspection-only", async () => {
    const publication = { ...PUBLICATION, snapshotId: 8, generation: 4 };
    const { entry, proof } = installCurrentBody(publication);
    // A failed replacement demotes the installed geometry in place (WP04): the
    // user still sees it, but it is history, not a current operation target.
    entry.displayState = "stale-inspection-only";
    const promoteSelection = vi.fn();
    const out = await promoteViewportPick(clientWith(promoteSelection), proof, {
      topoKey: "f:0",
      kind: "face",
    });
    expect(out).toBeNull();
    expect(promoteSelection).not.toHaveBeenCalled();
    expect(hint()).toBe(STALE_PICK_HINT);
    // The same proof promotes once the body is current again.
    entry.displayState = "current";
    const ok = vi.fn(async () => [
      { topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "body_1" },
    ]);
    await promoteViewportPick(clientWith(ok), proof, { topoKey: "f:0", kind: "face" });
    expect(ok).toHaveBeenCalledTimes(1);
  });

  it("TEST-PUB-03 / PR-01: refuses an `el_` label read off a stale-inspection-only entry", async () => {
    // The exact hole this package closes. `el_…` labels come out of the SAME
    // two-namespace MESH1 id table as `f:N`, so an ElementId lifted off a mesh
    // whose replacement failed is a persistent-looking handle for a publication
    // that is no longer current. It must be refused, not short-circuited.
    const { entry, proof } = installCurrentBody(PUBLICATION, "el_4f2a", elementIdMesh());
    entry.displayState = "stale-inspection-only";
    const promoteSelection = vi.fn();
    const out = await promoteViewportPick(clientWith(promoteSelection), proof, {
      topoKey: "el_4f2a",
      kind: "face",
    });
    expect(out).toBeNull();
    expect(promoteSelection).not.toHaveBeenCalled();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("treats an isolation-hidden installed candidate as nonactionable", async () => {
    const { proof } = installCurrentBody();
    viewportStore.setState({ isolatedBodyIds: ["some-other-body"] });
    const promoteSelection = vi.fn();
    await expect(
      promoteViewportPick(clientWith(promoteSelection), proof, { topoKey: "f:0", kind: "face" }),
    ).resolves.toBeNull();
    expect(promoteSelection).not.toHaveBeenCalled();
  });

  it("revalidates exact installed object identity after asynchronous promotion", async () => {
    const { proof } = installCurrentBody();
    let resolve!: (value: PromotedElement[]) => void;
    const pending = new Promise<PromotedElement[]>((done) => { resolve = done; });
    const invoke = vi.fn(() => pending);
    const result = promoteViewportPick(clientWith(invoke), proof, { topoKey: "f:0", kind: "face" });
    expect(invoke).toHaveBeenCalledOnce();
    registry.swap(
      "body_1",
      registry.buildBodyObjects(
        parseMeshPayload(makeBoxMesh()), "body_1", 2, undefined, undefined, PUBLICATION,
      ),
    );
    resolve([{ topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "body_1" }]);
    await expect(result).resolves.toBeNull();
  });

  it("refuses a reply that lands after a NEW publication was announced", async () => {
    // Nothing was swapped — the same entry object is still installed — but the
    // head has moved on, so the ordinal the backend just resolved was read
    // against geometry that is no longer the current publication.
    const { proof } = installCurrentBody();
    let resolve!: (value: PromotedElement[]) => void;
    const pending = new Promise<PromotedElement[]>((done) => { resolve = done; });
    const result = promoteViewportPick(clientWith(vi.fn(() => pending)), proof, {
      topoKey: "f:0",
      kind: "face",
    });
    registry.setCurrentMeshPublication({ ...PUBLICATION, snapshotId: 8, generation: 4 });
    resolve([{ topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "body_1" }]);
    await expect(result).resolves.toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("rejects a proof whose kind disagrees with the pick before IPC", async () => {
    const { proof } = installCurrentBody();
    const invoke = vi.fn();
    await expect(
      promoteViewportPick(clientWith(invoke), proof, { topoKey: "f:0", kind: "edge" }),
    ).resolves.toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("rejects a proof whose label disagrees with the pick before IPC", async () => {
    const { proof } = installCurrentBody();
    const invoke = vi.fn();
    await expect(
      promoteViewportPick(clientWith(invoke), proof, { topoKey: "f:3", kind: "face" }),
    ).resolves.toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("invokes IPC for a current proof and validates the returned identity", async () => {
    const { proof } = installCurrentBody();
    const promoted = { topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "body_1" } as const;
    const invoke = vi.fn(async () => [promoted]);
    await expect(
      promoteViewportPick(clientWith(invoke), proof, { topoKey: "f:0", kind: "face" }),
    ).resolves.toEqual(promoted);
    // The snapshot and runtime come from the PROOF's own provenance — there is
    // no caller-supplied snapshot to disagree with it any more.
    expect(invoke).toHaveBeenCalledWith("body_1", [{ topoKey: "f:0", kind: "face" }], 7, "runtime-1");
    expect(hint()).toBeUndefined();
  });

  it("keeps strict returned identity checks on the reply", async () => {
    const { proof } = installCurrentBody();
    await expect(promoteViewportPick(
      clientWith(async () => [{ topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "other" }]),
      proof, { topoKey: "f:0", kind: "face" },
    )).resolves.toBeNull();
    await expect(promoteViewportPick(
      clientWith(async () => [{ topoKey: "f:0", elementId: "el_1", kind: "edge", bodyId: "body_1" }]),
      proof, { topoKey: "f:0", kind: "face" },
    )).resolves.toBeNull();
    await expect(promoteViewportPick(
      clientWith(async () => [{ topoKey: "f:3", elementId: "el_1", kind: "face", bodyId: "body_1" }]),
      proof, { topoKey: "f:0", kind: "face" },
    )).resolves.toBeNull();
  });

  it("hints and returns null when the backend resolved nothing", async () => {
    // An unresolvable pick is OMITTED from the reply — an empty array IS the
    // signal, and it must not read as "success with no id".
    const { proof } = installCurrentBody();
    const out = await promoteViewportPick(
      clientWith(async () => []), proof, { topoKey: "f:0", kind: "face" },
    );
    expect(out).toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("hints and returns null when the backend REFUSES a stale pick", async () => {
    // The shape the Rust gate produces: a recoverable OpFailed naming both ids.
    const { proof } = installCurrentBody();
    const out = await promoteViewportPick(
      clientWith(async () => {
        throw new Error("pick was taken against snapshot 4; head is 7 — re-pick");
      }),
      proof,
      { topoKey: "f:0", kind: "face" },
    );
    expect(out).toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("does not throw out of the tool when promotion rejects", async () => {
    const { proof } = installCurrentBody();
    await expect(
      promoteViewportPick(
        clientWith(async () => { throw new Error("worker gone"); }),
        proof,
        { topoKey: "f:0", kind: "face" },
      ),
    ).resolves.toBeNull();
  });

  it("short-circuits an already-an-ElementId label on a CURRENT proof — no wire call, no hint", async () => {
    // The MESH1 id table is two-namespace: the worker substitutes a minted
    // ElementId for any element with a live binding, and `Picker.ts` hands that
    // label back verbatim as the pick's topoKey. `AcquireElementIds` parses only
    // `f:N`/`e:N`/`v:N` and would drop it — "Selection is out of date" on a fresh
    // pick. There is nothing to promote: the label IS the persistent handle.
    const { proof } = installCurrentBody(PUBLICATION, "el_4f2a", elementIdMesh());
    const promoteSelection = vi.fn();
    const out = await promoteViewportPick(clientWith(promoteSelection), proof, {
      topoKey: "el_4f2a",
      kind: "face",
    });
    expect(promoteSelection).not.toHaveBeenCalled();
    expect(out).toEqual({
      topoKey: "el_4f2a",
      elementId: "el_4f2a",
      // The mesh label carries no kind, so the short-circuit answers with the
      // one the PROOF establishes — always known, unlike a caller's optional
      // declaration.
      kind: "face",
      bodyId: "body_1",
    });
    expect(hint()).toBeUndefined();
  });

  it("keeps the hint sticky so it survives the next repaint", async () => {
    const { proof } = installCurrentBody();
    await promoteViewportPick(clientWith(async () => []), proof, { topoKey: "f:0", kind: "face" });
    expect(viewportStore.getState().statusHint?.sticky).toBe(true);
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });

  it("has no proof-free form at all (compile-time)", () => {
    const client = clientWith(vi.fn());
    // Never invoked: the assertion is that `tsc --noEmit` reports these, which is
    // what `@ts-expect-error` pins. A caller holding only a bodyId has exactly
    // one way in — `promoteAuthoritativeRef` — and it has to name its origin.
    const withoutProof = () =>
      // @ts-expect-error — the proof is MANDATORY on the viewport lane.
      promoteViewportPick(client, PICK);
    const bodyProof = () =>
      promoteViewportPick(
        client,
        // @ts-expect-error — a BODY pick has no element to promote; `kind` is
        // `"face" | "edge"` only.
        { entry: {} as registry.MeshEntry, kind: "body", topoKey: "body_1" },
        PICK,
      );
    expect([typeof withoutProof, typeof bodyProof]).toEqual(["function", "function"]);
  });
});

describe("promoteRef", () => {
  beforeEach(() => {
    resetStores();
    registry.disposeAll();
  });

  const faceRef = (topoKey: string): EntityRef => ({
    kind: "face",
    id: `body_1#${topoKey}`,
    bodyId: "body_1",
    topoKey,
    anchor: { worldPoint: [1, 2, 3] },
  });

  it("promotes a ref that carries its pick-time proof", async () => {
    const { proof } = installCurrentBody();
    const ref = faceRef("f:0");
    attachPickProof(ref, proof);
    const promoted = { topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "body_1" } as const;
    const invoke = vi.fn(async () => [promoted]);
    await expect(promoteRef(clientWith(invoke), ref)).resolves.toEqual(promoted);
    expect(invoke).toHaveBeenCalledWith(
      "body_1", [{ topoKey: "f:0", kind: "face", anchor: { worldPoint: [1, 2, 3] } }], 7, "runtime-1",
    );
  });

  it("FAILS CLOSED on a ref with no proof — never a proof-free wire call", async () => {
    // A ref restored from persistence or undo, or rewritten by a regen
    // reconcile, is not a live viewport pick. There is no fallback: the
    // authoritative lane is the only other way in, and it is not this one.
    installCurrentBody();
    const promoteSelection = vi.fn();
    await expect(promoteRef(clientWith(promoteSelection), faceRef("f:0"))).resolves.toBeNull();
    expect(promoteSelection).not.toHaveBeenCalled();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("refuses a non-element ref", async () => {
    const promoteSelection = vi.fn();
    const bodyRef: EntityRef = { kind: "body", id: "body_1" };
    await expect(promoteRef(clientWith(promoteSelection), bodyRef)).resolves.toBeNull();
    expect(promoteSelection).not.toHaveBeenCalled();
  });
});

describe("promoteAuthoritativeRef", () => {
  beforeEach(() => {
    resetStores();
    registry.disposeAll();
  });

  it("promotes a backend-named label with no proof, fenced by its snapshot", async () => {
    // Nothing is installed at all: a repair candidate names a body that may not
    // even be on screen, so the snapshot is the entire fence.
    const promoted = { topoKey: "f:5", elementId: "el_9", kind: "face", bodyId: "body_1" };
    const invoke = vi.fn(async () => [promoted]);
    await expect(promoteAuthoritativeRef(
      clientWith(invoke), "body_1", { topoKey: "f:5" }, 4, "history-repair",
    )).resolves.toEqual(promoted);
    expect(invoke).toHaveBeenCalledWith("body_1", [{ topoKey: "f:5" }], 4);
    expect(hint()).toBeUndefined();
  });

  it("short-circuits an already-an-ElementId label", async () => {
    const promoteSelection = vi.fn();
    const out = await promoteAuthoritativeRef(
      clientWith(promoteSelection), "body_1", { topoKey: "el_4f2a", kind: "face" }, 4,
      "edge-op-closure",
    );
    expect(promoteSelection).not.toHaveBeenCalled();
    expect(out).toEqual({
      topoKey: "el_4f2a", elementId: "el_4f2a", kind: "face", bodyId: "body_1",
    });
    expect(hint()).toBeUndefined();
  });

  it("hints and returns null on a refusal or an unresolvable label", async () => {
    await expect(promoteAuthoritativeRef(
      clientWith(async () => []), "body_1", { topoKey: "f:5" }, 4, "offset-closure",
    )).resolves.toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
    resetStores();
    await expect(promoteAuthoritativeRef(
      clientWith(async () => { throw new Error("head is 7 — re-pick"); }),
      "body_1", { topoKey: "f:5" }, 4, "offset-closure",
    )).resolves.toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("refuses a reply that does not name the element it asked about", async () => {
    // The viewport lane has always re-checked the reply's identity; this lane
    // had nothing but the snapshot fence. A `promote_selection` that answers
    // about another body, another label, or another kind of element is not an
    // answer to this question, and an authored record would carry it forever.
    await expect(promoteAuthoritativeRef(
      clientWith(async () => [{ topoKey: "f:5", elementId: "el_9", kind: "face", bodyId: "other" }]),
      "body_1", { topoKey: "f:5" }, 4, "history-repair",
    )).resolves.toBeNull();
    await expect(promoteAuthoritativeRef(
      clientWith(async () => [{ topoKey: "f:6", elementId: "el_9", kind: "face", bodyId: "body_1" }]),
      "body_1", { topoKey: "f:5" }, 4, "history-repair",
    )).resolves.toBeNull();
    await expect(promoteAuthoritativeRef(
      clientWith(async () => [{ topoKey: "f:5", elementId: "el_9", kind: "edge", bodyId: "body_1" }]),
      "body_1", { topoKey: "f:5", kind: "face" }, 4, "edge-op-closure",
    )).resolves.toBeNull();
    expect(hint()).toBe(STALE_PICK_HINT);
  });

  it("accepts a reply whose kind the caller never declared", async () => {
    // `historyActions` sends no `kind` — the candidate's TopoKey decides it
    // later — so the backend's own answer cannot be contradicted here.
    const promoted = { topoKey: "f:5", elementId: "el_9", kind: "edge", bodyId: "body_1" };
    await expect(promoteAuthoritativeRef(
      clientWith(async () => [promoted]), "body_1", { topoKey: "f:5" }, 4, "history-repair",
    )).resolves.toEqual(promoted);
  });

  it("cannot be reached without naming a non-viewport origin (compile-time)", () => {
    const client = clientWith(vi.fn());
    const untagged = () =>
      // @ts-expect-error — `origin` is required, and is a closed union of
      // non-viewport lanes.
      promoteAuthoritativeRef(client, "body_1", { topoKey: "f:5" }, 4);
    const wrongOrigin = () =>
      // @ts-expect-error — "viewport" is not an authoritative origin.
      promoteAuthoritativeRef(client, "body_1", { topoKey: "f:5" }, 4, "viewport");
    const unfenced = () =>
      // @ts-expect-error — an explicit snapshot is the whole fence here.
      promoteAuthoritativeRef(client, "body_1", { topoKey: "f:5" }, undefined, "history-repair");
    expect([typeof untagged, typeof wrongOrigin, typeof unfenced])
      .toEqual(["function", "function", "function"]);
  });
});
