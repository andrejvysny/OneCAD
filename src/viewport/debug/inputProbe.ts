/*
 * inputProbe — TEMPORARY diagnostic for `?inputprobe`. DELETE once the trackpad
 * navigation constants are calibrated.
 *
 * Wheel and WebKit gesture behaviour differs between Safari, Chromium and an
 * embedded WKWebView, and the differences that matter here are undocumented.
 * This records the raw event stream inside the real Tauri shell so the
 * navigation tunables are measured rather than guessed.
 *
 * It answers, specifically:
 *   1. Does WKWebView deliver gesturestart/gesturechange/gestureend at all?
 *   2. Does it ALSO set ctrlKey on wheel for a pinch (⇒ double-zoom risk)?
 *   3. Does holding shift swap deltaX/deltaY for a two-finger trackpad scroll?
 *      (Documented for mouse wheels; unverified for a trackpad's 2D scroll.)
 *   4. Does gesturechange carry usable clientX/clientY (⇒ zoom-to-cursor)?
 *   5. Real delta magnitudes and cadence, for the classifier + gain constants.
 *   6. How long a ctrl-wheel tail runs after gestureend.
 *
 * It deliberately does NOT preventDefault: the point is to observe native
 * behaviour, including any page magnification, not to mask it.
 */

interface ProbeRecord {
  type: string;
  t: number;
  /** ms since the previous record — the cadence signal. */
  dt: number;
  deltaX?: number;
  deltaY?: number;
  deltaMode?: number;
  scale?: number;
  rotation?: number;
  ctrlKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
  clientX?: number;
  clientY?: number;
  cancelable: boolean;
  defaultPrevented: boolean;
  target: string;
}

interface GestureLike {
  scale?: number;
  rotation?: number;
  clientX?: number;
  clientY?: number;
}

const MAX_RECORDS = 4000;

declare global {
  interface Window {
    __inputLog?: ProbeRecord[];
    __inputProbeDump?: () => string;
    __inputProbeClear?: () => void;
  }
}

/**
 * Attach the probe to `el`. Returns a teardown function.
 * Only called when the `?inputprobe` flag is present.
 */
export function installInputProbe(el: HTMLElement): () => void {
  const log: ProbeRecord[] = [];
  window.__inputLog = log;
  window.__inputProbeDump = () => JSON.stringify(log, null, 2);
  window.__inputProbeClear = () => {
    log.length = 0;
    render();
  };

  const overlay = document.createElement("div");
  overlay.dataset.inputProbe = "1";
  Object.assign(overlay.style, {
    position: "fixed",
    top: "8px",
    left: "8px",
    zIndex: "9999",
    maxWidth: "min(560px, 46vw)",
    maxHeight: "70vh",
    overflow: "auto",
    padding: "8px 10px",
    borderRadius: "6px",
    background: "rgba(0,0,0,0.82)",
    color: "#e8e8e8",
    font: "11px/1.45 ui-monospace, monospace",
    whiteSpace: "pre",
    pointerEvents: "none",
  } satisfies Partial<CSSStyleDeclaration>);
  document.body.appendChild(overlay);

  let last = 0;
  let raf = 0;

  const push = (r: Omit<ProbeRecord, "dt">): void => {
    const dt = last ? Math.round((r.t - last) * 10) / 10 : 0;
    last = r.t;
    log.push({ ...r, dt });
    if (log.length > MAX_RECORDS) log.shift();
    if (!raf) raf = requestAnimationFrame(flush);
  };

  const flush = (): void => {
    raf = 0;
    render();
  };

  const onWheel = (e: WheelEvent): void => {
    push({
      type: "wheel",
      t: e.timeStamp,
      deltaX: e.deltaX,
      deltaY: e.deltaY,
      deltaMode: e.deltaMode,
      ctrlKey: e.ctrlKey,
      shiftKey: e.shiftKey,
      altKey: e.altKey,
      metaKey: e.metaKey,
      clientX: e.clientX,
      clientY: e.clientY,
      cancelable: e.cancelable,
      defaultPrevented: e.defaultPrevented,
      target: describeTarget(e.target),
    });
  };

  const onGesture = (e: Event): void => {
    const g = e as unknown as GestureLike;
    push({
      type: e.type,
      t: e.timeStamp,
      scale: g.scale,
      rotation: g.rotation,
      clientX: g.clientX,
      clientY: g.clientY,
      cancelable: e.cancelable,
      defaultPrevented: e.defaultPrevented,
      target: describeTarget(e.target),
    });
  };

  el.addEventListener("wheel", onWheel, { passive: true });
  el.addEventListener("gesturestart", onGesture, { passive: true });
  el.addEventListener("gesturechange", onGesture, { passive: true });
  el.addEventListener("gestureend", onGesture, { passive: true });

  function render(): void {
    overlay.textContent = [summarise(log), "", ...log.slice(-24).map(fmt)].join("\n");
  }
  render();

  return () => {
    el.removeEventListener("wheel", onWheel);
    el.removeEventListener("gesturestart", onGesture);
    el.removeEventListener("gesturechange", onGesture);
    el.removeEventListener("gestureend", onGesture);
    if (raf) cancelAnimationFrame(raf);
    overlay.remove();
    delete window.__inputLog;
    delete window.__inputProbeDump;
    delete window.__inputProbeClear;
  };
}

