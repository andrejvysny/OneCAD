import type { OperationDiagnostic } from "./types";

/**
 * The two `mateAxisReversed` candidate labels (SCHEMA §9, kernel-hardening
 * WP-I; mirrored verbatim in Rust `document/repair.rs` and `dto.rs`). Both
 * rows share a TopoKey and the same score, so `label` — not row order — is
 * what a repair action must route on (a stable sort over equal scores is not
 * a promise of row order).
 */
export const MATE_AXIS_KEEP_LABEL = "Keep the component's direction";
export const MATE_AXIS_FOLLOW_LABEL = "Follow the reversed axis";

const MAX_DIAGNOSTICS = 64;
const MAX_CODE_LENGTH = 128;
const MAX_MESSAGE_LENGTH = 4096;
const MAX_STAGE_LENGTH = 64;
const MAX_EVIDENCE_LENGTH = 65_536;

function boundedEvidence(value: unknown): Record<string, unknown> | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return undefined;
  try {
    return JSON.stringify(value).length <= MAX_EVIDENCE_LENGTH
      ? (value as Record<string, unknown>)
      : undefined;
  } catch {
    return undefined;
  }
}

/** Tolerant additive-detail parser: malformed entries never hide the main error. */
export function parseOperationDiagnostics(value: unknown): OperationDiagnostic[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const diagnostics = value.slice(0, MAX_DIAGNOSTICS).flatMap((item): OperationDiagnostic[] => {
    if (typeof item !== "object" || item === null || Array.isArray(item)) return [];
    const candidate = item as Record<string, unknown>;
    if (
      (candidate.severity !== "info" &&
        candidate.severity !== "warning" &&
        candidate.severity !== "error") ||
      typeof candidate.code !== "string" ||
      typeof candidate.message !== "string" ||
      candidate.code.length > MAX_CODE_LENGTH ||
      candidate.message.length > MAX_MESSAGE_LENGTH
    ) {
      return [];
    }
    const diagnostic: OperationDiagnostic = {
      severity: candidate.severity,
      code: candidate.code,
      message: candidate.message,
    };
    if (typeof candidate.stage === "string" && candidate.stage.length <= MAX_STAGE_LENGTH)
      diagnostic.stage = candidate.stage;
    if (typeof candidate.reasonCode === "string" && candidate.reasonCode.length <= MAX_CODE_LENGTH)
      diagnostic.reasonCode = candidate.reasonCode;
    const evidence = boundedEvidence(candidate.evidence);
    if (evidence) diagnostic.evidence = evidence;
    return [diagnostic];
  });
  return diagnostics.length > 0 ? diagnostics : undefined;
}

/**
 * FE-authored, evidence-driven text for a diagnostic's `reasonCode` (kernel-
 * hardening WP-I; SCHEMA §7.3 Gear/generator bounds). Routes on `reasonCode`,
 * never `message` (per its own doc comment) — falls back to the backend's own
 * `message` for every code this does not know about, so an older or unmapped
 * diagnostic is still shown, just without the FE gloss.
 */
export function diagnosticHint(diagnostic: OperationDiagnostic): string {
  const evidence = diagnostic.evidence ?? {};
  switch (diagnostic.reasonCode) {
    case "GEAR_FACE_NOT_REFERENCEABLE":
      return "This gear face is tooth geometry and cannot be referenced — pick the bore or a cap";
    case "GEAR_PARAM_OUT_OF_RANGE":
    case "GENERATOR_PARAM_OUT_OF_RANGE": {
      const param = typeof evidence.param === "string" ? evidence.param : "parameter";
      const value = evidence.value;
      const min = evidence.min;
      const max = evidence.max;
      const bound =
        min !== undefined && max !== undefined
          ? `between ${min} and ${max}`
          : max !== undefined
            ? `at most ${max}`
            : min !== undefined
              ? `at least ${min}`
              : "out of range";
      return `${param} must be ${bound} (got ${String(value)})`;
    }
    default:
      return diagnostic.message;
  }
}
