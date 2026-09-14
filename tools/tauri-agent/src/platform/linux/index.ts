import type { PlatformAdapter } from "../adapter.ts";
import { unsupported } from "../adapter.ts";

/** Phase 1 is macOS-only; the Linux adapter is a deliberate, loud gap. */
export function createLinuxAdapter(): PlatformAdapter {
  throw unsupported(process.platform);
}