function describeTarget(t: EventTarget | null): string {
  const el = t as HTMLElement | null;
  if (!el || !el.tagName) return "?";
  return el.dataset?.testid ? `${el.tagName}[${el.dataset.testid}]` : el.tagName;
}

function n(v: number | undefined, digits = 2): string {
  return v === undefined ? "-" : v.toFixed(digits);
}

function fmt(r: ProbeRecord): string {
  const mods =
    (r.ctrlKey ? "C" : "") +
    (r.shiftKey ? "S" : "") +
    (r.altKey ? "A" : "") +
    (r.metaKey ? "M" : "");
  if (r.type === "wheel") {
    return `${r.type.padEnd(13)} dx=${n(r.deltaX).padStart(9)} dy=${n(r.deltaY).padStart(9)} mode=${r.deltaMode} ${mods.padEnd(4)} @${n(r.clientX, 0)},${n(r.clientY, 0)} dt=${r.dt}`;
  }
  return `${r.type.padEnd(13)} scale=${n(r.scale, 4).padStart(8)} rot=${n(r.rotation, 1).padStart(6)} @${n(r.clientX, 0)},${n(r.clientY, 0)} canc=${r.cancelable} dt=${r.dt}`;
}

/** The six answers, computed live so the findings are readable on the overlay. */
function summarise(log: ProbeRecord[]): string {
  const wheels = log.filter((r) => r.type === "wheel");
  const gestures = log.filter((r) => r.type.startsWith("gesture"));
  const ctrlWheels = wheels.filter((r) => r.ctrlKey);
  const shiftWheels = wheels.filter((r) => r.shiftKey);

  const shiftVertical = shiftWheels.filter((r) => (r.deltaY ?? 0) !== 0).length;
  const shiftHorizontal = shiftWheels.filter((r) => (r.deltaX ?? 0) !== 0).length;

  const gestureChanges = log.filter((r) => r.type === "gesturechange");
  const gestureHasCoords = gestureChanges.some((r) => (r.clientX ?? 0) !== 0 || (r.clientY ?? 0) !== 0);

  // How long ctrl-wheels keep arriving after the last gestureend.
  const lastEnd = [...log].reverse().find((r) => r.type === "gestureend");
  const tail = lastEnd
    ? ctrlWheels.filter((r) => r.t > lastEnd.t).map((r) => Math.round(r.t - lastEnd.t))
    : [];

  const fractional = wheels.filter(
    (r) => !Number.isInteger(r.deltaY ?? 0) || !Number.isInteger(r.deltaX ?? 0),
  ).length;
  const gaps = wheels.map((r) => r.dt).filter((d) => d > 0);
  const modes = [...new Set(wheels.map((r) => r.deltaMode))].join(",");
  const maxAbsY = wheels.reduce((m, r) => Math.max(m, Math.abs(r.deltaY ?? 0)), 0);

  return [
    `records=${log.length}  wheel=${wheels.length}  gesture=${gestures.length}`,
    `Q1 gesture events fire ......... ${gestures.length > 0 ? "YES" : "NO"}`,
    `Q2 ctrlKey wheel seen .......... ${ctrlWheels.length > 0 ? `YES (${ctrlWheels.length})` : "NO"}`,
    `Q3 shift+wheel axes ............ dy!=0:${shiftVertical}  dx!=0:${shiftHorizontal}` +
      (shiftWheels.length && shiftVertical === 0 && shiftHorizontal > 0 ? "  << AXIS SWAP" : ""),
    `Q4 gesturechange has coords .... ${gestureChanges.length === 0 ? "n/a" : gestureHasCoords ? "YES" : "NO"}`,
    `Q5 deltaMode(s)=${modes || "-"}  fractional=${fractional}/${wheels.length}  maxAbs dy=${maxAbsY.toFixed(1)}`,
    `   gap ms: min=${gaps.length ? Math.min(...gaps) : "-"} median=${median(gaps)} max=${gaps.length ? Math.max(...gaps) : "-"}`,
    `Q6 ctrl-wheel after gestureend . ${tail.length ? `${tail.length} within ${Math.max(...tail)}ms` : "none"}`,
    `window.__inputProbeDump() to copy JSON · __inputProbeClear() to reset`,
  ].join("\n");
}

function median(xs: number[]): string {
  if (!xs.length) return "-";
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)].toFixed(1);
}
