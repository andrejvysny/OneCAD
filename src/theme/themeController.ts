/*
 * Theme controller — the SOLE writer of the theme attribute on <html>.
 *
 * Collapses two inputs (the persisted preference on settingsStore, and the OS
 * appearance via matchMedia) into one resolved theme, stamps it on the document
 * element, and notifies subscribers. Everything downstream reads the DOM:
 *
 *   - CSS re-themes itself with zero JS, because Tailwind v4 utilities compile
 *     to var(--color-*) and tokens.css redefines those under
 *     :root[data-theme="dark"].
 *   - The 3D viewport cannot: it caches THREE.Color instances and builds most
 *     materials once. It subscribes here and rebuilds (see ViewportRoot).
 *
 * Notifications fire ONLY when the resolved value actually changes. A listener
 * here triggers a PMREM environment rebuild in the viewport, so firing on every
 * unrelated settings write (or on an OS media event that resolves to the same
 * theme, which happens whenever the preference is an explicit light/dark) would
 * burn real GPU work for nothing.
 */
import { settingsStore } from "@/stores/settingsStore";
import { resolveTheme, type ResolvedTheme, type ThemePref } from "./themes";

const DARK_QUERY = "(prefers-color-scheme: dark)";

const listeners = new Set<(theme: ResolvedTheme) => void>();

/** The theme currently stamped on the document. `null` until first applied. */
let current: ResolvedTheme | null = null;
/** Teardown for the active controller; also the "is running" flag. */
let stop: (() => void) | null = null;
/** Created once — `matchMedia()` allocates a live MediaQueryList per call. */
let mql: MediaQueryList | null = null;

function mediaQuery(): MediaQueryList | null {
  if (mql) return mql;
  // jsdom has no matchMedia unless stubbed (see src/test/setup.ts).
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return null;
  mql = window.matchMedia(DARK_QUERY);
  return mql;
}

function systemPrefersDark(): boolean {
  return mediaQuery()?.matches ?? false;
}

/** Preference + OS appearance → what the DOM should be wearing. */
function compute(): ResolvedTheme {
  return resolveTheme(settingsStore.getState().theme, systemPrefersDark());
}

function apply(next: ResolvedTheme): void {
  if (current === next) return;
  current = next;
  if (typeof document !== "undefined" && document.documentElement) {
    const root = document.documentElement;
    root.dataset.theme = next;
    // Tells the UA to render form controls, scrollbars and the canvas
    // background in the matching appearance. Without it a dark app keeps white
    // native scrollbars.
    root.style.colorScheme = next;
  }
  for (const cb of listeners) cb(next);
}

/**
 * Tell the NATIVE window which appearance to wear.
 *
 * macOS draws the traffic lights itself (tauri.conf.json sets
 * titleBarStyle "Overlay" + hiddenTitle), and those buttons follow the OS, not
 * our CSS — so a dark app on a light desktop gets light window controls sitting
 * on a dark title bar unless we say otherwise. "system" passes null, which
 * hands the decision back to the OS.
 *
 * Fire-and-forget, and deliberately un-awaited: this is decoration, and a
 * failure here must never block the DOM theme. No-ops in the browser/e2e lane,
 * where __TAURI_INTERNALS__ is absent.
 */
function syncNativeWindowTheme(pref: ThemePref): void {
  if (typeof window === "undefined") return;
  if (!("__TAURI_INTERNALS__" in window)) return;
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) =>
      getCurrentWindow().setTheme(pref === "system" ? null : pref),
    )
    .catch(() => {
      // Missing capability or an older webview — the in-app theme still applied.
    });
}

/** Last preference pushed to the native window, so unrelated writes are free. */
let lastNativePref: ThemePref | null = null;

function refresh(): void {
  apply(compute());
  // `refresh` runs on EVERY settings write (the subscription is store-wide), so
  // guard here too — otherwise toggling a snap checkbox would fire a dynamic
  // import and an IPC call.
  const pref = settingsStore.getState().theme;
  if (pref !== lastNativePref) {
    lastNativePref = pref;
    syncNativeWindowTheme(pref);
  }
}

/**
 * Start following the preference and the OS. Idempotent — a second call returns
 * the existing teardown rather than double-subscribing (StrictMode, tests).
 * Applies the current theme synchronously before returning, so callers can run
 * this before the first React render.
 */
export function startThemeController(): () => void {
  if (stop) return stop;

  const unsubStore = settingsStore.subscribe(refresh);

  let unsubMedia = () => {};
  const q = mediaQuery();
  if (q) {
    const onChange = () => refresh();
    if (typeof q.addEventListener === "function") {
      q.addEventListener("change", onChange);
      unsubMedia = () => q.removeEventListener("change", onChange);
    } else if (typeof q.addListener === "function") {
      // Safari < 14 (WKWebView on macOS 10.15) has only the deprecated form.
      q.addListener(onChange);
      unsubMedia = () => q.removeListener(onChange);
    }
  }

  refresh();

  stop = () => {
    unsubStore();
    unsubMedia();
    stop = null;
  };
  return stop;
}

/**
 * The theme the DOM is wearing. Safe to call before `startThemeController()` —
 * it derives on demand rather than reporting a stale or absent value.
 */
export function getResolvedTheme(): ResolvedTheme {
  return current ?? compute();
}

/**
 * Subscribe to resolved-theme CHANGES. Does not fire on subscribe; call
 * {@link getResolvedTheme} for the current value. Returns an unsubscribe.
 */
export function subscribeResolvedTheme(cb: (theme: ResolvedTheme) => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** Test seam: drop all module state so each test starts from a cold controller. */
export function __resetThemeControllerForTests(): void {
  stop?.();
  listeners.clear();
  current = null;
  mql = null;
  lastNativePref = null;
}
