/*
 * The mesh preparation plan — VP-HARDENING PR-03A (spec §9 VP05).
 *
 * ── Why this module exists ──────────────────────────────────────────────────
 * Admission used to price a mesh from its MESH1 payload alone, and construction
 * then chose its layout from a SECOND predicate that also looked at appearance
 * metadata (`needsVertexColors(view, bodyColor, authoredFaceColors)`). A body
 * whose colour lives in the DOCUMENT rather than in a FACE_COLORS section was
 * therefore admitted as zero-copy indexed geometry and built as de-indexed
 * position/normal/colour streams: for the review's 1,000,000-triangle body,
 * 24,000,000 bytes charged against 108,000,000 bytes allocated.
 *
 * Two predicates over the same question will drift. There is now exactly one:
 * {@link planMeshPreparation} decides the layout, prices every byte that layout
 * implies, and the result is IMMUTABLE. The reservation is taken against the
 * plan, construction consumes the same plan object, and `buildBodyObjects`
 * asserts in DEV that what it built is what the plan priced.
 *
 * ── What a plan prices ──────────────────────────────────────────────────────
 * Spec §9: "Count estimates include copies, edge expansion, colors, pending
 * old/new meshes, and acceleration indices."
 *
 *   gpuBytes = position + normal + colour + index attributes, plus the expanded
 *              fat-line edge endpoints — the buffers three uploads.
 *   cpuBytes = the RETAINED SOURCE (a zero-copy attribute keeps the whole MESH1
 *              ArrayBuffer alive, not merely the slice it describes), plus every
 *              array preparation materialises on top of it.
 *
 * An indexed mesh materialises nothing for its faces; a de-indexed one
 * materialises positions, normals and colours while still retaining the source
 * the de-indexing reads from. Both keep the payload, so both are charged for it.
 *
 * PURE and SYNCHRONOUS, allocating nothing, with the same CHECKED ARITHMETIC
 * and the same errors-as-values discipline as `validateMeshView` (numerics §10).
 */
import type { Rgba } from "@/ipc/types";
import { hasAuthoredFaceColors } from "./faceColors";
import { MESH_BUDGETS } from "./meshAdmission";
import type { BodyMeshView } from "./parseMeshPayload";
import type { MeshValidationBudgets, MeshValidationCode, ValidatedMesh } from "./validateMesh";

/** Bytes per Float32 attribute component, and per expanded fat-line segment. */
const BYTES_PER_FLOAT = 4;
const BYTES_PER_INDEX = 4;
/** 2 endpoints × 3 floats × 4 bytes — exactly what `expandEdgeSegments` allocates. */
const BYTES_PER_SEGMENT = 24;
/** Every vertex attribute this preparation builds is 3 components wide. */
const COMPONENTS = 3;

/**
 * Appearance metadata that is NOT part of the MESH1 payload: the document's
 * body colour and the user-authored per-face overrides. Both force the
 * de-indexed layout, which is the whole reason the plan cannot be derived from
 * a {@link ValidatedMesh} alone.
 */
export interface MeshAppearanceInputs {
  readonly bodyColor?: Rgba;
  readonly authoredFaceColors?: ReadonlyMap<string, Rgba>;
}

export type MeshLayout = "indexed" | "deindexed";

/**
 * The single layout predicate (spec §9, PR-03A). A mesh needs de-indexed
 * vertex colours when the payload carries authored FACE_COLORS, when the
 * document assigns the body a colour, or when the user has overridden any face
 * colour — three independent sources, one answer, computed in ONE place.
 *
 * De-indexing is required rather than merely convenient: INDICES is grouped by
 * face over SHARED positions, so a vertex on the boundary between two
 * differently coloured faces would have to carry both colours at once
 * (see `faceColors.ts`).
 */
export function needsVertexColors(
  view: BodyMeshView,
  bodyColor?: Rgba,
  authoredFaceColors?: ReadonlyMap<string, Rgba>,
): boolean {
  return (
    hasAuthoredFaceColors(view.faceColors) ||
    bodyColor !== undefined ||
    (authoredFaceColors?.size ?? 0) > 0
  );
}

/**
 * The immutable decision: one layout, and every byte it costs. Produced before
 * any derived allocation, reserved against, and then consumed VERBATIM by
 * construction.
 */
