/*
 * FrameScheduler — the ONE place a viewport frame is scheduled and consumed
 * (VP-HARDENING VP02, spec §6; algorithm NUM §11.1).
 *
 * Pure: no THREE, no DOM, no timers. The owner supplies `requestFrame` /
 * `cancelFrame` (rAF in the engine, a manual harness in tests) and the frame
 * work; everything else here is bookkeeping.
 *
 * THE DEFECT THIS FIXES (R06). The previous loop was
 *
 *     tick() { if (dirty) { renderFrame(); dirty = false; } }
 *
 * — it cleared the dirty flag AFTER the frame work. Anything that invalidated
 * DURING the frame (a contribution's frame hook, an after-render listener, an
 * overlay layout) had its request erased by the very frame it was raised in:
 * the rAF it scheduled then ran, found `dirty === false`, and drew nothing. The
 * redraw was silently lost.
 *
 * The rule is therefore: SNAPSHOT AND CLEAR the dirty mask BEFORE calling the
 * work function. Anything raised inside the work belongs to the NEXT frame, and
 * the tail of `tick` schedules it.
 *
 * The second rule is: at most ONE frame may be outstanding, and an idle engine
 * has none. There is no permanent rAF and there is never a second timer — a
 * loop that runs while nothing changed hides exactly the bug above (guide §22).
 */

/**
 * Why a frame is dirty. A bitset, not an enum: a single frame routinely
 * coalesces several reasons, and the frame work needs to know all of them
 * (a `resize` frame must re-derive screen metrics; an `appearance` frame need
 * not).
 */
export const DirtyReason = {
  /** Camera moved (orbit/pan/zoom/tween/projection swap). */
  camera: 1,
  /** Scene content changed — a body published, a sketch rebuilt, a preview. */
  geometry: 2,
  /** Colors, materials, visibility, highlight or theme change. */
  appearance: 4,
  /** HTML overlay / chip layout only. */
  overlay: 8,
  /** CSS box or device pixel ratio changed. */
  resize: 16,
  /** A quality/refinement step (LOD, one-shot refinement) asked for a repaint. */
  quality: 32,
  /** Context restored or renderer retried — repaint from a recovered state. */
  recovery: 64,
} as const;

/** An OR of {@link DirtyReason} values. `0` means "nothing is dirty". */
export type DirtyReasonMask = number;

/** No reason set. */
export const NO_REASON: DirtyReasonMask = 0;

/** What one tick consumed, handed to the frame work verbatim. */
export interface ConsumedFrame {
  /** The reasons accumulated since the last tick, already cleared from the scheduler. */
  readonly reasons: DirtyReasonMask;
  /** The newest `invalidate()` revision this frame answers. Monotonic. */
  readonly requestedRevision: number;
  /** The frame timestamp the owner's `requestFrame` callback was given. */
  readonly now: number;
}

/** What the frame work reports back. The two flags are deliberately distinct. */
export interface FrameWorkResult {
  /**
   * The tick actually did frame work (rendered). Reported for the owner and for
   * tests; it does NOT drive rescheduling — that is `stillActive` plus whatever
   * the work re-raised. Keeping them apart is what avoids one extra idle frame
   * when a tween finishes (NUM §11.1).
   */
  readonly changedThisTick: boolean;
  /** A transition is still running, so another frame is needed regardless of the mask. */
  readonly stillActive: boolean;
}

export type FrameWork = (consumed: ConsumedFrame) => FrameWorkResult;

export interface FrameSchedulerDeps {
  /** Schedule one frame; returns a handle for {@link FrameSchedulerDeps.cancelFrame}. */
  requestFrame(cb: (t: number) => void): number;
  cancelFrame(id: number): void;
}

const IDLE: FrameWorkResult = { changedThisTick: false, stillActive: false };

/**
 * Consecutive ticks whose work THREW before the scheduler stops rescheduling.
 *
 * A throwing work is a defect, not a state: the engine's own frame work guards
 * every layer and wraps the draw call, so nothing routine reaches here. The
 * bound exists because the recovery below (restore the reasons, then run the
 * normal tail) would otherwise re-queue a frame that throws again — the
 * permanent redraw loop guide §22 rejects. An `invalidate()` still schedules.
 *
 * The bound is per EXTERNAL wake, not a shutdown: parking cancels the queue and
 * keeps the mask, and the next `invalidate()` from outside a tick buys exactly
 * one more attempt. {@link FrameScheduler.resetErrors} is the explicit recovery.
 */
const WORK_ERROR_LIMIT = 3;

/**
 * The bound is counted over a WINDOW of recent ticks, not only consecutive
 * ones: a work that throws on every other tick while re-invalidating would
 * otherwise reset the count on each good tick and loop forever (R1(b)). Eight
 * ticks is long enough that three failures inside it is never routine.
 */
const WORK_ERROR_WINDOW = 8;

export class FrameScheduler {
  private readonly deps: FrameSchedulerDeps;
  /** The owner's frame work, run by a scheduled frame. See {@link setWork}. */
  private work: FrameWork | null = null;
  private frameId = 0;
  private scheduled = false;
  private suspended = false;
  private destroyed = false;
  private mask: DirtyReasonMask = NO_REASON;
  private revision = 0;
  /** `stillActive` from the last tick — a transition suspend/resume must survive. */
  private transitionActive = false;
  /** Outcomes of the last {@link WORK_ERROR_WINDOW} ticks, newest in bit 0; a set bit is a throw. */
  private recentThrows = 0;
  /** The last tick's tail hit {@link WORK_ERROR_LIMIT} and queued nothing. */
  private isParked = false;
  /**
   * True only while the work function is running. An `invalidate()` raised in
   * that window may NOT take a frame of its own (PR-05): the tail owns the
   * decision, and a tick that queued its own successor before failing left a
   * frame the error bound could not withhold — a permanent loop.
   */
  private inTick = false;

