export type ErrorCode =
  | "APP_NOT_RUNNING"
  | "WEBDRIVER_UNAVAILABLE"
  | "BRIDGE_WEDGED"
  | "WINDOW_NOT_FOUND"
  | "WINDOW_NOT_FOREGROUND"
  | "FOCUS_FAILED"
  | "ELEMENT_NOT_FOUND"
  | "ELEMENT_STALE"
  | "ELEMENT_MOVING"
  | "ELEMENT_OCCLUDED"
  | "POINT_OUTSIDE_WINDOW"
  | "CALIBRATION_FAILED"
  | "NATIVE_INPUT_PERMISSION_DENIED"
  | "SCREEN_CAPTURE_PERMISSION_DENIED"
  | "PORT_IN_USE"
  | "DEV_SERVER_PORT_IN_USE"
  | "APP_IDENTITY_MISMATCH"
  | "ACTION_TIMEOUT"
  | "SETTLE_TIMEOUT"
  | "STOP_INCOMPLETE"
  | "UNSUPPORTED_PLATFORM_CAPABILITY"
  | "BACKGROUND_CAPABILITY_UNAVAILABLE"
  | "INVALID_TARGET"
  | "HELPER_FAILED"
  | "INTERNAL";

export const REMEDIATION: Record<ErrorCode, string> = {
  APP_NOT_RUNNING:
    "No live session. Call session_start first ({mode:\"launch\", launch:\"dev\"} builds and runs `bun run tauri:agent`; {mode:\"attach\"} binds to an app already serving WebDriver on the configured port).",
  WEBDRIVER_UNAVAILABLE:
    "The app is not serving WebDriver. Rebuild/run it with the tauri-e2e feature (`bun run tauri:agent`) and confirm TAURI_WEBDRIVER_PORT matches webdriver.port in tauri-agent.config.json; check http://127.0.0.1:<port>/status responds.",
  BRIDGE_WEDGED:
    "The WebDriver bridge stopped answering. Call session_status; if it stays Reconnecting, run session_stop then session_start. Native screenshots (ui_screenshot) still work while the bridge is wedged.",
  WINDOW_NOT_FOUND:
    "No on-screen window for the app process. Confirm the app finished launching (window_list), that it is not minimised, and that app.windowLabel in tauri-agent.config.json matches the Tauri window label.",
  WINDOW_NOT_FOREGROUND:
    "The app window is behind another window. Call window_focus before input; if focus keeps failing, click the app in the Dock once and retry.",
  FOCUS_FAILED:
    "macOS refused to activate the app. Grant Accessibility to the terminal app running Claude Code: System Settings -> Privacy & Security -> Accessibility, then quit and reopen that terminal.",
  ELEMENT_NOT_FOUND:
    "No element matched the target. Take a fresh ui_snapshot and use a ref/testId from it; toolbar tools have no data-testid, so target them by {role:\"button\", name:\"<aria-label>\"}.",
  ELEMENT_STALE:
    "The element was re-rendered between snapshot and input, so its ref no longer identifies the same node. Re-run ui_snapshot and repeat the action with the new ref.",
  ELEMENT_MOVING:
    "The element is still animating (two rect samples 50ms apart disagreed). Call wait_for {kind:\"revision_stable\"} and retry.",
  ELEMENT_OCCLUDED:
    "Another element covers the target point (see details for the blocker). Dismiss the overlay/menu, scroll the target into the clear, or pass an offset that lands on a visible part of the element.",
  POINT_OUTSIDE_WINDOW:
    "The computed global point falls outside the app window. Scroll the element into view, resize the window, or re-run calibration via session_status after moving the window.",
  CALIBRATION_FAILED:
    "CSS-to-screen mapping could not be verified, so no input was sent. Ensure the window is fully on one display, unoccluded, at 100% zoom, then session_stop and session_start again.",
  NATIVE_INPUT_PERMISSION_DENIED:
    "Grant Accessibility to the terminal app running Claude Code: System Settings -> Privacy & Security -> Accessibility, enable the toggle for that app, then quit and reopen it (the grant is not picked up by a running process).",
  SCREEN_CAPTURE_PERMISSION_DENIED:
    "Grant Screen Recording to the terminal app running Claude Code: System Settings -> Privacy & Security -> Screen Recording, enable the toggle for that app, then quit and reopen it. Without it captures return wallpaper only and window names are hidden.",
  PORT_IN_USE:
    "The WebDriver port is held by another process. Either stop that process, or call session_start with {mode:\"attach\"} (or {reuseExisting:true}) to bind to the app that already owns it. The agent never kills a port owner it did not launch.",
  DEV_SERVER_PORT_IN_USE:
    "The Vite dev port (devServer.port, default 1420) is already taken — usually a `bun run dev` or `tauri dev` left running. Stop it, then retry session_start.",
  APP_IDENTITY_MISMATCH:
    "The process serving WebDriver is not the app from this checkout (see details.identity for what it reported: agentTesting, pid and the compile-time cargoManifestDir). No input was sent. Stop that process, or point session_start at the port your own app serves with {port:<n>} — the agent never drives an app it cannot identify.",
  ACTION_TIMEOUT:
    "The action exceeded its budget. Check session_status for a wedged bridge, look at observe_logs for a blocked regen, and retry with a larger timeoutMs once the app is idle.",
  SETTLE_TIMEOUT:
    "The UI never went quiet within settle.timeoutMs. The action was sent; verify the outcome with ui_snapshot or observe_logs. Long regens are expected — wait_for {kind:\"element_hidden\", testId:\"regen-busy\"} before asserting.",
  STOP_INCOMPLETE:
    "Processes survived session_stop. Inspect with `pgrep -fl 'target/debug/onecad|onecad-worker-'` and kill the leftovers manually before starting a new session; a surviving app will hold the WebDriver and dev ports.",
  UNSUPPORTED_PLATFORM_CAPABILITY:
    "This capability exists only in the macOS adapter (Phase 1). Run the agent on macOS, or use mode:\"webview\" tools, which do not need native input.",
  BACKGROUND_CAPABILITY_UNAVAILABLE:
    "This session runs under interaction:\"background\", which never moves the cursor, never activates the app and never posts an OS event. Nothing was sent. Either use a verb the background lane supports (see details.supportedInBackground), drive the native chrome with native_press / native_set_value / native_menu_invoke, or start a session with interaction:\"foreground\" for real CGEvent fidelity \u2014 which WILL take over the pointer and the frontmost application.",
  INVALID_TARGET:
    "The target object is malformed. Use exactly one of {ref}, {testId}, {role,name?}, {text}, {css}, {point:{x,y,space}} with space one of webview|window|global.",
  HELPER_FAILED:
    "The native input helper could not be built or crashed. Confirm `swiftc --version` works (full Xcode required), delete ~/.cache/tauri-agent to force a rebuild, and check stderr for the compiler output.",
  INTERNAL: "Unexpected agent failure. Re-run with TAURI_AGENT_LOG=debug and report the stderr trace with the actionId.",
};

export class AgentError extends Error {
  readonly code: ErrorCode;
  readonly remediation: string;
  readonly details?: Record<string, unknown>;

  constructor(
    code: ErrorCode,
    message: string,
    opts?: { remediation?: string; details?: Record<string, unknown>; cause?: unknown },
  ) {
    super(message, opts?.cause === undefined ? undefined : { cause: opts.cause });
    this.name = "AgentError";
    this.code = code;
    this.remediation = opts?.remediation ?? REMEDIATION[code];
    this.details = opts?.details;
  }
}

export function isAgentError(e: unknown): e is AgentError {
  return e instanceof AgentError;
}
