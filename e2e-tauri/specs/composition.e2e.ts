import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { $, browser, expect } from "@wdio/globals";
import "@wdio/tauri-service";

interface Projection {
  status: "empty" | "loading" | "ready";
  documentId: string;
  revision: number;
  dirty: boolean;
  bodies: Record<string, { id: string }>;
  features: Array<{ id: string; opType: string; status: string }>;
  appliedOps: number;
  totalOps: number;
  geometrySource: string;
}

interface CompositionStatus {
  runtime: "tauri";
  workerPath: string;
  binarySha256: string;
  manifest: {
    binarySha256: string;
    protocolVersion: number;
    workerVersion: string;
    quantizationVersion: number;
    solverPolicyVersion: number;
    occt: { version: string; fingerprint: string };
  };
  hello: {
    protocolVersion: number;
    workerVersion: string;
    quantizationVersion: number;
    solverPolicyVersion: number;
    occt: { version: string; fingerprint: string };
  };
  snapshotId: number;
}

interface RegionResult {
  regionIdentityVersion: number;
  regions: Array<{ regionId: string }>;
}

interface EdgeEvidence {
  elementId?: string;
  bodyId?: string;
  anchor?: { worldPoint?: [number, number, number] };
}

interface PreparedEdge {
  snapshotId: number;
  targetBodyId: string;
  edges: EdgeEvidence[];
  refusal?: { code: string; message: string };
}

interface MassProperties {
  bodyId: string;
  volume: number;
}

interface BodyTopology {
  bodyId: string;
  solidCount: number;
  faceCount: number;
}

const SKETCH = "10000000-0000-4000-8000-000000000001";
const EXTRUDE = "20000000-0000-4000-8000-000000000001";
const FILLET = "30000000-0000-4000-8000-000000000001";
const POINTS = [
  "11000000-0000-4000-8000-000000000001",
  "11000000-0000-4000-8000-000000000002",
  "11000000-0000-4000-8000-000000000003",
  "11000000-0000-4000-8000-000000000004",
] as const;
const LINES = [
  "12000000-0000-4000-8000-000000000001",
  "12000000-0000-4000-8000-000000000002",
  "12000000-0000-4000-8000-000000000003",
  "12000000-0000-4000-8000-000000000004",
] as const;

const artifact = resolve(
  process.env.ONECAD_TAURI_E2E_DOCUMENT ?? "e2e-tauri/results/composition.onecad",
);

async function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  const result = await browser.tauri.execute(
    (tauri, cmd, callArgs) => tauri.core.invoke(cmd, callArgs),
    command,
    args,
  );
  return result as T;
}

async function waitProjection(
  predicate: (projection: Projection) => boolean,
  message: string,
): Promise<Projection> {
  let current: Projection | undefined;
  await browser.waitUntil(
    async () => {
      current = await invoke<Projection>("get_projection");
      return predicate(current);
    },
    { timeout: 60_000, interval: 250, timeoutMsg: message },
  );
  if (!current) throw new Error(message);
  return current;
}

function addSketchCommand(): Record<string, unknown> {
  const positions: Array<[number, number]> = [
    [0, 0],
    [20, 0],
    [20, 10],
    [0, 10],
  ];
  return {
    cmd: "addSketch",
    sketch: {
      id: SKETCH,
      name: "Composition fixture",
      plane: {
        origin: [0, 0, 0],
        xAxis: [1, 0, 0],
        yAxis: [0, 1, 0],
        normal: [0, 0, 1],
      },
      attachment: { kind: "world", plane: "XY" },
      entities: [
        ...POINTS.map((id, index) => ({
          kind: "point",
          id,
          at: positions[index],
          construction: false,
        })),
        ...LINES.map((id, index) => ({
          kind: "line",
          id,
          start: POINTS[index],
          end: POINTS[(index + 1) % POINTS.length],
          construction: false,
        })),
      ],
    },
  };
}

async function exactBodyEvidence(bodyId: string): Promise<{
  mass: MassProperties;
  topology: BodyTopology;
}> {
  const wireBody = `body_${bodyId}`;
  const [mass, topology] = await Promise.all([
    invoke<MassProperties>("query_mass_properties", { bodyId: wireBody }),
    invoke<BodyTopology>("query_body_topology", { bodyId: wireBody }),
  ]);
  return { mass, topology };
}