  constructor(deps: FrameSchedulerDeps) {
    this.deps = deps;
  }

  /**
   * Register what a scheduled frame runs. Set once by the owner; a test that
   * drives {@link tick} directly can pass its work there instead and skip this.
   */
  setWork(work: FrameWork): void {
    this.work = work;
  }

  /** A frame is outstanding. */
  get pending(): boolean {
    return this.scheduled;
  }

  /** The reasons accumulated but not yet consumed. */
  get dirtyMask(): DirtyReasonMask {
    return this.mask;
  }

  /** Monotonic count of accepted `invalidate()` calls — the frame revision asked for. */
  get requestedRevision(): number {
    return this.revision;
  }

  get disposed(): boolean {
    return this.destroyed;
  }

  /** Frames are suppressed (context lost, or the owner is not ready yet). */
  get isSuspended(): boolean {
    return this.suspended;
  }

  /** The last tick reported a transition still running. */
  get transitionRunning(): boolean {
    return this.transitionActive;
  }

  /**
   * The error bound tripped on the last tick: nothing is queued and the mask is
   * held for an external wake. Cleared by the next non-throwing tick or by
   * {@link resetErrors}.
   */
  get parked(): boolean {
    return this.isParked;
  }

  /**
   * Forget the consecutive-failure run — an EXPLICIT recovery said so
   * (`retryRenderer()`, a restored GL context). The scheduler is then exactly
   * as forgiving as a fresh one.
   */
  resetErrors(): void {
    this.recentThrows = 0;
    this.isParked = false;
  }

  /** Throwing ticks inside the window. */
  private get workErrors(): number {
    let n = 0;
    for (let bits = this.recentThrows; bits !== 0; bits >>>= 1) n += bits & 1;
    return n;
  }

  /** Mark the frame dirty for `reason` and ensure exactly one frame is pending. */
  invalidate(reason: DirtyReasonMask): void {
    if (this.destroyed) return;
    this.revision += 1;
    this.mask |= reason;
    // Inside a tick this is only a mask write: the tail schedules (or parks).
    if (!this.inTick) this.ensureFrame();
  }

  /**
   * Stop (or resume) scheduling. Used while the GL context is lost and before
   * the engine has a renderer: no frame may be submitted into a dead context.
   * Resuming schedules one frame if, and only if, something is still dirty.
   */
  setSuspended(value: boolean): void {
    if (this.destroyed || this.suspended === value) return;
    this.suspended = value;
    if (value) {
      this.cancelPending();
    } else if (this.mask !== NO_REASON || this.transitionActive) {
      // A transition that was running when the context went away still needs a
      // frame on resume: its next step is not in the dirty mask.
      this.ensureFrame();
    }
  }

  /**
   * Run one frame. NUM §11.1 verbatim: clear `scheduled`, bail when disposed or
   * suspended, snapshot AND CLEAR the dirty mask, run the work, then reschedule
   * if the work re-dirtied the mask or a transition is still running.
   */
  tick(now: number, work: FrameWork): void {
    this.scheduled = false;
    this.frameId = 0;
    if (this.destroyed || this.suspended) return;

    // Consume BEFORE the callbacks — the whole point of this class.
    const consumed: ConsumedFrame = {
      reasons: this.mask,
      requestedRevision: this.revision,
      now,
    };
    this.mask = NO_REASON;

    let result = IDLE;
    let threw = false;
    this.inTick = true;
    try {
      result = work(consumed);
    } catch (error) {
      // A throwing work must not SWALLOW the redraw it was answering: put the
      // reasons back so the request survives, then let the error out so it is
      // visible. `finally` still runs the normal tail.
      threw = true;
      this.mask |= consumed.reasons;
      throw error;
    } finally {
      this.inTick = false;
      this.recentThrows = ((this.recentThrows << 1) | (threw ? 1 : 0)) & ((1 << WORK_ERROR_WINDOW) - 1);
      this.isParked = threw && this.workErrors >= WORK_ERROR_LIMIT;
      // A `dispose()` raised inside the work has already cleared everything;
      // the tail must not resurrect a transition on a dead scheduler.
      if (!this.destroyed) this.transitionActive = result.stillActive;
      if (this.destroyed) {
        // nothing to schedule
      } else if (this.isParked) {
        // Defensive: nothing may be queued from inside a tick any more, but a
        // park must leave the queue EMPTY whatever the work managed to do.
        this.cancelPending();
      } else if (this.mask !== NO_REASON || result.stillActive) {
        this.ensureFrame();
      }
    }
  }

  /** Cancel any pending frame and refuse every later one. */
  dispose(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    this.cancelPending();
    this.mask = NO_REASON;
    this.transitionActive = false;
    this.work = null;
  }

  private ensureFrame(): void {
    if (this.destroyed || this.suspended || this.scheduled) return;
    this.scheduled = true;
    this.frameId = this.deps.requestFrame(this.handleFrame);
  }

  private cancelPending(): void {
    if (!this.scheduled) return;
    this.scheduled = false;
    this.deps.cancelFrame(this.frameId);
    this.frameId = 0;
  }

  private readonly handleFrame = (now: number): void => {
    this.tick(now, this.work ?? (() => IDLE));
  };
}
