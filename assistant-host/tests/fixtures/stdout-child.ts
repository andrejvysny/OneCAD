/**
 * Spawned by `../stdout.test.ts`. The claim can only be observed honestly in a
 * child process: in-process, the test runner owns stdout and patching it would
 * change what the assertion is measuring.
 */
import { claimStdout } from "../../src/log.js";

const writeFrame = claimStdout();

console.log("POLLUTION_log");
console.info("POLLUTION_info");
console.debug("POLLUTION_debug");
console.warn("POLLUTION_warn");
console.error("POLLUTION_error");
process.stdout.write("POLLUTION_raw\n");

// The one writer that is allowed through.
await writeFrame(new TextEncoder().encode("FRAME_ONLY"));
