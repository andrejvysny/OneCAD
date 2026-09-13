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
import type { MeshAccounting } from "./validateMesh";

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

/** Prepared CPU cost of one validated mesh: the payload plus every derived array. */
export function preparedCpuBytesOf(accounting: MeshAccounting): number {
  return accounting.payloadBytes + accounting.edgeSegmentBytes + accounting.colorBytes;
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
  reserve(bodyId: string, accounting: MeshAccounting): MeshReservation | MeshAdmissionRefusal {
    const cpu = preparedCpuBytesOf(accounting);
    const gpu = accounting.estimatedGpuBytes;
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
