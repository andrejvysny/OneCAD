#!/usr/bin/env node
/*
 * VP-HARDENING evidence recorder (PLAN.md D12, T0).
 *
 * Runs a gate command, tees its stdout+stderr to a committed-ignored log file
 * under runs/<run-id>/logs/<lane>.log, and writes/merges two COMMITTED JSON
 * summaries (manifest.json, tests.json) that the verifier can check from a
 * clean checkout without the raw logs being present.
 *
 * Usage:
 *   node scripts/record-viewport-run.mjs <run-id> <lane> [--case ID,ID,...] \
 *     [--cwd <dir>] -- <command...>
 *
 * Exit code is the exit code of the recorded command (spawn/setup failures
 * exit 1 before the command runs).
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve, relative, sep } from "node:path";

const root = resolve(import.meta.dirname, "..");

function usageError(message) {
  process.stderr.write(`record-viewport-run: ${message}\n`);
  process.exit(1);
}

function parseArgs(argv) {
  const sepIdx = argv.indexOf("--");
  if (sepIdx === -1) usageError("missing `--` before the command");
  const head = argv.slice(0, sepIdx);
  const command = argv.slice(sepIdx + 1);
  if (command.length === 0) usageError("empty command after `--`");

  const positional = [];
  let cases = [];
  let cwd = root;
  for (let i = 0; i < head.length; i++) {
    const arg = head[i];
    if (arg === "--case") {
      cases = (head[++i] ?? "").split(",").map((s) => s.trim()).filter(Boolean);
    } else if (arg === "--cwd") {
      cwd = resolve(root, head[++i] ?? ".");
    } else if (!arg.startsWith("--")) {
      positional.push(arg);
    } else {
      usageError(`unknown flag: ${arg}`);
    }
  }
  const [runId, lane] = positional;
  if (!runId || !lane) usageError("usage: <run-id> <lane> [--case ID,...] [--cwd dir] -- <command...>");
  return { runId, lane, cases, cwd, command };
}

function sha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

function repoRelative(p) {
  return relative(root, p).split(sep).join("/");
}

function toolVersion(cmd) {
  const res = spawnSync(cmd, ["--version"], { encoding: "utf8" });
  if (res.error || res.status !== 0) return null;
  const line = (res.stdout || res.stderr || "").split("\n")[0].trim();
  return line || null;
}

/** Strip any literal occurrence of the caller's home dir from a string. */
function stripHome(s) {
  const home = process.env.HOME;
  if (!home) return s;
  return s.split(home).join("~");
}

function gitDirtyDiffSha256() {
  const res = spawnSync(
    "git",
    ["diff", "--", ".", ":!TODO.md", ":!HANDOFF.md", ":!CURRENT_STATE.md"],
    { cwd: root, encoding: "utf8" },
  );
  const diff = res.status === 0 ? res.stdout : "";
  return sha256(Buffer.from(diff, "utf8"));
}

function readJson(path, fallback) {
  if (!existsSync(path)) return fallback;
  return JSON.parse(readFileSync(path, "utf8"));
}

const { runId, lane, cases, cwd, command } = parseArgs(process.argv.slice(2));

const runDir = resolve(root, "docs/qa/viewport-hardening/runs", runId);
const logsDir = resolve(runDir, "logs");
mkdirSync(logsDir, { recursive: true });
const logPath = resolve(logsDir, `${lane}.log`);

const startedAt = new Date().toISOString();

// spawnSync doesn't tee natively; run with pipes and forward chunks to both
// the terminal and the log file as they arrive, so a long-running command
// still shows live progress.
const chunks = [];
const child = spawnSync(command[0], command.slice(1), {
  cwd,
  shell: false,
  env: process.env,
  stdio: ["inherit", "pipe", "pipe"],
  maxBuffer: 1024 * 1024 * 1024,
});

// spawnSync buffers stdout/stderr separately; interleave isn't preserved,
// but both streams are captured in full and both are echoed to the terminal.
if (child.stdout) {
  process.stdout.write(child.stdout);
  chunks.push(child.stdout);
}
if (child.stderr) {
  process.stderr.write(child.stderr);
  chunks.push(child.stderr);
}
if (child.error) {
  process.stderr.write(`record-viewport-run: spawn failed: ${child.error.message}\n`);
}

const endedAt = new Date().toISOString();
const exit = child.status ?? 1;

const logContent = chunks.map((b) => b.toString("utf8")).join("");
writeFileSync(logPath, logContent);
const logSha256 = sha256(Buffer.from(logContent, "utf8"));
const logLines = logContent.split("\n");
const tailLines = logLines.slice(-31).filter((_, i, arr) => !(i === arr.length - 1 && arr[arr.length - 1] === ""));

// --- manifest.json -------------------------------------------------------

const manifestPath = resolve(runDir, "manifest.json");
const manifest = readJson(manifestPath, {
  version: 1,
  runId,
  createdAt: startedAt,
  commit: null,
  branch: null,
  dirtyDiffSha256: null,
  tools: {},
  worker: {},
  lanes: {},
});

const commitRes = spawnSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" });
const branchRes = spawnSync("git", ["rev-parse", "--abbrev-ref", "HEAD"], { cwd: root, encoding: "utf8" });
manifest.commit = commitRes.status === 0 ? commitRes.stdout.trim() : null;
manifest.branch = branchRes.status === 0 ? branchRes.stdout.trim() : null;
manifest.dirtyDiffSha256 = gitDirtyDiffSha256();

manifest.tools = {
  node: toolVersion("node"),
  bun: toolVersion("bun"),
  rustc: toolVersion("rustc"),
  cargo: toolVersion("cargo"),
  cmake: toolVersion("cmake"),
};

const workerPath = resolve(root, "worker/build/onecad-worker");
manifest.worker = {
  path: "worker/build/onecad-worker",
  sha256: existsSync(workerPath) ? sha256(readFileSync(workerPath)) : null,
};

const envKeys = ["ONECAD_REQUIRE_WORKER", "ONECAD_WORKER_PATH", "E2E_PORT", "RUST_LOG"];
const pickedEnv = {};
for (const key of envKeys) {
  if (process.env[key] !== undefined) pickedEnv[key] = stripHome(process.env[key]);
}

manifest.lanes[lane] = {
  command: command.map(stripHome),
  cwd: repoRelative(cwd),
  env: pickedEnv,
  startedAt,
  endedAt,
  durationMs: Date.parse(endedAt) - Date.parse(startedAt),
  exit,
  logPath: repoRelative(logPath),
  logSha256,
  tailLines,
};

writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n");

// --- tests.json ------------------------------------------------------------

if (cases.length > 0) {
  const testsPath = resolve(runDir, "tests.json");
  const tests = readJson(testsPath, { version: 1, runId, cases: {} });
  const summaryPattern = /Tests: |Test Files |tests passed|test result:|\d+% tests passed|passed \/|failed/;
  const summaryLine = logLines.find((l) => summaryPattern.test(l)) ?? null;
  for (const caseId of cases) {
    tests.cases[caseId] = {
      lane,
      result: exit === 0 ? "pass" : "fail",
      exit,
      logSha256,
      summary: summaryLine,
    };
  }
  writeFileSync(testsPath, JSON.stringify(tests, null, 2) + "\n");
}

process.exit(exit);
