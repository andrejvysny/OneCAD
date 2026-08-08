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
