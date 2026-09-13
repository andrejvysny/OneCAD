/*
 * Bounded LRU cache for OWNED highlight overlay resources (spec §8.3, VP04).
 *
 * Everything in here owns GL buffers of its own: compact face-set geometry cut
 * by `faceSliceGeometry`, and the per-edge `LineSegmentsGeometry` built over a
 * slice of the entry's segment positions. Whole-body highlights are NOT cached
 * — they borrow the body's exact geometry object through a registry lease and
 * have nothing to own (spec §8.2).
 *
 * Three rules make the bound honest:
 *
 *  1. A key is the resource IDENTITY plus the kind plus the element ordinals.
 *     A mesh swap changes the identity, so a stale overlay can never be served
 *     as a hit for the new mesh.
 *  2. An entry that is currently displayed is PINNED and can never be evicted.
 *     Eviction takes unpinned entries in least-recently-used order.
 *  3. When a source resource retires, every overlay cut from it retires with
 *     it — pinned or not. A compact copy of a superseded mesh is stale
 *     geometry, and keeping it would be exactly the "silent wrong bind" the
 *     program exists to remove.
 *
 * DEGRADED MODE: if the pinned set alone exceeds the budget, the semantic
 * selection is NEVER discarded. The cache refuses the reservation, raises
 * `degraded`, and emits ONE diagnostic per selection change; HighlightLayer
 * then draws that body's selection as a whole-body leased highlight, which owns
 * no buffers at all. An ordinary single-face hover always fits, because
 * unpinned entries are evicted to make room for it first.
 */
import type * as THREE from "three";
import { logWarn } from "@/debug/log";
import { reportHighlightCache } from "../vph/instrumentation";
import type { MeshEntry } from "./meshRegistry";
import type { OwnedFaceGeometry } from "./faceSliceGeometry";

export const HIGHLIGHT_CACHE_MAX_BYTES = 64 * 1024 * 1024;
export const HIGHLIGHT_CACHE_MAX_ENTRIES = 256;

/** What kind of owned overlay a cache slot holds. */
export type HighlightCacheKind = "faceSet" | "edge";

export interface HighlightCacheValue {
  readonly kind: HighlightCacheKind;
  /** The owned overlay geometry. Disposed by the cache, and only by the cache. */
  readonly geometry: THREE.BufferGeometry;
  /** Present for a face-set slot, so the layer can refresh it in place. */
  readonly owned: OwnedFaceGeometry | null;
  /** Owned typed-array bytes charged to the budget. */
  readonly bytes: number;
  /** The resource this overlay was cut from — retirement follows it. */
  readonly entry: MeshEntry;
  /** Display references holding this slot. Non-zero ⇒ never evicted. */
  pinned: number;
}

export interface HighlightCacheStats {
  readonly entries: number;
  readonly bytes: number;
  readonly pinned: number;
}

export class HighlightCache {
  /** Insertion order IS the LRU order: `get` re-inserts at the end. */
  private readonly slots = new Map<string, HighlightCacheValue>();
  private totalBytes = 0;
  /** The last selection change could not fit its exact overlays in the budget. */
  private degradedFlag = false;
  private warnedThisChange = false;

  constructor(
    private readonly maxBytes = HIGHLIGHT_CACHE_MAX_BYTES,
    private readonly maxEntries = HIGHLIGHT_CACHE_MAX_ENTRIES,
  ) {}

  get size(): number {
    return this.slots.size;
  }

  get bytes(): number {
    return this.totalBytes;
  }

  /** True when the last selection change had to fall back to a degraded overlay. */
  get degraded(): boolean {
    return this.degradedFlag;
  }

  stats(): HighlightCacheStats {
    let pinned = 0;
    for (const v of this.slots.values()) if (v.pinned > 0) pinned++;
    return { entries: this.slots.size, bytes: this.totalBytes, pinned };
  }

  /**
   * A new selection/hover state is being built. Resets the per-change
   * diagnostic budget so a degraded selection reports exactly once, however
   * many bodies it spans.
   */
  beginChange(): void {
    this.degradedFlag = false;
    this.warnedThisChange = false;
  }

  /** LRU read. A hit moves the slot to the most-recently-used end. */
  get(key: string): HighlightCacheValue | undefined {
    const value = this.slots.get(key);
    if (!value) return undefined;
    this.slots.delete(key);
    this.slots.set(key, value);
    return value;
  }

