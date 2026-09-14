/*
 * Mesh resource admission — VP-HARDENING WP04 (spec §9 VP05).
 *
 * Parsing proves byte layout, validation proves semantics, and admission proves
 * the machine can afford the result. The budgets below are PRODUCT SAFETY
 * limits from the specification's §9 table, not measurements of GPU capacity;
 * they exist so a structurally plausible but enormous payload is refused before
 * anything is allocated rather than after the tab dies.
 *
 * Admission is PEAK, not steady-state. A double-buffered swap has the outgoing
 * body still installed while the incoming one is being prepared, so both must
 * fit at once. That is expressed by holding a reservation for as long as the
 * resource it paid for is installed: `MeshIngest` keeps the reservation of the
 * body currently on screen and only releases it once the replacement has
 * actually swapped in. A refused replacement therefore leaves the old body
 * accounted for and visible, which is exactly the stale-inspection-only policy.
 *
 * Acceptance: TEST-MESH-04, TEST-PUB-01, TEST-PUB-02.
 */
const MIB = 1024 * 1024;

/** Specification §9 resource table. Overridable per instance for tests only. */
export const MESH_BUDGETS = {
  /** One mesh payload, including its derived edge/colour expansion. */
  singleMeshPayloadBytes: 256 * MIB,
  /** Prepared CPU mesh resources across the active document. */
  preparedCpuBytes: 1024 * MIB,
  /** Estimated active geometry GPU buffers across the active document. */
  estimatedGpuBytes: 768 * MIB,
} as const;

export interface MeshAdmissionLimits {
  readonly preparedCpuBytes: number;
  readonly estimatedGpuBytes: number;
}

export type MeshAdmissionReason = "cpu-budget" | "gpu-budget";

/** A granted hold on the budget. Idempotent `release`; holding it keeps the bytes counted. */
export interface MeshReservation {
  readonly ok: true;
  release(): void;
}

export interface MeshAdmissionRefusal {
  readonly ok: false;
  readonly reason: MeshAdmissionReason;
  readonly detail: string;
}

export interface MeshAdmissionSnapshot {
  readonly preparedCpuBytes: number;
  readonly estimatedGpuBytes: number;
  /** Reservations currently held (installed bodies plus in-flight preparations). */
  readonly holdings: number;
}

/**
 * What one prepared mesh costs. The ONLY shape `reserve` accepts, and the only
 * producer of it is `planMeshPreparation` (PR-03A) — so a reservation can never
 * again be priced from the payload while construction builds a different
 * layout. `MeshPreparationPlan` satisfies this structurally.
 */
export interface MeshResourceCost {
  readonly cpuBytes: number;
  readonly gpuBytes: number;
}

export class MeshAdmission {
  private readonly limits: MeshAdmissionLimits;
  private cpuBytes = 0;
  private gpuBytes = 0;
  private holdings = 0;

  constructor(limits: Partial<MeshAdmissionLimits> = {}) {
    this.limits = {
      preparedCpuBytes: limits.preparedCpuBytes ?? MESH_BUDGETS.preparedCpuBytes,
      estimatedGpuBytes: limits.estimatedGpuBytes ?? MESH_BUDGETS.estimatedGpuBytes,
    };
  }

  /**
   * Take a hold on the budget for one prepared mesh. Refused when the request
   * does not fit ALONGSIDE everything currently held — the peak rule.
   */
  reserve(bodyId: string, cost: MeshResourceCost): MeshReservation | MeshAdmissionRefusal {
    const cpu = cost.cpuBytes;
    const gpu = cost.gpuBytes;
    if (this.cpuBytes + cpu > this.limits.preparedCpuBytes) {
      return {
        ok: false,
        reason: "cpu-budget",
        detail: `body ${bodyId} needs ${cpu} prepared CPU bytes; ${this.cpuBytes} of ${this.limits.preparedCpuBytes} already held`,
      };
    }
    if (this.gpuBytes + gpu > this.limits.estimatedGpuBytes) {
      return {
        ok: false,
        reason: "gpu-budget",
        detail: `body ${bodyId} needs ${gpu} estimated GPU bytes; ${this.gpuBytes} of ${this.limits.estimatedGpuBytes} already held`,
      };
    }
    this.cpuBytes += cpu;
    this.gpuBytes += gpu;
    this.holdings++;
    let released = false;
    return {
      ok: true,
      release: () => {
        if (released) return;
        released = true;
        this.cpuBytes -= cpu;
        this.gpuBytes -= gpu;
        this.holdings--;
      },
    };
  }

  /** Current totals, for tests and telemetry. */
  snapshot(): MeshAdmissionSnapshot {
    return {
      preparedCpuBytes: this.cpuBytes,
      estimatedGpuBytes: this.gpuBytes,
      holdings: this.holdings,
    };
  }
}

/**
 * The ACTIVE DOCUMENT's budget. The §9 limits are per document, not per lane,
 * so every lane that puts geometry on the GPU spends this one: `MeshIngest` for
 * committed bodies, and `previewMesh` for exact previews and placement ghosts.
 * A preview that reserved against its own instance would be free as far as the
 * document is concerned, which is how a drag could outspend the budget while
 * admission reported the bodies only.
 */
let documentAdmission = new MeshAdmission();

export function getDocumentAdmission(): MeshAdmission {
  return documentAdmission;
}

/**
 * Test-only: start from an empty budget, optionally a small one. Call it BEFORE
 * building anything that reserves — holds taken against the previous instance
 * keep releasing into that instance, which is now unobservable.
 */
export function __resetDocumentAdmissionForTests(limits: Partial<MeshAdmissionLimits> = {}): void {
  documentAdmission = new MeshAdmission(limits);
}
