import type { ErrorCode } from "../errors.ts";
import { AgentError, REMEDIATION, isAgentError } from "../errors.ts";
import type { Pt } from "../geometry/types.ts";

export type Mode = "real_user" | "webview" | "diagnostic";

export interface ActionResult {
  actionId: string;
  status: "ok" | "warning" | "error";
  mode: Mode;
  backend: "cgevent" | "webdriver" | "js" | "none";
  windowId?: number;
  target?: { ref?: string; role?: string; name?: string; testId?: string };
  resolvedPoint?: { global: Pt; css: Pt };
  state?: { beforeRevision: number; afterRevision: number; settled: boolean };
  effects?: { consoleErrors: number; logErrors: number; newWindows: number; windowMoved: boolean };
  screenshot?: {
    path: string;
    previewPath: string;
    width: number;
    height: number;
    pixelScale: number;
    captureMode: string;
  };
  warnings: string[];
  error?: { code: ErrorCode; message: string; remediation: string; details?: Record<string, unknown> };
  timingsMs: { resolve: number; input: number; settle: number; capture: number };
  data?: unknown;
}

export function emptyTimings(): ActionResult["timingsMs"] {
  return { resolve: 0, input: 0, settle: 0, capture: 0 };
}

export function okResult(actionId: string, mode: Mode, patch: Partial<ActionResult> = {}): ActionResult {
  return {
    actionId,
    status: "ok",
    mode,
    backend: "none",
    warnings: [],
    timingsMs: emptyTimings(),
    ...patch,
  };
}

export function errorResult(actionId: string, mode: Mode, cause: unknown): ActionResult {
  const err = toAgentError(cause);
  return {
    actionId,
    status: "error",
    mode,
    backend: "none",
    warnings: [],
    timingsMs: emptyTimings(),
    error: {
      code: err.code,
      message: err.message,
      remediation: err.remediation,
      ...(err.details === undefined ? {} : { details: err.details }),
    },
  };
}

export function toAgentError(cause: unknown): AgentError {
  if (isAgentError(cause)) return cause;
  const message = cause instanceof Error ? cause.message : String(cause);
  return new AgentError("INTERNAL", message, { remediation: REMEDIATION.INTERNAL, cause });
}
