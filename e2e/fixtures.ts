/*
 * Wraps @playwright/test with an auto fixture that captures browser console
 * output and uncaught page errors for every test, then attaches them — plus
 * the FE `window.__logsDump()` ring buffer (src/debug/log.ts) — to the
 * Playwright report when a test fails or the page threw at all. This is the
 * e2e half of DEV-OBSERVABILITY (see docs/DEBUGGING.md): before this, a
 * failing e2e spec gave no browser-side signal beyond the assertion diff.
 *
 * Specs import `test`/`expect` from here instead of "@playwright/test" so
 * every spec gets capture for free with zero call-site changes; `helpers.ts`
 * and `modelToolHelpers.ts` stay on the raw @playwright/test import — they
 * only need the `Page`/`Locator` *types*, never a `test` fixture.
 *
 * Filename has no `.spec.` segment, so playwright.config.ts's `testDir: "./e2e"`
 * default match never collects this file as a test. The fixture depends only
 * on the built-in `page` fixture, which Playwright scopes per-test, so the
 * buffers below can't leak across tests even under this project's
 * `workers: 1`.
 *
 * ── Why `path`, not `body` ───────────────────────────────────────────────────
 * `testInfo.attach(name, { body })` keeps the content as an in-memory Buffer —
 * playwright/lib/util.js `normalizeAndSaveAttachment` only copies bytes to disk
 * (under `<outputPath>/attachments/`) when given `{ path }`. This project's
 * reporter is "list"/"line" (not "html"/"json"), which never serializes a
 * `body` attachment to a file, so a `body`-based attach here would leave
 * nothing in `test-results/` to open. Writing the file ourselves and pushing
 * it onto `testInfo.attachments` directly (the pattern from Playwright's own
 * "Automatic Fixture" doc) lands it at `<outputDir>/<test>/<name>` exactly —
 * `test-results/run-<stamp>/<test>/<name>`, since every run gets its own
 * directory so a later run cannot wipe this one's evidence (`e2e/outputDir.ts`).
 *
 * Both failure modes are covered, verified by running them: an assertion diff
 * and a hard test timeout each leave `console.log`, `pageerror.log` and
 * `fe-logs.json` on disk — the teardown below still runs after a timeout.
 */
import { promises as fs } from "node:fs";
import { test as base, expect } from "@playwright/test";

export { expect };

async function attachFile(
  testInfo: import("@playwright/test").TestInfo,
  name: string,
  contentType: string,
  body: string,
): Promise<void> {
  const path = testInfo.outputPath(name);
  await fs.writeFile(path, body, "utf8");
  testInfo.attachments.push({ name, contentType, path });
}

export const test = base.extend<{ forwardBrowserLogs: void }>({
  forwardBrowserLogs: [
    async ({ page }, use, testInfo) => {
      const consoleLines: string[] = [];
      const pageerrorLines: string[] = [];

      page.on("console", (msg) => {
        consoleLines.push(`[${msg.type()}] ${msg.text()}`);
      });
      page.on("pageerror", (err) => {
        pageerrorLines.push(err.stack ?? String(err));
      });

      await use();

      // Always capture on a failing test; also capture on a PASSING test that
      // still threw a page error, since a caught-but-logged error is exactly
      // the kind of thing that should be visible without waiting for the
      // assertion it happens not to affect to fail too.
      const failed = testInfo.status !== testInfo.expectedStatus;
      if (!failed && pageerrorLines.length === 0) return;

      await attachFile(testInfo, "console.log", "text/plain", consoleLines.join("\n"));
      await attachFile(testInfo, "pageerror.log", "text/plain", pageerrorLines.join("\n"));
      const feLogs = await page
        .evaluate(() => (window as unknown as { __logsDump?: () => string }).__logsDump?.() ?? "")
        .catch(() => "");
      await attachFile(testInfo, "fe-logs.json", "application/json", feLogs);
    },
    { auto: true },
  ],
});
