/**
 * macOS native capture via /usr/sbin/screencapture, which photographs the real window
 * server output — the only evidence that survives a wedged WebView bridge.
 *
 * Without the Screen Recording grant screencapture either fails or writes a tiny
 * wallpaper-only image, so a non-zero exit or a sub-kilobyte file is reported as
 * SCREEN_CAPTURE_PERMISSION_DENIED rather than handed back as a picture of nothing.
 */
import { AgentError } from "../../errors.ts";
import type { Rect } from "../../geometry/types.ts";
import { log } from "../../log.ts";
import type { CaptureResult, NativeCapture } from "../adapter.ts";

const SCREENCAPTURE = "/usr/sbin/screencapture";
const SIPS = "/usr/bin/sips";
/** Anything smaller than this is a failed/blank capture, not a window. */
const MIN_CAPTURE_BYTES = 1024;
const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
/** Rounding a rect to whole points can move each ratio by a pixel or two; more is a mismatch. */
const ASPECT_TOLERANCE = 0.05;

export class MacCapture implements NativeCapture {
  /** `boundsPt` (the window's global-point bounds) turns the pixel size into a pixelScale. */
  async window(windowId: number, outPath: string, boundsPt?: Rect): Promise<CaptureResult> {
    await run([SCREENCAPTURE, "-x", "-o", "-l", String(windowId), outPath], outPath);
    // `-l` has been seen to answer with a DIFFERENT window of the same pid; the size is the
    // only evidence available here that the image is not the window that was asked for.
    return measure(outPath, boundsPt);
  }

  async region(rect: Rect, outPath: string): Promise<CaptureResult> {
    // screencapture -R takes integers; a fractional rect is silently re-interpreted, which
    // then makes every pixelScale derived from it wrong.
    const r = roundRect(rect);
    await run([SCREENCAPTURE, "-x", "-R", `${r.x},${r.y},${r.width},${r.height}`, outPath], outPath);
    return measure(outPath, r);
  }

  /** `boundsPt` is the display's point size when the caller knows it; otherwise pixelScale is 1. */
  async screen(outPath: string, boundsPt?: Rect): Promise<CaptureResult> {
    await run([SCREENCAPTURE, "-x", outPath], outPath);
    return measure(outPath, boundsPt);
  }

  async preview(srcPath: string, outPath: string, maxPx: number): Promise<void> {
    const proc = Bun.spawn([SIPS, "-Z", String(Math.round(maxPx)), srcPath, "--out", outPath], {
      stdin: "ignore",
      stdout: "pipe",
      stderr: "pipe",
    });
    const [code, stderr] = await Promise.all([proc.exited, new Response(proc.stderr).text()]);
    if (code !== 0) {
      throw new AgentError("INTERNAL", `sips exited ${code} downscaling ${srcPath}`, {
        details: { srcPath, outPath, maxPx, stderr: stderr.trim().slice(0, 400) },
      });
    }
  }
}

async function run(cmd: string[], outPath: string): Promise<void> {
  const proc = Bun.spawn(cmd, { stdin: "ignore", stdout: "pipe", stderr: "pipe" });
  const [code, stderr] = await Promise.all([proc.exited, new Response(proc.stderr).text()]);
  const text = stderr.trim();
  // A window that closed (or a stale CGWindowID) is not a permission problem, and telling the
  // caller to open System Settings would send them to fix something that is not broken.
  if (/could not create image from window/i.test(text)) {
    throw new AgentError("WINDOW_NOT_FOUND", "screencapture could not photograph that window id", {
      details: { outPath, exitCode: code, stderr: text.slice(0, 400) },
    });
  }
  if (code !== 0) throw captureDenied(outPath, { exitCode: code, stderr: text.slice(0, 400) });
  if (text) log.debug("screencapture stderr", { stderr: text.slice(0, 400) });
}

async function measure(outPath: string, pointSize?: { width: number; height: number }): Promise<CaptureResult> {
  const file = Bun.file(outPath);
  const size = file.size;
  if (!(await file.exists()) || size < MIN_CAPTURE_BYTES) {
    throw captureDenied(outPath, { bytes: size });
  }
  const head = new Uint8Array(await file.slice(0, 33).arrayBuffer());
  const px = readPngSize(head);
  return { ...px, ...pixelScaleOf(px, pointSize) };
}

function roundRect(r: Rect): Rect {
  return { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) };
}

/**
 * Backing scale of a capture, plus the disagreement that says the image is not what was asked
 * for: one image of a `pointSize` area has ONE scale, so a width ratio that does not match the
 * height ratio means a different window (or a different display) was photographed.
 */
export function pixelScaleOf(
  px: { width: number; height: number },
  pointSize?: { width: number; height: number },
): { pixelScale: number; warning?: string } {
  if (pointSize === undefined || pointSize.width <= 0 || pointSize.height <= 0) return { pixelScale: 1 };
  const sx = px.width / pointSize.width;
  const sy = px.height / pointSize.height;
  if (Math.abs(sy - sx) > ASPECT_TOLERANCE * sx) {
    return {
      pixelScale: sx,
      warning:
        `captured ${px.width}x${px.height}px does not match the requested ` +
        `${pointSize.width}x${pointSize.height}pt (scale ${sx.toFixed(3)} wide vs ${sy.toFixed(3)} tall)`,
    };
  }
  return { pixelScale: sx };
}

function captureDenied(outPath: string, details: Record<string, unknown>): AgentError {
  return new AgentError(
    "SCREEN_CAPTURE_PERMISSION_DENIED",
    "screencapture produced no usable image; Screen Recording is most likely not granted",
    { details: { outPath, ...details } },
  );
}

/**
 * Reads width/height straight out of the PNG IHDR chunk — the first chunk of every PNG —
 * so no image dependency is needed just to size a screenshot.
 */
export function readPngSize(bytes: Uint8Array): { width: number; height: number } {
  if (bytes.length < 24) throw new AgentError("INTERNAL", `PNG header truncated (${bytes.length} bytes)`);
  for (let i = 0; i < PNG_SIGNATURE.length; i++) {
    if (bytes[i] !== PNG_SIGNATURE[i]) throw new AgentError("INTERNAL", "not a PNG file (bad signature)");
  }
  if (String.fromCharCode(bytes[12]!, bytes[13]!, bytes[14]!, bytes[15]!) !== "IHDR") {
    throw new AgentError("INTERNAL", "PNG first chunk is not IHDR");
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = view.getUint32(16, false);
  const height = view.getUint32(20, false);
  if (width <= 0 || height <= 0) throw new AgentError("INTERNAL", `PNG IHDR has degenerate size ${width}x${height}`);
  return { width, height };
}
