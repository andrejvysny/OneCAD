/*
 * Registry invariants (ADR-0006, ADR-0007). These are the three properties every
 * later wave leans on, so each failure mode is pinned explicitly rather than
 * implied by a happy-path test.
 */
import { describe, it, expect, vi } from "vitest";
import { createRegistry, RegistrationError, DEFAULT_PRIORITY } from "./registry";
import { moduleId, addonId } from "./ids";

const MODELING = moduleId("onecad.modeling");
const FOO = addonId("com.example.foo");

interface Entry {
  id: string;
  group?: string;
  priority?: number;
  label?: string;
}

const entry = (id: string, priority?: number, group?: string): Entry => ({
  id,
  priority,
  group,
});

describe("registry — ownership", () => {
  it("records the owner of each registration", () => {
    const r = createRegistry<Entry>("thing");
    r.register(MODELING, entry("onecad.modeling.a"));
    r.register(FOO, entry("com.example.foo.a"));

    expect(r.ownerOf("onecad.modeling.a")).toBe(MODELING);
    expect(r.ownerOf("com.example.foo.a")).toBe(FOO);
  });

  it("disposeOwner removes every one of that owner's entries and no others", () => {
    const r = createRegistry<Entry>("thing");
    r.register(MODELING, entry("onecad.modeling.a"));
    r.register(MODELING, entry("onecad.modeling.b"));
    r.register(FOO, entry("com.example.foo.a"));

    expect(r.disposeOwner(MODELING)).toBe(2);
    expect(r.entries().map((e) => e.id)).toEqual(["com.example.foo.a"]);
  });

  it("a single handle removes only its own registration", () => {
    const r = createRegistry<Entry>("thing");
    const handle = r.register(MODELING, entry("onecad.modeling.a"));
    r.register(MODELING, entry("onecad.modeling.b"));

    handle.dispose();
    expect(r.entries().map((e) => e.id)).toEqual(["onecad.modeling.b"]);
  });

  it("a stale handle cannot evict a later registration of the same id", () => {
    const r = createRegistry<Entry>("thing");
    const stale = r.register(MODELING, entry("onecad.modeling.a"));
    stale.dispose();

    // Re-registered after the first handle went away — a leftover disposer from
    // the previous owner of the id must not silently unregister the new one.
    r.register(MODELING, { id: "onecad.modeling.a", label: "second" });
    stale.dispose();

    expect(r.get("onecad.modeling.a")?.label).toBe("second");
  });

  it("disposing twice is harmless", () => {
    const r = createRegistry<Entry>("thing");
    const handle = r.register(MODELING, entry("onecad.modeling.a"));
    handle.dispose();
    handle.dispose();
    expect(r.size).toBe(0);
  });
});

describe("registry — duplicate and namespace rejection", () => {
  it("a duplicate id is rejected, naming the current holder", () => {
    const r = createRegistry<Entry>("command");
    r.register(MODELING, entry("onecad.modeling.a"));

    // Deterministic failure, never last-one-wins — including for the module's own
    // second registration, which is how a real collision usually happens (two
    // call sites contributing the same command).
    expect(() => r.register(MODELING, { id: "onecad.modeling.a" })).toThrowError(
      RegistrationError,
    );
    try {
      r.register(MODELING, { id: "onecad.modeling.a" });
    } catch (e) {
      expect((e as RegistrationError).code).toBe("duplicate-id");
      expect((e as Error).message).toContain("onecad.modeling");
    }
    expect(r.ownerOf("onecad.modeling.a")).toBe(MODELING);
  });

  it("a foreign owner is rejected for namespace before duplication", () => {
    const r = createRegistry<Entry>("command");
    r.register(MODELING, entry("onecad.modeling.a"));
    // The more specific diagnosis wins: an addon reaching for a built-in id is an
    // impersonation attempt, not merely a naming clash.
    try {
      r.register(FOO, { id: "onecad.modeling.a" });
      expect.unreachable("a foreign namespace must be refused");
    } catch (e) {
      expect((e as RegistrationError).code).toBe("foreign-namespace");
    }
  });

  it("an unowned-id registry still rejects duplicates across owners", () => {
    const r = createRegistry<Entry>("module", { allowUnownedIds: true });
    r.register(MODELING, { id: "shared" });
    try {
      r.register(FOO, { id: "shared" });
      expect.unreachable("a duplicate must be refused even without namespacing");
    } catch (e) {
      expect((e as RegistrationError).code).toBe("duplicate-id");
    }
  });

  it("an id outside the owner's namespace is rejected", () => {
    const r = createRegistry<Entry>("command");
    // An addon impersonating a built-in is the case this closes.
    expect(() => r.register(FOO, entry("onecad.modeling.extrude"))).toThrowError(
      RegistrationError,
    );
    try {
      r.register(FOO, entry("onecad.modeling.extrude"));
    } catch (e) {
      expect((e as RegistrationError).code).toBe("foreign-namespace");
    }
  });

  it("a rejected registration leaves the registry untouched", () => {
    const r = createRegistry<Entry>("command");
    r.register(MODELING, entry("onecad.modeling.a"));
    expect(() => r.register(FOO, entry("onecad.modeling.a"))).toThrow();
    expect(r.size).toBe(1);
  });

  it("allowUnownedIds lets a registry key by the owner id itself", () => {
    const r = createRegistry<Entry>("module", { allowUnownedIds: true });
    expect(() => r.register(MODELING, { id: MODELING })).not.toThrow();
  });
});

