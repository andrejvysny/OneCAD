/*
 * Viewport instrumentation counters (VP-HARDENING WP00 step 4, SPEC §25 VP21).
 *
 * Behaviour-neutral observation points around frame stages, mesh resource
 * creation/retirement and pick queries. Nothing here changes what is drawn or
 * picked; it only counts and times. The counters are process-global on purpose:
 * the engine, the mesh registry and the picker are separate modules with no
 * shared owner, and a benchmark or a leak test reads them after the fact.
 *
 * Numbers are what was MEASURED by this process. `renderer.info` remains the
 * renderer's own bookkeeping and is not mirrored here; a resource test reads
 * both and labels them separately (ACCEPTANCE §3.3).
 */

export interface FrameStageSample {
  /** Monotonic submission number the sample belongs to. */
  submission: number;
  /** Milliseconds spent in per-frame layer updates before `renderer.render()`. */
  prepareMs: number;
  /** Milliseconds spent inside `renderer.render()` (CPU side of submission). */
  submitMs: number;
  /** Milliseconds spent in overlay projection + disposal flush after submission. */
  postMs: number;
}

export interface ViewportCounters {
  /** Frames actually submitted to the renderer. */
  framesSubmitted: number;
  /** `invalidate()` calls accepted (engine not disposed). */
  invalidations: number;
  /** Mesh registry entries built (one per body publication accepted). */
  meshEntriesBuilt: number;
  /** Mesh registry entries retired (disposed at a frame boundary or on close). */
  meshEntriesRetired: number;
  /** Estimated bytes of typed-array storage handed to GPU geometry by the registry. */
  meshBytesUploaded: number;
  /** Typed-array bytes currently held by OWNED highlight overlays (WP03 cache). */
  highlightBytesOwned: number;
  /** Owned highlight overlay resources currently cached (WP03 cache). */
  highlightEntries: number;
  /** Registry leases currently open across every borrower (spec §8.2). */
  leasesOpen: number;
  /** Pick queries executed (`raycastAll`), including hover coalesced queries. */
  pickQueries: number;
  /** Total milliseconds spent in pick queries. */
  pickMs: number;
  /** Largest single pick query in milliseconds. */
  pickMaxMs: number;
}

const FRAME_RING = 256;

const counters: ViewportCounters = {
  framesSubmitted: 0,
  invalidations: 0,
  meshEntriesBuilt: 0,
  meshEntriesRetired: 0,
  meshBytesUploaded: 0,
  highlightBytesOwned: 0,
  highlightEntries: 0,
  leasesOpen: 0,
  pickQueries: 0,
  pickMs: 0,
  pickMaxMs: 0,
};

const frameRing: FrameStageSample[] = [];

function nowMs(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

/** Read-only snapshot of the counters. */
export function readViewportCounters(): Readonly<ViewportCounters> {
  return { ...counters };
}

/** The most recent frame-stage samples, oldest first (at most 256). */
export function readFrameStages(): readonly FrameStageSample[] {
  return frameRing.slice();
}

/** Zero every counter and drop the frame ring (tests and benchmark warm-up). */
export function resetViewportCounters(): void {
  for (const k of Object.keys(counters) as (keyof ViewportCounters)[]) counters[k] = 0;
  frameRing.length = 0;
}

export function countInvalidation(): void {
  counters.invalidations++;
}

/**
 * Frame-stage timer. `begin()` before layer updates, `submitted()` immediately
 * after `renderer.render()` returns, `end()` after post-submission work.
 */
export function beginFrame(submission: number): {
  submitStart(): void;
  submitted(): void;
  end(): void;
} {
  const t0 = nowMs();
  let t1 = t0;
  let t2 = t0;
  return {
    submitStart() {
      t1 = nowMs();
    },
    submitted() {
      t2 = nowMs();
    },
    end() {
      const t3 = nowMs();
      counters.framesSubmitted++;
      frameRing.push({ submission, prepareMs: t1 - t0, submitMs: t2 - t1, postMs: t3 - t2 });
      if (frameRing.length > FRAME_RING) frameRing.splice(0, frameRing.length - FRAME_RING);
    },
  };
}

export function countMeshBuilt(bytes: number): void {
  counters.meshEntriesBuilt++;
  counters.meshBytesUploaded += bytes;
}

export function countMeshRetired(n = 1): void {
  counters.meshEntriesRetired += n;
}

/**
 * Owned-highlight cache occupancy (spec §8.3). A LEVEL, not a total: the cache
 * writes its current size after every admission, eviction and retirement, so a
 * resource test can assert the plateau and the return to baseline directly.
 */
export function reportHighlightCache(entries: number, bytes: number): void {
  counters.highlightEntries = entries;
  counters.highlightBytesOwned = bytes;
}

export function countLeaseAcquired(): void {
  counters.leasesOpen++;
}

export function countLeaseReleased(): void {
  counters.leasesOpen--;
}

/** Time one pick query; returns a closure to call when the query is done. */
export function beginPick(): () => void {
  const t0 = nowMs();
  return () => {
    const dt = nowMs() - t0;
    counters.pickQueries++;
    counters.pickMs += dt;
    if (dt > counters.pickMaxMs) counters.pickMaxMs = dt;
  };
}