export interface MeshPreparationPlan {
  readonly layout: MeshLayout;
  /** Vertices AFTER the layout: the view's own for `indexed`, 3·T for `deindexed`. */
  readonly vertexCount: number;
  readonly positionBytes: number;
  /** 0 when the payload carries no normals — preparation never invents them. */
  readonly normalBytes: number;
  /** 0 for `indexed`: an indexed mesh has no per-vertex colour attribute. */
  readonly colorBytes: number;
  /** 0 for `deindexed`: de-indexing removes the index buffer entirely. */
  readonly indexBytes: number;
  /** The fat-line segment endpoint buffer `expandEdgeSegments` materialises. */
  readonly edgeExpansionBytes: number;
  /**
   * The MESH1 ArrayBuffer the zero-copy attribute views keep alive. It is the
   * WHOLE buffer, not the sum of the slices: one surviving view pins all of it,
   * header, section table, padding and all.
   */
  readonly sourceRetainedBytes: number;
  /** Prepared CPU bytes the admission charges: retained source + every derived array. */
  readonly cpuBytes: number;
  /** Estimated GPU bytes the admission charges: every attribute, plus the edge expansion. */
  readonly gpuBytes: number;
}

/**
 * Plan-stage refusals, drawn from {@link MeshValidationCode} so a refusal flows
 * through `MeshIngest.rejectPayload` and the preview lane's throttled warn with
 * no new vocabulary.
 *
 * `count-overflow` is defence in depth: `validateMeshView` has already capped
 * the payload, so no mesh that reaches here can reach the safe-integer ceiling.
 * The check stays because numerics §10 forbids computing a byte total that has
 * not been proved representable — not because a caller can trip it today.
 */
export type MeshPlanCode = Extract<MeshValidationCode, "count-overflow" | "payload-budget">;

export type MeshPreparationResult =
  | { readonly ok: true; readonly plan: MeshPreparationPlan }
  | {
      readonly ok: false;
      readonly code: MeshPlanCode;
      readonly detail: string;
      readonly bodyId: string;
    };

/** `a·b` when the exact product is representable, else null (numerics §10). */
function checkedMul(a: number, b: number): number | null {
  const product = a * b;
  return Number.isSafeInteger(product) ? product : null;
}

function checkedAdd(a: number, b: number): number | null {
  const sum = a + b;
  return Number.isSafeInteger(sum) ? sum : null;
}

/** Sum `terms` with checked arithmetic; null if any term or partial sum overflows. */
function checkedSum(terms: readonly (number | null)[]): number | null {
  let total = 0;
  for (const term of terms) {
    if (term === null) return null;
    const next = checkedAdd(total, term);
    if (next === null) return null;
    total = next;
  }
  return total;
}

/**
 * Bytes of backing storage the view keeps alive. Zero-copy attributes are
 * SLICES: a surviving `Float32Array` over the blob pins the whole ArrayBuffer,
 * so the honest price is the buffer, not the slice (the review's point — "a
 * zero-copy view can retain more source storage than its own slice describes").
 *
 * Each distinct buffer is counted ONCE, so a real MESH1 payload — where every
 * section is a view over one ArrayBuffer — costs exactly that buffer. A bundle
 * buffer shared by several bodies is charged to each of them, which over-counts
 * on purpose: any one of those bodies alone is enough to keep all of it
 * resident, and §9 is a safety budget.
 */
function retainedSourceBytes(view: BodyMeshView): number | null {
  const sources: ReadonlyArray<ArrayBufferLike | null | undefined> = [
    view.buffer,
    view.positions.buffer,
    view.indices.buffer,
    view.faceRanges.buffer,
    view.faceIdOffsets.buffer,
    view.faceIdChars.buffer,
    view.normals?.buffer,
    view.edgeRanges?.buffer,
    view.edgePositions?.buffer,
    view.edgeIdOffsets?.buffer,
    view.edgeIdChars?.buffer,
    view.faceBboxes?.buffer,
    view.faceColors?.buffer,
  ];
  const seen = new Set<ArrayBufferLike>();
  let total = 0;
  for (const source of sources) {
    if (!source || seen.has(source)) continue;
    seen.add(source);
    const next = checkedAdd(total, source.byteLength);
    if (next === null) return null;
    total = next;
  }
  return total;
}

