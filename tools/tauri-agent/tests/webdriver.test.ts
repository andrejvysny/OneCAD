import { describe, expect, test } from "bun:test";
import { isAgentError } from "../src/errors.ts";
import { INIT_TIMEOUT_MS, pollPluginInit, validateGeom } from "../src/semantic/webdriver.ts";
import type { InitPollDeps, WindowGeomReading } from "../src/semantic/webdriver.ts";

function reading(over: Partial<WindowGeomReading> = {}): WindowGeomReading {
  return {
    innerPositionPx: { x: 400, y: 200 },
    innerSizePx: { width: 1600, height: 1000 },
    scaleFactor: 2,
    focused: true,
    dpr: 2,
    vvScale: 1,
    innerWidth: 800,
    innerHeight: 500,
    ...over,
  };
}

function codeOf(fn: () => unknown): string {
  try {
    fn();
  } catch (e) {
    return isAgentError(e) ? e.code : `not-an-AgentError:${String(e)}`;
  }
  return "no-throw";
}

describe("validateGeom", () => {
  test("a consistent reading passes through unchanged", () => {
    const raw = reading();
    expect(validateGeom(raw)).toBe(raw);
    // A tenth of a percent of scale-factor noise is not a calibration failure.
    expect(validateGeom(reading({ dpr: 2.005 }))).toBeDefined();
    expect(validateGeom(reading({ innerWidth: 799.5 }))).toBeDefined();
  });

  test("a pinch-zoomed visual viewport refuses: every css px would be mis-scaled", () => {
    expect(codeOf(() => validateGeom(reading({ vvScale: 1.5 })))).toBe("CALIBRATION_FAILED");
    expect(codeOf(() => validateGeom(reading({ vvScale: 0.9 })))).toBe("CALIBRATION_FAILED");
  });

  test("devicePixelRatio disagreeing with the tauri scale factor refuses", () => {
    expect(codeOf(() => validateGeom(reading({ dpr: 1 })))).toBe("CALIBRATION_FAILED");
    expect(codeOf(() => validateGeom(reading({ dpr: 2.02 })))).toBe("CALIBRATION_FAILED");
  });

  test("an inner size that does not divide down to innerWidth/Height refuses", () => {
    expect(codeOf(() => validateGeom(reading({ innerWidth: 700 })))).toBe("CALIBRATION_FAILED");
    expect(codeOf(() => validateGeom(reading({ innerHeight: 480 })))).toBe("CALIBRATION_FAILED");
  });

  test("a missing or unusable field is a transport failure, not a calibration one", () => {
    expect(codeOf(() => validateGeom(null as unknown as WindowGeomReading))).toBe("WEBDRIVER_UNAVAILABLE");
    expect(codeOf(() => validateGeom(reading({ scaleFactor: 0 })))).toBe("WEBDRIVER_UNAVAILABLE");
    expect(codeOf(() => validateGeom(reading({ dpr: null as unknown as number })))).toBe("WEBDRIVER_UNAVAILABLE");
    expect(codeOf(() => validateGeom(reading({ innerHeight: Number.NaN })))).toBe("WEBDRIVER_UNAVAILABLE");
  });

  test("the remediation does not blame async execute, which this driver does await", () => {
    try {
      validateGeom(reading({ vvScale: 2 }));
      throw new Error("expected a refusal");
    } catch (e) {
      if (!isAgentError(e)) throw e;
      expect(e.remediation).not.toMatch(/promise|BiDi/i);
      expect(e.remediation.length).toBeGreaterThan(20);
    }
  });
});

describe("pollPluginInit", () => {
  function deps(over: Partial<InitPollDeps> & Pick<InitPollDeps, "probe">): InitPollDeps {
    return {
      transient: () => true,
      sleep: async () => {},
      now: () => 0,
      ...over,
    };
  }

  test("an app built without the wdio plugin answers once and is believed", async () => {
    const sleeps: number[] = [];
    let calls = 0;
    const state = await pollPluginInit(
      deps({
        probe: async () => {
          calls += 1;
          return "absent";
        },
        sleep: async (ms) => {
          sleeps.push(ms);
        },
      }),
    );
    expect(state).toBe("absent");
    expect(calls).toBe(1);
    expect(sleeps).toEqual([]);
  });

  test("ready ends the poll immediately", async () => {
    let calls = 0;
    const state = await pollPluginInit(
      deps({
        probe: async () => {
          calls += 1;
          return "ready";
        },
      }),
    );
    expect(state).toBe("ready");
    expect(calls).toBe(1);
  });

  test("a transient failure is retried until the deadline, then reported as a timeout", async () => {
    let t = 0;
    let calls = 0;
    const state = await pollPluginInit({
      probe: async () => {
        calls += 1;
        throw new Error("budget");
      },
      transient: () => true,
      sleep: async (ms) => {
        t += ms;
      },
      now: () => t,
    });
    expect(state).toBe("timeout");
    expect(calls).toBeGreaterThan(1);
    expect(t).toBeGreaterThanOrEqual(INIT_TIMEOUT_MS);
  });

  test("a failure that is not transient propagates instead of burning the budget", async () => {
    const boom = new Error("no such session");
    await expect(
      pollPluginInit(
        deps({
          probe: async () => {
            throw boom;
          },
          transient: () => false,
        }),
      ),
    ).rejects.toThrow("no such session");
  });
});
