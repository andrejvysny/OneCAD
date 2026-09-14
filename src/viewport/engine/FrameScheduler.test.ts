/*
 * FrameScheduler — TEST-LIFE-01 (U): the consume-before-work contract.
 *
 * Every case here is about the ORDER the dirty mask is read and cleared in,
 * because that order is the whole defect (R06): a redraw requested from inside
 * a frame must survive that frame. The scheduler is pure, so this drives it
 * with a fake frame queue and no DOM at all.
 */
import { describe, it, expect, vi } from "vitest";
import {
  DirtyReason,
  FrameScheduler,
  type ConsumedFrame,
  type FrameWork,
} from "./FrameScheduler";

/** A manual frame queue standing in for rAF. */
function harness() {
  const queued: Array<{ id: number; cb: (t: number) => void }> = [];
  const cancelled: number[] = [];
  let nextId = 1;
  const scheduler = new FrameScheduler({
    requestFrame: (cb) => {
      const id = nextId++;
      queued.push({ id, cb });
      return id;
    },
    cancelFrame: (id) => {
      cancelled.push(id);
      const i = queued.findIndex((q) => q.id === id);
      if (i >= 0) queued.splice(i, 1);
    },
  });
  return {
    scheduler,
    queued,
    cancelled,
    /** Run every queued frame once (the callbacks may queue more). */
    flush(t = 16) {
      const due = queued.splice(0, queued.length);
      for (const q of due) q.cb(t);
    },
  };
}

const idleWork: FrameWork = () => ({ changedThisTick: false, stillActive: false });

/**
 * Drive the queue until it drains, BOUNDED — a scheduler that re-queues from
 * inside a failing tick would otherwise spin here forever, and the point of
 * these cases is that the bound is the scheduler's, not the harness's.
 */
function drain(h: ReturnType<typeof harness>, cap = 12): void {
  for (let i = 0; i < cap && h.queued.length > 0; i++) {
    try {
      h.flush();
    } catch {
      // The work's own error; the caller asserts on attempts and the queue.
    }
  }
}