describe("real Tauri composition", () => {
  it("opens fixture, Extrudes, Fillets, Undoes, saves, and reopens", async () => {
    mkdirSync(dirname(artifact), { recursive: true });
    await $("button=New project").waitForDisplayed();

    await invoke("new_document");
    await invoke("apply_edit_command", { command: addSketchCommand() });
    const regions = await invoke<RegionResult>("get_sketch_regions", { sketchId: SKETCH });
    expect(regions.regions).toHaveLength(1);

    await invoke("save_document", { path: artifact, previewPng: null });
    await invoke("close_document");
    await invoke("open_document", { path: artifact });
    const opened = await waitProjection(
      (projection) => projection.status === "ready" && projection.dirty === false,
      "fixture did not reopen cleanly",
    );
    await invoke("apply_edit_command", {
      command: {
        cmd: "addOperation",
        atCursor: true,
        record: {
          recordId: EXTRUDE,
          opType: "Extrude",
          params: {
            profile: {
              sketchId: SKETCH,
              regionId: regions.regions[0].regionId,
              regionIdentityVersion: regions.regionIdentityVersion,
            },
            distance: { value: 12 },
            draftAngleDeg: { value: 0 },
            extrudeMode: "Blind",
            booleanMode: "NewBody",
            twoDirections: false,
            extrudeMode2: "Blind",
            distance2: { value: 0 },
          },
        },
      },
    });
    const extruded = await waitProjection(
      (projection) =>
        projection.geometrySource === "live" &&
        projection.features.some((feature) => feature.id === EXTRUDE && feature.status === "ok") &&
        Object.keys(projection.bodies).length === 1,
      "Extrude did not publish",
    );
    const bodyId = Object.keys(extruded.bodies)[0];
    const box = await exactBodyEvidence(bodyId);
    expect(box.mass.volume).toBeCloseTo(2400, 6);
    expect(box.topology).toMatchObject({ solidCount: 1, faceCount: 6 });

    const status = await invoke<CompositionStatus>("composition_status");
    expect(status.runtime).toBe("tauri");
    expect(status.workerPath).toContain(".app/Contents/MacOS/onecad-worker");
    expect(status.binarySha256).toBe(status.manifest.binarySha256);
    expect(status.hello).toMatchObject({
      protocolVersion: status.manifest.protocolVersion,
      workerVersion: status.manifest.workerVersion,
      quantizationVersion: status.manifest.quantizationVersion,
      solverPolicyVersion: status.manifest.solverPolicyVersion,
      occt: status.manifest.occt,
    });

    const prepared = await invoke<PreparedEdge>("prepare_edge_op", {
      snapshotId: status.snapshotId,
      mode: "Fillet",
      pickedEdges: [{ bodyId: `body_${bodyId}`, topoKey: "e:1", elementId: null }],
      chainTangentEdges: false,
    });
    expect(prepared.refusal).toBeUndefined();
    expect(prepared.edges).toHaveLength(1);
    const edge = prepared.edges[0];
    if (!edge.elementId) throw new Error("PrepareEdgeOp did not promote the edge");

    await invoke("apply_edit_command", {
      command: {
        cmd: "addOperation",
        atCursor: true,
        record: {
          recordId: FILLET,
          opType: "Fillet",
          params: {
            radius: { value: 1 },
            edgeIds: [edge.elementId],
            edges: [
              {
                primary: { bodyId, elementId: edge.elementId, kind: "edge" },
                ...(edge.anchor ? { anchor: edge.anchor } : {}),
              },
            ],
            chainTangentEdges: false,
          },
        },
      },
    });
    const filleted = await waitProjection(
      (projection) =>
        projection.features.some((feature) => feature.id === FILLET && feature.status === "ok"),
      "Fillet did not publish",
    );
    const rounded = await exactBodyEvidence(bodyId);
    expect(rounded.mass.volume).toBeLessThan(box.mass.volume);
    expect(rounded.topology.faceCount).toBeGreaterThan(box.topology.faceCount);
    const filletSnapshot = (await invoke<CompositionStatus>("composition_status")).snapshotId;

    await invoke("undo");
    let undone: Projection | undefined;
    await browser.waitUntil(
      async () => {
        undone = await invoke<Projection>("get_projection");
        const current = await invoke<CompositionStatus>("composition_status");
        return undone.appliedOps === filleted.appliedOps - 1 && current.snapshotId !== filletSnapshot;
      },
      { timeout: 60_000, interval: 250, timeoutMsg: "Undo did not publish restored geometry" },
    );
    if (!undone) throw new Error("Undo projection is unavailable");
    const undoneProjection = undone;
    const restored = await exactBodyEvidence(bodyId);
    expect(restored.mass.volume).toBeCloseTo(box.mass.volume, 6);
    expect(restored.topology).toEqual(box.topology);

    const saved = await invoke<{
      savedRevision: number;
      currentRevision: number;
      clean: boolean;
      path: string;
    }>("save_document", { path: artifact, previewPng: null });
    expect(saved).toMatchObject({
      savedRevision: undoneProjection.revision,
      currentRevision: undoneProjection.revision,
      clean: true,
      path: artifact,
    });

    await invoke("close_document");
    await invoke("open_document", { path: artifact });
    const reopened = await waitProjection(
      (projection) =>
        projection.status === "ready" &&
        projection.dirty === false &&
        projection.geometrySource === "live" &&
        projection.appliedOps === undoneProjection.appliedOps,
      "saved result did not reopen",
    );
    const reopenedBody = Object.keys(reopened.bodies)[0];
    const reopenedEvidence = await exactBodyEvidence(reopenedBody);
    expect(reopened.documentId).toBe(opened.documentId);
    // `documentRevision` is SESSION-SCOPED and does not survive a reopen. This
    // assertion used to read `toBe(saved.currentRevision)` — the first real run of
    // this lane (it had never executed on any machine) measured `0` against `3`,
    // and the expectation, not the product, is what was wrong:
    //   - SCHEMA §3 calls it a "Rust-owned document revision (an edit counter);
    //     ADVISORY stamp, NOT a fencing token (D4)";
    //   - the container persists no revision at all (nothing in `container.rs`
    //     writes or reads one, and the document model has no such field);
    //   - `DocumentRuntime` constructs `revision: AtomicU64::new(0)` per runtime,
    //     and replaying records at open is not an edit, so it bumps nothing.
    // What the reopen must actually preserve is IDENTITY and GEOMETRY, asserted
    // below. A fresh counter on a clean document is the contract.
    expect(reopened.revision).toBe(0);
    expect(reopened.dirty).toBe(false);
    expect(reopenedBody).toBe(bodyId);
    expect(reopenedEvidence).toEqual(restored);
  });
});
