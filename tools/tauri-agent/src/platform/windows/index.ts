import type { PlatformAdapter } from "../adapter.ts";
import { unsupported } from "../adapter.ts";

/** Phase 1 is macOS-only; the Windows adapter is a deliberate, loud gap. */
export function createWindowsAdapter(): PlatformAdapter {
  throw unsupported("win32");
}