/**
 * Decide and price the preparation of one semantically validated mesh.
 *
 * The mesh must already be a {@link ValidatedMesh}: counts, products and the
 * payload cap are proved there, and `accounting.segmentCount` is the edge
 * expansion measured from the ranges themselves. This function adds exactly the
 * one thing validation cannot know — the APPEARANCE, which is document
 * metadata, not payload — and turns the pair into a cost.
 *
 * Returns a refusal rather than throwing: a refused plan is a display state at
 * the ingestion boundary and a skipped body in the preview lanes.
 */
export function planMeshPreparation(
  mesh: ValidatedMesh,
  appearance: MeshAppearanceInputs,
  bodyId: string,
  budgets: Partial<MeshValidationBudgets> = {},
): MeshPreparationResult {
  const payloadCap = budgets.singleMeshPayloadBytes ?? MESH_BUDGETS.singleMeshPayloadBytes;
  const fail = (code: MeshPlanCode, detail: string): MeshPreparationResult => ({
    ok: false,
    code,
    detail,
    bodyId,
  });

  const view = mesh.view;
  const layout: MeshLayout = needsVertexColors(view, appearance.bodyColor, appearance.authoredFaceColors)
    ? "deindexed"
    : "indexed";

  // De-indexing gives every triangle its own three vertices, in the same
  // triangle order (faceColors.ts). `indexWords` is what construction walks.
  const indexWords = checkedMul(COMPONENTS, view.triangleCount);
  if (indexWords === null) {
    return fail("count-overflow", `3·T overflows safe integer arithmetic (T=${view.triangleCount})`);
  }
  const vertexCount = layout === "deindexed" ? indexWords : view.vertexCount;

  const attributeFloats = checkedMul(vertexCount, COMPONENTS);
  const positionBytes = attributeFloats === null ? null : checkedMul(attributeFloats, BYTES_PER_FLOAT);
  const normalBytes = view.normals ? positionBytes : 0;
  const colorBytes = layout === "deindexed" ? positionBytes : 0;
  const indexBytes = layout === "indexed" ? checkedMul(indexWords, BYTES_PER_INDEX) : 0;
  const edgeExpansionBytes = checkedMul(mesh.accounting.segmentCount, BYTES_PER_SEGMENT);

  // Every array preparation MATERIALISES. An indexed mesh materialises nothing
  // for its faces (position/normal/index alias the blob); a de-indexed one
  // materialises all three of its vertex attributes.
  const derivedFaceBytes =
    layout === "deindexed" ? checkedSum([positionBytes, normalBytes, colorBytes]) : 0;

  const gpuBytes = checkedSum([positionBytes, normalBytes, colorBytes, indexBytes, edgeExpansionBytes]);
  const sourceRetainedBytes = retainedSourceBytes(view);
  const cpuBytes = checkedSum([sourceRetainedBytes, derivedFaceBytes, edgeExpansionBytes]);

  if (
    positionBytes === null ||
    normalBytes === null ||
    colorBytes === null ||
    indexBytes === null ||
    edgeExpansionBytes === null ||
    derivedFaceBytes === null ||
    sourceRetainedBytes === null ||
    gpuBytes === null ||
    cpuBytes === null
  ) {
    return fail(
      "count-overflow",
      `${layout} preparation of V=${vertexCount} T=${view.triangleCount} overflows safe integer arithmetic`,
    );
  }

  // Spec §9: one mesh payload INCLUDING its derived expansion. The de-indexed
  // colour expansion is part of that total, and it is only knowable here.
  const peak = Math.max(cpuBytes, gpuBytes);
  if (peak > payloadCap) {
    return fail(
      "payload-budget",
      `${layout} preparation needs ${cpuBytes} prepared CPU / ${gpuBytes} GPU bytes, exceeding the ${payloadCap} byte cap`,
    );
  }

  return {
    ok: true,
    plan: {
      layout,
      vertexCount,
      positionBytes,
      normalBytes,
      colorBytes,
      indexBytes,
      edgeExpansionBytes,
      sourceRetainedBytes,
      cpuBytes,
      gpuBytes,
    },
  };
}
