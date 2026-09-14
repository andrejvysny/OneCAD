import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { AgentError } from "../src/errors.ts";
import { measure, pixelScaleOf, readPngSize } from "../src/platform/macos/capture.ts";

const SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

function pngHeader(width: number, height: number, opts?: { signature?: number[]; chunk?: string }): Uint8Array {
  const bytes = new Uint8Array(33);
  bytes.set(opts?.signature ?? SIGNATURE, 0);
  const view = new DataView(bytes.buffer);
  view.setUint32(8, 13, false);
  const chunk = opts?.chunk ?? "IHDR";
  for (let i = 0; i < 4; i++) bytes[12 + i] = chunk.charCodeAt(i);
  view.setUint32(16, width, false);
  view.setUint32(20, height, false);
  return bytes;
}

describe("readPngSize", () => {
  test("reads width and height from the IHDR chunk", () => {
    expect(readPngSize(pngHeader(2880, 1800))).toEqual({ width: 2880, height: 1800 });
  });

  test("handles dimensions above 16 bits", () => {
    expect(readPngSize(pngHeader(6016, 3384))).toEqual({ width: 6016, height: 3384 });
  });

  test("reads from a view with a non-zero byteOffset", () => {
    const padded = new Uint8Array(40);
    padded.set(pngHeader(1440, 900), 7);
    expect(readPngSize(padded.subarray(7))).toEqual({ width: 1440, height: 900 });
  });

  test("rejects a truncated header", () => {
    expect(() => readPngSize(new Uint8Array(12))).toThrow(AgentError);
  });

  test("rejects a bad signature", () => {
    const bad = pngHeader(10, 10, { signature: [0x89, 0x50, 0x4e, 0x46, 0x0d, 0x0a, 0x1a, 0x0a] });
    expect(() => readPngSize(bad)).toThrow(/bad signature/);
  });

  test("rejects a first chunk that is not IHDR", () => {
    expect(() => readPngSize(pngHeader(10, 10, { chunk: "IDAT" }))).toThrow(/not IHDR/);
  });

  test("rejects a degenerate size", () => {
    expect(() => readPngSize(pngHeader(0, 0))).toThrow(/degenerate/);
  });
});

describe("pixelScaleOf", () => {
  test("is 1 when the caller cannot say what the capture should have covered", () => {
    expect(pixelScaleOf({ width: 1440, height: 900 })).toEqual({ pixelScale: 1 });
    expect(pixelScaleOf({ width: 1440, height: 900 }, { width: 0, height: 0 })).toEqual({ pixelScale: 1 });
  });

  test("derives the backing scale from the requested point size", () => {
    expect(pixelScaleOf({ width: 2880, height: 1800 }, { width: 1440, height: 900 })).toEqual({ pixelScale: 2 });
  });

  test("tolerates a rounding-sized disagreement between the two ratios", () => {
    const out = pixelScaleOf({ width: 2880, height: 1798 }, { width: 1440, height: 900 });
    expect(out.pixelScale).toBe(2);
    expect(out.warning).toBeUndefined();
  });

  test("warns when the image is not the window that was asked for", () => {
    // Two same-pid windows returned one image: the height ratio gives the lie away.
    const out = pixelScaleOf({ width: 2880, height: 1200 }, { width: 1440, height: 900 });
    expect(out.pixelScale).toBe(2);
    expect(out.warning).toMatch(/1440x900/);
  });
});

/**
 * The size heuristic can only say an image LOOKS degraded. Which error that is depends on the
 * LIVE grant: a preflight reading taken minutes ago may have been revoked, and sending a user
 * to a System Settings toggle that is already correct hides the real failure.
 */
describe("measure — degraded capture verdict", () => {
  const files: string[] = [];

  afterEach(() => {
    for (const f of files.splice(0)) rmSync(f, { recursive: true, force: true });
  });

  function tinyFile(): string {
    const dir = mkdtempSync(join(tmpdir(), "tauri-agent-capture-"));
    files.push(dir);
    const path = join(dir, "shot.png");
    writeFileSync(path, Buffer.alloc(64));
    return path;
  }

  function fullPng(): string {
    const dir = mkdtempSync(join(tmpdir(), "tauri-agent-capture-"));
    files.push(dir);
    const path = join(dir, "shot.png");
    const body = new Uint8Array(4096);
    body.set(pngHeader(2880, 1800), 0);
    writeFileSync(path, body);
    return path;
  }

  test("a sub-kilobyte image with the grant present is NOT the permission error", async () => {
    const err = (await measure(tinyFile(), undefined, async () => true).catch((e: unknown) => e)) as AgentError;
    expect(err).toBeInstanceOf(AgentError);
    expect(err.code).not.toBe("SCREEN_CAPTURE_PERMISSION_DENIED");
    expect(err.message).toContain("IS granted");
    // The user must not be sent to fix a setting that is already correct.
    expect(err.remediation).not.toContain("System Settings");
    expect(err.details?.screenRecording).toBe(true);
  });

  test("a sub-kilobyte image with the grant genuinely gone is the permission error", async () => {
    const err = (await measure(tinyFile(), undefined, async () => false).catch((e: unknown) => e)) as AgentError;
    expect(err.code).toBe("SCREEN_CAPTURE_PERMISSION_DENIED");
    expect(err.details?.screenRecording).toBe(false);
  });

  test("an unanswerable grant question stays on the conservative verdict", async () => {
    const err = (await measure(tinyFile(), undefined, async () => undefined).catch((e: unknown) => e)) as AgentError;
    expect(err.code).toBe("SCREEN_CAPTURE_PERMISSION_DENIED");
    expect(err.message).toContain("most likely");
  });

  test("an image whose aspect disagrees with the bounds is returned, but not as evidence", async () => {
    const out = await measure(fullPng(), { width: 1440, height: 900 }, async () => true);
    expect(out.pixelScale).toBe(2);
    expect(out.authoritative).toBeUndefined();

    const bad = await measure(fullPng(), { width: 1440, height: 1200 }, async () => true);
    expect(bad.authoritative).toBe(false);
    expect(bad.reason).toBe("bounds-mismatch");
    expect(bad.warning).toBeDefined();
  });
});