  has(key: string): boolean {
    return this.slots.has(key);
  }

  /**
   * Make room for `bytes` (one more entry) by evicting unpinned LRU slots.
   * Returns false when the pinned set alone still exceeds the budget: the
   * caller must then take the documented degraded path rather than allocate.
   */
  reserve(bytes: number, reason: string): boolean {
    if (bytes > this.maxBytes) return this.refuse(bytes, reason);
    for (const [key, value] of this.slots) {
      if (this.totalBytes + bytes <= this.maxBytes && this.slots.size < this.maxEntries) break;
      if (value.pinned > 0) continue;
      this.drop(key, value);
    }
    if (this.totalBytes + bytes > this.maxBytes || this.slots.size >= this.maxEntries) {
      return this.refuse(bytes, reason);
    }
    return true;
  }

  private refuse(bytes: number, reason: string): boolean {
    this.degradedFlag = true;
    if (!this.warnedThisChange) {
      this.warnedThisChange = true;
      logWarn("vp", "highlight overlay budget exhausted — degraded selection display", {
        reason,
        wantBytes: bytes,
        bytes: this.totalBytes,
        entries: this.slots.size,
        maxBytes: this.maxBytes,
        maxEntries: this.maxEntries,
      });
    }
    return false;
  }

  /** Insert an owned overlay. Call {@link reserve} first — this does not evict. */
  put(key: string, value: HighlightCacheValue): HighlightCacheValue {
    const existing = this.slots.get(key);
    if (existing) this.drop(key, existing);
    this.slots.set(key, value);
    this.totalBytes += value.bytes;
    this.report();
    return value;
  }

  /**
   * Remove a slot WITHOUT disposing it, handing its buffers to the caller. This
   * is how a growing multi-face selection keeps ONE owned buffer per (body,
   * role): the previous set's slot is taken and rewritten in place under the
   * new key. Refuses a pinned slot — something is still drawing it.
   */
  take(key: string): HighlightCacheValue | undefined {
    const value = this.slots.get(key);
    if (!value || value.pinned > 0) return undefined;
    this.slots.delete(key);
    this.totalBytes -= value.bytes;
    this.report();
    return value;
  }

  pin(key: string): void {
    const value = this.slots.get(key);
    if (value) value.pinned++;
  }

  unpin(key: string): void {
    const value = this.slots.get(key);
    if (value && value.pinned > 0) value.pinned--;
  }

  /** Drop every unpinned slot (a cache clear that respects live display). */
  evictUnpinned(): void {
    for (const [key, value] of [...this.slots]) {
      if (value.pinned === 0) this.drop(key, value);
    }
  }

  /**
   * The source resource is retiring: every overlay cut from it goes, pinned or
   * not. The layer rebuilds against the new resource in the same synchronous
   * step, so nothing renders in between.
   */
  retireSource(entry: MeshEntry): void {
    for (const [key, value] of [...this.slots]) {
      if (value.entry === entry) this.drop(key, value);
    }
  }

  /** Document close / layer teardown: drop and dispose everything. */
  clear(): void {
    for (const [key, value] of [...this.slots]) this.drop(key, value);
    this.degradedFlag = false;
    this.warnedThisChange = false;
  }

  private drop(key: string, value: HighlightCacheValue): void {
    this.slots.delete(key);
    this.totalBytes -= value.bytes;
    // Through the owner where there is one, so the wrapper knows it is spent
    // and a later `update()` on a taken-then-dropped buffer cannot resurrect it.
    if (value.owned) value.owned.dispose();
    else value.geometry.dispose();
    this.report();
  }

  private report(): void {
    reportHighlightCache(this.slots.size, this.totalBytes);
  }
}

/** Canonical ordinal component of a cache key (ascending, `,`-joined). */
export function ordinalsKey(ordinals: readonly number[]): string {
  return [...ordinals].sort((a, b) => a - b).join(",");
}

/** `<identityKey>|<kind>|<ordinals>` — full resource identity + element ordinals. */
export function highlightCacheKey(
  identity: string,
  kind: HighlightCacheKind,
  ordinals: readonly number[],
): string {
  return `${identity}|${kind}|${ordinalsKey(ordinals)}`;
}
