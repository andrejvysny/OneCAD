/*
 * The bounded owned-overlay cache (VP-HARDENING spec §8.3, TEST-RES-03).
 *
 * Byte sizes here are declared rather than measured: the cache budgets what its
 * callers tell it they own, and the point of these cases is the POLICY — the
 * 64 MiB / 256-entry ceiling, LRU order, that a displayed (pinned) overlay is
 * never evicted, that retiring a source retires everything cut from it, and
 * that an exhausted budget degrades loudly instead of dropping the selection.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import type * as THREE from "three";
import {
  HighlightCache,
  highlightCacheKey,
  ordinalsKey,
  HIGHLIGHT_CACHE_MAX_BYTES,
  HIGHLIGHT_CACHE_MAX_ENTRIES,
  type HighlightCacheValue,
} from "./highlightCache";
import type { MeshEntry } from "./meshRegistry";
import { __resetLogForTests, logSnapshot } from "@/debug/log";

const MIB = 1024 * 1024;

function fakeEntry(bodyId = "body1"): MeshEntry {
  return { bodyId } as unknown as MeshEntry;
}

function fakeValue(entry: MeshEntry, bytes: number): HighlightCacheValue {
  return {
    kind: "faceSet",
    geometry: { dispose: vi.fn() } as unknown as THREE.BufferGeometry,
    owned: null,
    bytes,
    entry,
    pinned: 0,
  };
}

/** Reserve-then-put, the sequence every caller uses. */
function admit(cache: HighlightCache, key: string, entry: MeshEntry, bytes: number): boolean {
  if (!cache.reserve(bytes, "test")) return false;
  cache.put(key, fakeValue(entry, bytes));
  return true;
}

beforeEach(() => {
  __resetLogForTests({ enabled: true, console: false });
});

