// Extends Vitest's `expect` with jest-dom matchers (toBeInTheDocument, ...)
// and registers the corresponding TypeScript augmentation.
import "@testing-library/jest-dom/vitest";

import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import { __resetLogForTests } from "@/debug/log";

// jsdom implements no matchMedia at all, and themeController needs one to
// resolve the "system" preference. Defaults to LIGHT (matches: false) so tests
// that never touch appearance behave exactly as they did before dark mode.
// A test wanting a dark OS overrides window.matchMedia itself.
if (typeof window !== "undefined" && typeof window.matchMedia !== "function") {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    configurable: true,
    value: (query: string): MediaQueryList =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
      }) as unknown as MediaQueryList,
  });
}

// React Testing Library does not auto-clean without global afterEach.
afterEach(() => {
  cleanup();
  // The logger's gate is CLOSED under vitest (see src/debug/log.ts). A test that
  // opened it with `__resetLogForTests()` to assert on emitted events must not
  // leak that into its neighbours — closing it here also drops the ring and any
  // subscriber the test forgot to detach.
  __resetLogForTests({ enabled: false });
});
