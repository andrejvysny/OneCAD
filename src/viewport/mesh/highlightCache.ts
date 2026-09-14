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
 * ADMISSION IS A RECEIPT: `reserve` prices what the caller promises to
 * allocate and `put` refuses anything larger. The old accounting reserved a
 * per-triangle ESTIMATE and then charged the ×1.5-grown capacity after the
 * ceiling had already been checked, so a growing selection could push the
 * occupancy past the advertised bound (finding PR-04). Callers price with
 * `planFaceSetCapacity`, whose `peakBytes` also covers the outgoing buffer.
 *
 * DEGRADED MODE: if the pinned set alone exceeds the budget, the semantic
 * selection is NEVER discarded. The cache refuses the reservation, raises
 * `degraded`, and emits ONE diagnostic per selection change; HighlightLayer
 * then draws that body's leased edge outline plus a selection count, which owns
 * no buffers at all. An ordinary single-face hover always fits, because
 * unpinned entries are evicted to make room for it first.
 */
import type * as THREE from "three";
import { logError, logWarn } from "@/debug/log";
import { reportHighlightCache } from "../vph/instrumentation";
import type { MeshEntry } from "./meshRegistry";
import type { OwnedFaceGeometry } from "./faceSliceGeometry";

export const HIGHLIGHT_CACHE_MAX_BYTES = 64 * 1024 * 1024;
export const HIGHLIGHT_CACHE_MAX_ENTRIES = 256;

/**
 * A successful {@link HighlightCache.reserve}. Carries the byte budget the
 * caller was granted; {@link HighlightCache.put} admits nothing larger.
 */
export interface CacheReservation {
  /**
   * Budget bytes held. For a face set this is the PEAK — the outgoing buffer is
   * still allocated while the new one is filled — which is roughly twice the
   * resting size after a growth step.
   */
  readonly bytes: number;
  /**
   * The exact resting bytes the caller's plan predicted, when it can predict
   * them. {@link HighlightCache.put} admits only a value that matches, so a
   * build that disagrees with its plan is caught instead of slipping through
   * on the peak's slack (decision D5).
   */
  readonly expectedBytes?: number;
}

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
   * Returns null when the pinned set alone still exceeds the budget: the caller
   * must then take the documented degraded path rather than allocate.
   *
   * The returned RECEIPT is what {@link put} admits against, so a caller that
   * allocates more than it priced is refused instead of quietly pushing the
   * occupancy over the ceiling (finding PR-04).
   */
  reserve(bytes: number, reason: string, expectedBytes?: number): CacheReservation | null {
    if (bytes > this.maxBytes) return this.refuse(bytes, reason);
    for (const [key, value] of this.slots) {
      if (this.totalBytes + bytes <= this.maxBytes && this.slots.size < this.maxEntries) break;
      if (value.pinned > 0) continue;
      this.drop(key, value);
    }
    if (this.totalBytes + bytes > this.maxBytes || this.slots.size >= this.maxEntries) {
      return this.refuse(bytes, reason);
    }
    return { bytes, expectedBytes };
  }

  private refuse(bytes: number, reason: string): null {
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
    return null;
  }

  /**
   * Insert an owned overlay against the receipt {@link reserve} issued. Does
   * not evict — and REFUSES a value that does not match its reservation,
   * because the budget was checked against the receipt and nothing else has
   * made room for a difference. The caller owns the geometry it built and must
   * dispose it, then take the degraded path.
   *
   * Every refusal is NON-DESTRUCTIVE: the decision is taken before the slot
   * already under this key is touched, and a PINNED slot is never replaced at
   * all — something is drawing it, and both dropping it and silently
   * overwriting the reference to it are ways of losing live geometry.
   */
  put(
    key: string,
    value: HighlightCacheValue,
    receipt: CacheReservation,
  ): HighlightCacheValue | undefined {
    const existing = this.slots.get(key);
    // Only an unpinned same-key slot is about to be freed, so only its bytes
    // may be counted as headroom for the ceiling check.
    const freed = existing && existing.pinned === 0 ? existing.bytes : 0;
    const mismatch =
      receipt.expectedBytes === undefined
        ? value.bytes > receipt.bytes
        : value.bytes !== receipt.expectedBytes;
    if (mismatch || this.totalBytes - freed + value.bytes > this.maxBytes) {
      this.degradedFlag = true;
      logError("vp", "highlight overlay does not match its reservation — refused", {
        key,
        kind: value.kind,
        reservedBytes: receipt.bytes,
        plannedBytes: receipt.expectedBytes,
        actualBytes: value.bytes,
        bytes: this.totalBytes,
        maxBytes: this.maxBytes,
      });
      return undefined;
    }
    if (existing && existing.pinned > 0) {
      this.degradedFlag = true;
      logError("vp", "highlight cache slot is displayed — refused", {
        key,
        kind: value.kind,
        pinned: existing.pinned,
      });
      return undefined;
    }
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