describe("FrameScheduler (TEST-LIFE-01)", () => {
  it("consumes the dirty mask BEFORE the work, so a reentrant invalidate is not lost", () => {
    const { scheduler, queued, flush } = harness();
    const seen: ConsumedFrame[] = [];
    let reentered = false;
    scheduler.setWork((consumed) => {
      seen.push(consumed);
      // The mask must already be clear at this point — this is the invariant.
      expect(scheduler.dirtyMask).toBe(0);
      if (!reentered) {
        reentered = true;
        scheduler.invalidate(DirtyReason.overlay);
      }
      return { changedThisTick: true, stillActive: false };
    });

    scheduler.invalidate(DirtyReason.geometry);
    expect(queued).toHaveLength(1);

    flush(); // frame 1 renders and re-invalidates from inside
    expect(seen).toHaveLength(1);
    expect(seen[0].reasons).toBe(DirtyReason.geometry);
    expect(queued).toHaveLength(1); // exactly one more frame

    flush(); // frame 2 answers the reentrant request
    expect(seen).toHaveLength(2);
    expect(seen[1].reasons).toBe(DirtyReason.overlay);

    expect(queued).toHaveLength(0); // …then idle
    flush();
    expect(seen).toHaveLength(2);
  });

  it("coalesces reasons and never keeps two frames pending", () => {
    const { scheduler, queued, flush } = harness();
    const seen: ConsumedFrame[] = [];
    scheduler.setWork((consumed) => {
      seen.push(consumed);
      return { changedThisTick: true, stillActive: false };
    });

    scheduler.invalidate(DirtyReason.camera);
    scheduler.invalidate(DirtyReason.appearance);
    scheduler.invalidate(DirtyReason.camera);
    expect(queued).toHaveLength(1);
    expect(scheduler.pending).toBe(true);

    flush();
    expect(seen).toHaveLength(1);
    expect(seen[0].reasons).toBe(DirtyReason.camera | DirtyReason.appearance);
  });

  it("carries a monotonic requestedRevision and never rewinds it", () => {
    const { scheduler, flush } = harness();
    const revisions: number[] = [];
    scheduler.setWork((consumed) => {
      revisions.push(consumed.requestedRevision);
      return { changedThisTick: true, stillActive: false };
    });

    scheduler.invalidate(DirtyReason.camera);
    scheduler.invalidate(DirtyReason.camera);
    flush();
    scheduler.invalidate(DirtyReason.geometry);
    flush();

    expect(revisions).toEqual([2, 3]);
    expect(scheduler.requestedRevision).toBe(3);
  });

  it("reschedules for stillActive but NOT for a tween that just ended", () => {
    const { scheduler, queued, flush } = harness();
    let ticks = 0;
    scheduler.setWork(() => {
      ticks++;
      // Two animating ticks, then the tween ends: changed, but no longer active.
      return { changedThisTick: true, stillActive: ticks < 2 };
    });

    scheduler.invalidate(DirtyReason.camera);
    flush(); // tick 1 — still animating
    expect(queued).toHaveLength(1);
    flush(); // tick 2 — the tween ended on this very frame
    expect(ticks).toBe(2);
    expect(queued).toHaveLength(0); // no extra idle frame
  });

  it("suspended: schedules nothing, and resuming schedules one frame iff dirty", () => {
    const { scheduler, queued, cancelled, flush } = harness();
    const work = vi.fn(idleWork);
    scheduler.setWork(work);

    scheduler.invalidate(DirtyReason.geometry);
    expect(queued).toHaveLength(1);
    scheduler.setSuspended(true);
    expect(queued).toHaveLength(0); // the pending frame was cancelled …
    expect(cancelled).toHaveLength(1);

    scheduler.invalidate(DirtyReason.appearance);
    expect(queued).toHaveLength(0); // … and no new one is taken while suspended

    scheduler.setSuspended(false);
    expect(queued).toHaveLength(1); // dirty → exactly one
    flush();
    expect(work).toHaveBeenCalledTimes(1);
    // The reasons raised while suspended survived the suspension.
    expect(work.mock.calls[0][0].reasons).toBe(DirtyReason.geometry | DirtyReason.appearance);

    scheduler.setSuspended(true);
    scheduler.setSuspended(false);
    expect(queued).toHaveLength(0); // clean → resuming schedules nothing
  });

  it("a tick that lands while suspended runs no work and consumes nothing", () => {
    const { scheduler, queued } = harness();
    const work = vi.fn(idleWork);
    scheduler.invalidate(DirtyReason.geometry);
    const pending = queued[0];
    scheduler.setSuspended(true);

    pending.cb(16); // the browser fired the already-scheduled frame anyway
    expect(work).not.toHaveBeenCalled();
    expect(scheduler.dirtyMask).toBe(DirtyReason.geometry); // NOT consumed
    expect(scheduler.pending).toBe(false);
  });

  it("dispose() cancels the pending frame and refuses every later invalidate", () => {
    const { scheduler, queued, cancelled, flush } = harness();
    const work = vi.fn(idleWork);
    scheduler.setWork(work);

    scheduler.invalidate(DirtyReason.camera);
    expect(queued).toHaveLength(1);
    scheduler.dispose();
    expect(cancelled).toHaveLength(1);
    expect(queued).toHaveLength(0);
    expect(scheduler.disposed).toBe(true);

    scheduler.invalidate(DirtyReason.camera);
    expect(queued).toHaveLength(0);
    flush();
    expect(work).not.toHaveBeenCalled();
    scheduler.dispose(); // idempotent
    expect(cancelled).toHaveLength(1);
  });

  it("a throwing work restores its reasons and still runs the reschedule tail (N3)", () => {
    const { scheduler, queued, flush } = harness();
    let throwNext = true;
    scheduler.setWork(() => {
      if (throwNext) {
        throwNext = false;
        throw new Error("a layer blew up mid-frame");
      }
      return { changedThisTick: true, stillActive: false };
    });

    scheduler.invalidate(DirtyReason.geometry);
    // The error is not swallowed — an invisible frame failure is worse.
    expect(() => flush()).toThrow("a layer blew up mid-frame");
    // …but the redraw it was answering is NOT lost, and a frame is queued.
    expect(scheduler.dirtyMask).toBe(DirtyReason.geometry);
    expect(queued).toHaveLength(1);

    flush(); // the retry succeeds and consumes the restored reason
    expect(scheduler.dirtyMask).toBe(0);
    expect(queued).toHaveLength(0);
  });

  it("parks a work that throws on alternate ticks while re-invalidating (windowed bound, R1(b))", () => {
    const { scheduler, queued, flush } = harness();
    let attempts = 0;
    scheduler.setWork(() => {
      attempts++;
      scheduler.invalidate(DirtyReason.camera);
      if (attempts % 2 === 1) throw new Error("odd");
      return { changedThisTick: true, stillActive: false };
    });
    scheduler.invalidate(DirtyReason.geometry);
    for (let i = 0; i < 40 && queued.length > 0; i++) {
      try { flush(); } catch { /* the odd ticks throw by design */ }
    }
    // Three throws inside the eight-tick window: attempts 1, 3, 5 → parked at 5.
    expect(attempts).toBe(5);
    expect(scheduler.parked).toBe(true);
    expect(queued).toHaveLength(0);
  });

  it("dispose() from inside the work leaves no transition reported running", () => {
    const { scheduler, queued, flush } = harness();
    scheduler.setWork(() => {
      scheduler.dispose();
      return { changedThisTick: true, stillActive: true };
    });
    scheduler.invalidate(DirtyReason.camera);
    flush();
    expect(scheduler.disposed).toBe(true);
    expect(scheduler.transitionRunning).toBe(false);
    expect(queued).toHaveLength(0);
  });

  it("stops rescheduling after three consecutive throwing ticks, but invalidate still works", () => {
    const { scheduler, queued, flush } = harness();
    scheduler.setWork(() => {
      throw new Error("always");
    });

    scheduler.invalidate(DirtyReason.geometry);
    for (let i = 0; i < 3; i++) expect(() => flush()).toThrow("always");
    expect(queued).toHaveLength(0); // no permanent rAF loop (guide §22)

    // The engine is still schedulable — this is a bound, not a shutdown.
    scheduler.invalidate(DirtyReason.camera);
    expect(queued).toHaveLength(1);
  });

  /*
   * PR-05. A tick that invalidates and THEN throws used to queue its own
   * successor from inside the work: `invalidate()` called `ensureFrame()`
   * immediately, so the error bound below — which only withholds the TAIL's
   * `ensureFrame()` — never emptied the queue. The probe measured 12 attempts
   * (harness cap) with a frame still queued. An in-tick request is now only a
   * mask write; the tail decides, and a park cancels.
   */
  it("a work that invalidates and then throws parks after three attempts, queue empty (PR-05)", () => {
    const h = harness();
    const { scheduler, queued, flush } = h;
    let attempts = 0;
    scheduler.setWork(() => {
      attempts++;
      scheduler.invalidate(DirtyReason.camera); // a layer asks for a redraw…
      throw new Error("layer blew up"); // …and then fails
    });

    scheduler.invalidate(DirtyReason.geometry);
    for (let i = 0; i < 12 && queued.length > 0; i++) {
      expect(() => flush()).toThrow("layer blew up");
    }

    expect(attempts).toBe(3);
    expect(queued).toHaveLength(0);
    expect(scheduler.pending).toBe(false);
    expect(scheduler.parked).toBe(true);
    // The request itself survives, for an EXTERNAL wake to answer.
    expect(scheduler.dirtyMask).toBe(DirtyReason.geometry | DirtyReason.camera);
  });

  it("a throwing work that does NOT reinvalidate also stops at three, queue empty", () => {
    const h = harness();
    let attempts = 0;
    h.scheduler.setWork(() => {
      attempts++;
      throw new Error("always");
    });

    h.scheduler.invalidate(DirtyReason.geometry);
    drain(h);

    expect(attempts).toBe(3);
    expect(h.queued).toHaveLength(0);
    expect(h.scheduler.parked).toBe(true);
  });

  it("a healthy work that reinvalidates once runs exactly two ticks and then idles", () => {
    const h = harness();
    let attempts = 0;
    h.scheduler.setWork(() => {
      attempts++;
      if (attempts === 1) h.scheduler.invalidate(DirtyReason.overlay);
      return { changedThisTick: true, stillActive: false };
    });

    h.scheduler.invalidate(DirtyReason.geometry);
    drain(h);

    expect(attempts).toBe(2);
    expect(h.queued).toHaveLength(0);
    expect(h.scheduler.parked).toBe(false);
  });

  it("a parked scheduler answers an EXTERNAL invalidate with exactly one attempt", () => {
    const h = harness();
    let attempts = 0;
    h.scheduler.setWork(() => {
      attempts++;
      h.scheduler.invalidate(DirtyReason.camera);
      throw new Error("layer blew up");
    });

    h.scheduler.invalidate(DirtyReason.geometry);
    drain(h);
    expect(attempts).toBe(3);

    // One attempt per external wake — the bound is per wake, not a shutdown.
    h.scheduler.invalidate(DirtyReason.camera);
    expect(h.queued).toHaveLength(1);
    drain(h);
    expect(attempts).toBe(4);
    expect(h.queued).toHaveLength(0);
    expect(h.scheduler.parked).toBe(true);
  });

  it("resetErrors() makes a parked scheduler behave like a fresh one", () => {
    const h = harness();
    let attempts = 0;
    h.scheduler.setWork(() => {
      attempts++;
      h.scheduler.invalidate(DirtyReason.camera);
      throw new Error("layer blew up");
    });

    h.scheduler.invalidate(DirtyReason.geometry);
    drain(h);
    expect(attempts).toBe(3);

    h.scheduler.resetErrors(); // an explicit recovery: retryRenderer / context restore
    expect(h.scheduler.parked).toBe(false);
    h.scheduler.invalidate(DirtyReason.geometry);
    drain(h);
    expect(attempts).toBe(6); // three more before it parks again
    expect(h.queued).toHaveLength(0);
    expect(h.scheduler.parked).toBe(true);
  });

  it("an invalidate raised inside a tick that then suspends queues nothing until resume", () => {
    const { scheduler, queued, flush } = harness();
    let ticks = 0;
    scheduler.setWork(() => {
      ticks++;
      scheduler.invalidate(DirtyReason.overlay);
      scheduler.setSuspended(true); // the context went away mid-frame
      return { changedThisTick: true, stillActive: false };
    });

    scheduler.invalidate(DirtyReason.geometry);
    flush();
    expect(ticks).toBe(1);
    expect(queued).toHaveLength(0);

    scheduler.setSuspended(false);
    expect(queued).toHaveLength(1); // the in-frame request survived the suspension
  });

  it("dispose() from inside the work queues nothing and refuses every later invalidate", () => {
    const { scheduler, queued, flush } = harness();
    let ticks = 0;
    scheduler.setWork(() => {
      ticks++;
      scheduler.invalidate(DirtyReason.overlay);
      scheduler.dispose(); // an after-render listener tore the viewport down
      return { changedThisTick: true, stillActive: false };
    });

    scheduler.invalidate(DirtyReason.geometry);
    flush();
    expect(ticks).toBe(1);
    expect(queued).toHaveLength(0);

    scheduler.invalidate(DirtyReason.camera);
    expect(queued).toHaveLength(0);
    expect(scheduler.disposed).toBe(true);
  });

  it("resuming mid-transition schedules a frame even with a clean mask (N4)", () => {
    const { scheduler, queued, flush } = harness();
    scheduler.setWork(() => ({ changedThisTick: true, stillActive: true }));

    scheduler.invalidate(DirtyReason.camera);
    flush(); // one tick of a running tween
    expect(scheduler.transitionRunning).toBe(true);
    expect(queued).toHaveLength(1);

    scheduler.setSuspended(true); // the context went away mid-tween
    expect(queued).toHaveLength(0);
    expect(scheduler.dirtyMask).toBe(0); // the mask is clean — only the tween is live

    scheduler.setSuspended(false);
    expect(queued).toHaveLength(1); // the transition still needs its next step
  });

  it("tick(now, work) passes the frame timestamp through to the work", () => {
    const { scheduler, flush } = harness();
    const seen: number[] = [];
    scheduler.setWork((consumed) => {
      seen.push(consumed.now);
      return { changedThisTick: true, stillActive: false };
    });
    scheduler.invalidate(DirtyReason.camera);
    flush(1234.5);
    expect(seen).toEqual([1234.5]);
  });
});