describe("registry — deterministic order", () => {
  it("orders by priority, not by registration order", () => {
    const r = createRegistry<Entry>("thing");
    r.register(MODELING, entry("onecad.modeling.c", 300));
    r.register(MODELING, entry("onecad.modeling.a", 100));
    r.register(MODELING, entry("onecad.modeling.b", 200));

    expect(r.entries().map((e) => e.id)).toEqual([
      "onecad.modeling.a",
      "onecad.modeling.b",
      "onecad.modeling.c",
    ]);
  });

  it("produces the same order regardless of the order things registered in", () => {
    const ids = ["onecad.modeling.a", "onecad.modeling.b", "onecad.modeling.c"];
    const priorities = new Map([
      ["onecad.modeling.a", 30],
      ["onecad.modeling.b", 10],
      ["onecad.modeling.c", 20],
    ]);

    const orderOf = (registration: string[]): string[] => {
      const r = createRegistry<Entry>("thing");
      for (const id of registration) r.register(MODELING, entry(id, priorities.get(id)));
      return r.entries().map((e) => e.id);
    };

    const expected = ["onecad.modeling.b", "onecad.modeling.c", "onecad.modeling.a"];
    expect(orderOf(ids)).toEqual(expected);
    expect(orderOf([...ids].reverse())).toEqual(expected);
    expect(orderOf([ids[1], ids[0], ids[2]])).toEqual(expected);
  });

  it("does NOT sort by group — priority alone decides (ADR-0007)", () => {
    // Groups deliberately spelled so that alphabetical order is the REVERSE of
    // the intended one. A registry that sorted by group first would return
    // "aaa.late" before "zzz.early", which is the failure the ADR describes:
    // the UI would order itself by how a group happens to be spelled.
    const r = createRegistry<Entry>("thing");
    r.register(MODELING, entry("onecad.modeling.late", 200, "aaa.group"));
    r.register(MODELING, entry("onecad.modeling.early", 100, "zzz.group"));

    expect(r.entries().map((e) => e.id)).toEqual([
      "onecad.modeling.early",
      "onecad.modeling.late",
    ]);
  });

  it("falls back to registration order only for an exact priority tie", () => {
    const r = createRegistry<Entry>("thing");
    r.register(MODELING, entry("onecad.modeling.second", 100));
    r.register(MODELING, entry("onecad.modeling.first", 100));

    expect(r.entries().map((e) => e.id)).toEqual([
      "onecad.modeling.second",
      "onecad.modeling.first",
    ]);
  });

  it("an absent priority is the documented default, not zero", () => {
    const r = createRegistry<Entry>("thing");
    r.register(MODELING, entry("onecad.modeling.explicit", DEFAULT_PRIORITY - 1));
    r.register(MODELING, entry("onecad.modeling.default"));

    expect(r.entries().map((e) => e.id)).toEqual([
      "onecad.modeling.explicit",
      "onecad.modeling.default",
    ]);
  });
});

describe("registry — change notification", () => {
  it("notifies on register and on dispose", () => {
    const r = createRegistry<Entry>("thing");
    const listener = vi.fn();
    const unsubscribe = r.subscribe(listener);

    const handle = r.register(MODELING, entry("onecad.modeling.a"));
    expect(listener).toHaveBeenCalledTimes(1);
    handle.dispose();
    expect(listener).toHaveBeenCalledTimes(2);

    unsubscribe();
    r.register(MODELING, entry("onecad.modeling.b"));
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("keeps a stable snapshot reference between changes", () => {
    // useSyncExternalStore loops forever on a snapshot that is a fresh array
    // every call, so this identity is load-bearing, not an optimization.
    const r = createRegistry<Entry>("thing");
    r.register(MODELING, entry("onecad.modeling.a"));

    const first = r.entries();
    expect(r.entries()).toBe(first);

    r.register(MODELING, entry("onecad.modeling.b"));
    expect(r.entries()).not.toBe(first);
  });

  it("does not notify when a no-op dispose changes nothing", () => {
    const r = createRegistry<Entry>("thing");
    const handle = r.register(MODELING, entry("onecad.modeling.a"));
    handle.dispose();

    const listener = vi.fn();
    r.subscribe(listener);
    handle.dispose();
    expect(listener).not.toHaveBeenCalled();
    expect(r.disposeOwner(FOO)).toBe(0);
    expect(listener).not.toHaveBeenCalled();
  });
});
