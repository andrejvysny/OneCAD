import { describe, expect, test } from "bun:test";
import { AgentError, REMEDIATION, isAgentError, type ErrorCode } from "../src/errors.ts";

const CODES = Object.keys(REMEDIATION) as ErrorCode[];

describe("errors", () => {
  test("every code has actionable remediation text", () => {
    expect(CODES.length).toBe(23);
    for (const code of CODES) {
      expect(REMEDIATION[code].length).toBeGreaterThan(40);
    }
  });

  test("permission codes name the exact macOS grant", () => {
    expect(REMEDIATION.NATIVE_INPUT_PERMISSION_DENIED).toContain("Privacy & Security -> Accessibility");
    expect(REMEDIATION.SCREEN_CAPTURE_PERMISSION_DENIED).toContain("Privacy & Security -> Screen Recording");
  });

  test("defaults remediation from the code", () => {
    const e = new AgentError("ELEMENT_STALE", "ref @e7 no longer matches");
    expect(e.code).toBe("ELEMENT_STALE");
    expect(e.remediation).toBe(REMEDIATION.ELEMENT_STALE);
    expect(e.message).toBe("ref @e7 no longer matches");
    expect(e.name).toBe("AgentError");
    expect(e instanceof Error).toBe(true);
  });

  test("carries overrides, details and cause", () => {
    const cause = new Error("boom");
    const e = new AgentError("HELPER_FAILED", "helper exited", {
      remediation: "custom",
      details: { exitCode: 3 },
      cause,
    });
    expect(e.remediation).toBe("custom");
    expect(e.details).toEqual({ exitCode: 3 });
    expect(e.cause).toBe(cause);
  });

  test("isAgentError discriminates", () => {
    expect(isAgentError(new AgentError("INTERNAL", "x"))).toBe(true);
    expect(isAgentError(new Error("x"))).toBe(false);
    expect(isAgentError("APP_NOT_RUNNING")).toBe(false);
  });
});
