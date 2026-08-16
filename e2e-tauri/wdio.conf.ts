import { existsSync, mkdirSync } from "node:fs";
import { basename, resolve } from "node:path";

const resultsDir = resolve(process.env.ONECAD_TAURI_E2E_RESULTS ?? "e2e-tauri/results");

/**
 * The service documents a macOS `.app` bundle as a valid `appBinaryPath`, but its
 * launcher `spawn()`s that path verbatim — and a bundle is a DIRECTORY, so the
 * spawn fails with `EACCES` before any driver starts (`A service failed in the
 * 'onPrepare' hook`). That is why this lane had never executed anywhere. Resolve
 * the bundle to its executable ourselves; a plain binary path is passed through.
 */
function appExecutable(path: string): string {
  if (!path.endsWith(".app")) return path;
  const exe = resolve(path, "Contents/MacOS", basename(path, ".app"));
  if (!existsSync(exe)) {
    throw new Error(
      `Tauri bundle ${path} has no executable at ${exe} — build it with ` +
        `\`bun run tauri build --features tauri-e2e --config src-tauri/tauri.e2e.conf.json --bundles app\``,
    );
  }
  return exe;
}

const appPath = appExecutable(
  resolve(process.env.ONECAD_TAURI_E2E_APP ?? "src-tauri/target/release/bundle/macos/onecad.app"),
);

mkdirSync(resultsDir, { recursive: true });
process.env.ONECAD_LOG_DIR = resolve(resultsDir, "app-logs");
process.env.ONECAD_WORKER_LOG = "debug";

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: [resolve("e2e-tauri/specs/composition.e2e.ts")],
  maxInstances: 1,
  capabilities: [{ browserName: "tauri" }],
  services: [
    [
      "tauri",
      {
        appBinaryPath: appPath,
        driverProvider: "embedded",
        captureBackendLogs: true,
        captureFrontendLogs: true,
        backendLogLevel: "debug",
        frontendLogLevel: "debug",
        logDir: resolve(resultsDir, "wdio-service-logs"),
        commandTimeout: 60_000,
        startTimeout: 120_000,
        env: {
          ONECAD_LOG_DIR: resolve(resultsDir, "app-logs"),
          ONECAD_WORKER_LOG: "debug",
        },
      },
    ],
  ],
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { timeout: 180_000 },
  waitforTimeout: 20_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 0,
  logLevel: "info",
  outputDir: resultsDir,
  afterTest: async (_test, _context, result) => {
    if (!result.passed) {
      await browser.saveScreenshot(resolve(resultsDir, "failure.png"));
    }
  },
};
