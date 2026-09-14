/**
 * Public session types. They live apart from `orchestrator.ts` so the calibration and
 * teardown modules can reference the session's bridge surface without importing the class.
 */
import type { Bridge } from "../semantic/webdriver.ts";
import type { SnapNode } from "../semantic/snapshot.ts";
import type { PlatformAdapter, Permissions } from "../platform/adapter.ts";
import type { CalibrationReport } from "./calibrate.ts";
import type { FetchFn, Spawner } from "./launch.ts";
import type { KillFn, Runner } from "./procs.ts";

export type SessionState =
  | "Idle"
  | "Launching"
  | "WaitingForBridge"
  | "WaitingForWindow"
  | "Ready"
  | "Reconnecting"
  | "Stopping"
  | "Failed";

export type RefStore = Map<string, SnapNode>;

/** The bridge surface the session and the tools use; a `Pick` so tests can supply a fake. */
export type SessionBridge = Pick<Bridge, "execute" | "invoke" | "windowGeom" | "close"> & {
  readonly wedged: boolean;
};

export interface StartOptions {
  mode: "launch" | "attach";
  launch?: "dev" | "bundled";
  port?: number;
  reuseExisting?: boolean;
  env?: Record<string, string>;
  /** Proceed without the Screen Recording grant; screenshots then show wallpaper only. */
  allowDegradedCapture?: boolean;
}

export interface StopOptions {
  killApp?: boolean;
}

export interface SessionStatus {
  sessionId: string;
  state: SessionState;
  ready: boolean;
  pid?: number;
  windowId?: number;
  webdriverPort: number;
  devServerPort: number;
  bridgeConnected: boolean;
  platformReady: boolean;
  calibrated: boolean;
  artifactsDir: string;
  sessionEpoch: number;
  launched: boolean;
  wheelLinesPerNotch: number;
  permissions?: Permissions;
  calibration?: CalibrationReport;
  failure?: { code: string; message: string };
  /** Measured result of the last stop: what survived the sweep and whether the ports are free. */
  teardown?: TeardownReport;
}

export interface TeardownReport {
  survivors: Array<{ pid: number; command: string }>;
  portsFree: Record<string, boolean>;
  launched: boolean;
}

export interface SessionDeps {
  runner?: Runner;
  platform?: () => PlatformAdapter;
  bridgeFactory?: (port: number) => Promise<SessionBridge>;
  fetch?: FetchFn;
  spawn?: Spawner;
  kill?: KillFn;
  sleep?: (ms: number) => Promise<void>;
}

