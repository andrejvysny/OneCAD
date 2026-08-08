import type { OperationDiagnostic } from "./types";

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
    const evidence = boundedEvidence(candidate.evidence);
    if (evidence) diagnostic.evidence = evidence;
    return [diagnostic];
  });
  return diagnostics.length > 0 ? diagnostics : undefined;
}