describe("TEST-RES-03 — highlight cache bounds", () => {
  it("never exceeds 256 entries, evicting the least recently used", () => {
    const cache = new HighlightCache();
    const entry = fakeEntry();
    for (let i = 0; i < HIGHLIGHT_CACHE_MAX_ENTRIES; i++) admit(cache, `k${i}`, entry, 16);
    expect(cache.size).toBe(HIGHLIGHT_CACHE_MAX_ENTRIES);

    expect(admit(cache, "overflow", entry, 16)).toBe(true);

    expect(cache.size).toBe(HIGHLIGHT_CACHE_MAX_ENTRIES);
    expect(cache.has("k0")).toBe(false); // the oldest went
    expect(cache.has("k1")).toBe(true);
    expect(cache.has("overflow")).toBe(true);
  });

  it("never exceeds 64 MiB, whichever ceiling is reached first", () => {
    const cache = new HighlightCache();
    const entry = fakeEntry();
    for (let i = 0; i < 64; i++) admit(cache, `k${i}`, entry, MIB);
    expect(cache.bytes).toBe(HIGHLIGHT_CACHE_MAX_BYTES);

    admit(cache, "next", entry, MIB);

    expect(cache.bytes).toBeLessThanOrEqual(HIGHLIGHT_CACHE_MAX_BYTES);
    expect(cache.has("k0")).toBe(false);
    expect(cache.has("next")).toBe(true);
  });

  it("evicts in LRU order — a `get` moves a slot to the young end", () => {
    const cache = new HighlightCache(4 * MIB, 8);
    const entry = fakeEntry();
    admit(cache, "a", entry, MIB);
    admit(cache, "b", entry, MIB);
    admit(cache, "c", entry, MIB);

    cache.get("a"); // touched, so "b" is now the oldest
    admit(cache, "d", entry, 2 * MIB);

    expect(cache.has("b")).toBe(false);
    expect(cache.has("a")).toBe(true);
    expect(cache.has("d")).toBe(true);
  });

  it("never evicts a PINNED slot, however old", () => {
    const cache = new HighlightCache(3 * MIB, 8);
    const entry = fakeEntry();
    admit(cache, "pinned", entry, MIB);
    cache.pin("pinned");
    admit(cache, "young", entry, MIB);

    expect(admit(cache, "next", entry, 2 * MIB)).toBe(true);

    expect(cache.has("pinned")).toBe(true);
    expect(cache.has("young")).toBe(false); // the unpinned one paid instead
  });

  it("unpinning makes a slot evictable again", () => {
    const cache = new HighlightCache(2 * MIB, 8);
    const entry = fakeEntry();
    admit(cache, "a", entry, MIB);
    cache.pin("a");
    cache.unpin("a");

    admit(cache, "b", entry, 2 * MIB);

    expect(cache.has("a")).toBe(false);
  });

  it("retireSource drops and DISPOSES everything cut from that resource", () => {
    const cache = new HighlightCache();
    const doomed = fakeEntry("body1");
    const other = fakeEntry("body2");
    const a = fakeValue(doomed, 32);
    const b = fakeValue(other, 32);
    cache.put("a", a);
    cache.put("b", b);
    cache.pin("a"); // pinned, and it still goes: the source is stale

    cache.retireSource(doomed);

    expect(cache.has("a")).toBe(false);
    expect(cache.has("b")).toBe(true);
    expect(a.geometry.dispose).toHaveBeenCalledTimes(1);
    expect(b.geometry.dispose).not.toHaveBeenCalled();
    expect(cache.bytes).toBe(32);
  });

  it("take removes a slot WITHOUT disposing it, and refuses a pinned one", () => {
    const cache = new HighlightCache();
    const entry = fakeEntry();
    const value = fakeValue(entry, 64);
    cache.put("a", value);
    cache.pin("a");

    expect(cache.take("a")).toBeUndefined(); // still displayed

    cache.unpin("a");
    expect(cache.take("a")).toBe(value);
    expect(cache.has("a")).toBe(false);
    expect(cache.bytes).toBe(0);
    expect(value.geometry.dispose).not.toHaveBeenCalled(); // handed on, not freed
  });

  it("evictUnpinned clears everything not on screen", () => {
    const cache = new HighlightCache();
    const entry = fakeEntry();
    admit(cache, "a", entry, 16);
    admit(cache, "b", entry, 16);
    cache.pin("b");

    cache.evictUnpinned();

    expect(cache.has("a")).toBe(false);
    expect(cache.has("b")).toBe(true);
  });

  it("an all-pinned over-budget selection degrades with ONE diagnostic per change", () => {
    const cache = new HighlightCache(2 * MIB, 8);
    const entry = fakeEntry();
    admit(cache, "a", entry, MIB);
    cache.pin("a");
    admit(cache, "b", entry, MIB);
    cache.pin("b");

    cache.beginChange();
    expect(cache.reserve(MIB, "faceSet")).toBe(false);
    expect(cache.reserve(MIB, "faceSet")).toBe(false); // a second body, same change
    expect(cache.degraded).toBe(true);
    // The semantic selection is untouched — nothing was evicted or dropped.
    expect(cache.size).toBe(2);
    const warns = logSnapshot().filter((e) => e.level === "warn");
    expect(warns).toHaveLength(1);
    expect(warns[0].msg).toContain("highlight overlay budget exhausted");

    // A NEW change gets its own single diagnostic.
    cache.beginChange();
    expect(cache.degraded).toBe(false);
    expect(cache.reserve(MIB, "faceSet")).toBe(false);
    expect(logSnapshot().filter((e) => e.level === "warn")).toHaveLength(2);
  });

  it("an ordinary hover still fits: unpinned slots are evicted to make room", () => {
    const cache = new HighlightCache(2 * MIB, 8);
    const entry = fakeEntry();
    admit(cache, "cold", entry, 2 * MIB); // fills the budget, but nothing displays it

    cache.beginChange();
    expect(admit(cache, "hover", entry, MIB)).toBe(true);
    expect(cache.degraded).toBe(false);
    expect(cache.has("cold")).toBe(false);
  });

  it("clear disposes every slot, pinned included (document close)", () => {
    const cache = new HighlightCache();
    const entry = fakeEntry();
    const value = fakeValue(entry, 64);
    cache.put("a", value);
    cache.pin("a");

    cache.clear();

    expect(cache.size).toBe(0);
    expect(cache.bytes).toBe(0);
    expect(value.geometry.dispose).toHaveBeenCalledTimes(1);
  });
});

describe("cache keys", () => {
  it("ordinals are canonicalized so selection ORDER never forks the cache", () => {
    expect(ordinalsKey([3, 1, 2])).toBe(ordinalsKey([1, 2, 3]));
    expect(highlightCacheKey("id", "faceSet", [2, 1])).toBe(
      highlightCacheKey("id", "faceSet", [1, 2]),
    );
  });

  it("kind and identity are both part of the key", () => {
    expect(highlightCacheKey("id", "edge", [1])).not.toBe(highlightCacheKey("id", "faceSet", [1]));
    expect(highlightCacheKey("id-a", "edge", [1])).not.toBe(highlightCacheKey("id-b", "edge", [1]));
  });
});
