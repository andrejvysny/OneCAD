/*
 * Architecture enforcement (spec §163).
 *
 * The dependency rules in docs/ARCHITECTURE.md decay the moment they exist only
 * in prose — the next plausible-looking import puts them back. This scans the
 * real import graph and fails on a forbidden edge, naming the file.
 *
 * It is deliberately a plain text scan: no new dependency, no build step, and it
 * reads the same specifiers a human would grep for.
 */
import { describe, it, expect } from "vitest";

type Sources = Record<string, string>;

const platformSources = import.meta.glob("./**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Sources;

const moduleSources = import.meta.glob("../modules/**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Sources;

/** Every module specifier a file imports (static or dynamic). */
function importsOf(source: string): string[] {
  const specifiers: string[] = [];
  for (const m of source.matchAll(/from\s+["']([^"']+)["']/g)) specifiers.push(m[1]);
  for (const m of source.matchAll(/import\(\s*["']([^"']+)["']\s*\)/g)) specifiers.push(m[1]);
  return specifiers;
}

function offenders(sources: Sources, forbidden: readonly string[], skip: (f: string) => boolean) {
  const found: string[] = [];
  for (const [file, source] of Object.entries(sources)) {
    if (file.endsWith(".test.ts") || file.endsWith(".test.tsx") || skip(file)) continue;
    for (const specifier of importsOf(source)) {
      if (forbidden.some((prefix) => specifier.startsWith(prefix))) {
        found.push(`${file} → ${specifier}`);
      }
    }
  }
  return found;
}

describe("platform dependency rules", () => {
  it("finds the files it is supposed to be checking", () => {
    // A glob that silently matches nothing would make every rule below vacuous.
    expect(Object.keys(platformSources).length).toBeGreaterThan(5);
    expect(Object.keys(moduleSources).length).toBeGreaterThan(5);
  });

  it("the scanner actually sees imports", () => {
    // Positive control. Every rule below asserts an EMPTY result, which is also
    // what a broken scanner returns — so prove it finds an edge that really
    // exists: the document-state service imports the IPC client contract.
    expect(offenders(platformSources, ["@/ipc"], () => false)).not.toEqual([]);
  });

  it("the Platform does not depend on any module or feature implementation", () => {
    // This is THE rule the refactor exists to establish: composition flows
    // Shell → Platform → contracts ← modules, never Platform → Modeling.
    expect(
      offenders(
        platformSources,
        ["@/features", "@/tools", "@/modules", "@/stores", "@/viewport", "@/app"],
        () => false,
      ),
    ).toEqual([]);
  });

  it("the Platform does not reach into the app shell", () => {
    expect(offenders(platformSources, ["@/app/"], () => false)).toEqual([]);
  });

  it("modules do not import the app shell", () => {
    // A module contributes and is rendered; it never reaches for the shell that
    // renders it, or the inversion is gone.
    expect(offenders(moduleSources, ["@/app/"], () => false)).toEqual([]);
  });

  it("modules reach the platform through its public surface", () => {
    // `@/platform/...` deep paths bypass the barrel and pin internal file layout
    // as if it were API.
    expect(offenders(moduleSources, ["@/platform/"], () => false)).toEqual([]);
  });
});

const addonSources = import.meta.glob("../addons/**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Sources;

const sdkSources = import.meta.glob("../sdk/**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Sources;

/*
 * The SDK boundary (P2.5 WP5/WP6).
 *
 * `@/platform` is the internal host API and `@onecad/sdk` is the public one.
 * The difference is only real if something checks it — in prose it lasts until
 * the first plausible-looking import.
 */
describe("SDK dependency rules", () => {
  it("finds the files it is supposed to be checking", () => {
    expect(Object.keys(addonSources).length).toBeGreaterThan(2);
    expect(Object.keys(sdkSources).length).toBeGreaterThan(0);
  });

  it("the scanner sees the addon's imports", () => {
    // Positive control, same reasoning as above: the rules assert EMPTY, which a
    // broken glob also returns. The addon really does import the SDK.
    expect(offenders(addonSources, ["@onecad/sdk"], () => false)).not.toEqual([]);
  });

  it("the reference addon imports NOTHING of the application", () => {
    // If this ever needs an exception, the exception is a missing SDK export.
    expect(
      offenders(
        addonSources,
        [
          "@/platform",
          "@/modules",
          "@/features",
          "@/stores",
          "@/tools",
          "@/viewport",
          "@/ipc",
          "@/app",
          "@/ui",
          "@/icons",
          "@/debug",
        ],
        // Its own tests drive the REAL hosts, so they may reach for them.
        (file) => file.includes(".test."),
      ),
    ).toEqual([]);
  });

  it("the SDK re-exports the platform and nothing else", () => {
    // The SDK is a curated view OF the platform, so `@/platform*` is its one
    // allowed dependency; anything else would put application implementation
    // behind a contract we promise to keep.
    expect(
      offenders(
        sdkSources,
        ["@/modules", "@/features", "@/stores", "@/tools", "@/viewport", "@/ipc", "@/app"],
        () => false,
      ),
    ).toEqual([]);
  });

  it("the SDK does not export the host itself", async () => {
    // An addon that can build a Platform, reach a Registry, or dispose another
    // owner is not sandboxed by a narrower context — it can simply go around it.
    const sdk: Record<string, unknown> = await import("@onecad/sdk");
    for (const forbidden of [
      "createPlatform",
      "createRegistry",
      "createServiceRegistry",
      "createEventBus",
      "createToolHost",
      "createShortcutService",
      "createExtensionContext",
      "registerExtension",
      "createDocumentStateService",
      "PlatformProvider",
      "SlotHost",
      "usePlatform",
      "useRegistryEntries",
    ]) {
      expect(sdk[forbidden], `@onecad/sdk must not export ${forbidden}`).toBeUndefined();
    }
  });

  it("the SDK's runtime surface is a deliberate list", () => {
    // A snapshot with a REASON: every runtime value here is a promise to keep
    // working. Widening it should be a visible diff in a review, not a side
    // effect of adding an export somewhere in the platform.
    return import("@onecad/sdk").then((sdk) => {
      expect(Object.keys(sdk).sort()).toEqual([
        "DEFAULT_PRIORITY",
        "IdError",
        "Slots",
        "addonId",
        "contributionId",
        "isOwnerIdShape",
        "isSlotId",
      ]);
    });
  });
});
