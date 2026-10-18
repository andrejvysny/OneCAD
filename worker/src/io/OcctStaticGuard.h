// OcctStaticGuard.h — RAII guards around OCCT's PROCESS-GLOBAL side channels
// (STEP-IMPORT WP-A W0).
//
// Two distinct hazards, one header:
//
//  1. `Interface_Static` is a process-global registry. Every knob a reader or a
//     writer sets (`read.precision.mode`, `xstep.cascade.unit`, …) stays set for
//     the lifetime of the process and silently changes the behaviour of the NEXT
//     unrelated STEP call. In a long-lived worker that is a cross-request state
//     leak. `InterfaceStaticGuard` snapshots each knob it touches and restores it
//     in its destructor, so the restore runs on every exit path — early return,
//     `Standard_Failure`, or a C++ exception thrown past the scope.
//
//  2. OCCT's default `Message_Messenger` prints through a `Message_PrinterOStream`
//     bound to `std::cout`, i.e. straight onto fd 1 — which for this process is
//     the OCW1 FRAME CHANNEL (protocol/SCHEMA.md §2). `main.cpp` installs a
//     global cerr redirect at startup, but a pure library function must not
//     depend on that having happened (unit-test binaries never run `main.cpp`'s
//     startup sequence — see tests/test_wp6_exportstep.cpp's own local
//     replication of it). `OcctMessengerGuard` installs a cerr-only printer set
//     for the scope and restores the caller's printers afterwards.
//
// ── Why the messenger, and NOT an fd-level (`dup2`) redirect ─────────────────
// A `dup2(2, 1)` guard would also catch C-level chatter, but it would point fd 1
// at stderr PROCESS-WIDE for its duration. The Dispatcher runs the kernel and
// solver lanes concurrently and both write terminal frames to fd 1
// (protocol/Dispatcher.h), so a solver frame emitted while a kernel-lane STEP
// read held such a guard would be written to stderr and LOST — a far worse
// failure than the chatter it prevents. The messenger is the channel OCCT's
// XSControl/STEP stack actually logs through, and swapping printers is
// lane-local in effect. (A `std::cout` rdbuf swap is likewise unavailable here:
// scripts/check-worker-stdout-hygiene.sh forbids the identifier anywhere under
// worker/src, by design.) The guarantee is asserted empirically instead — the
// STEP tests capture fd 1 across the whole read and require zero bytes.
#pragma once

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <mutex>
#include <string>
#include <vector>

#include <Message_SequenceOfPrinters.hxx>

#include "util/Cancel.h"

namespace onecad::io {

// Serializes every use of OCCT's process-global XSControl/STEP state:
// `STEPControl_Controller::Init()`, `Interface_Static` (read/written by the
// guards below AND by the STEP reader/writer internals for the whole transfer)
// and (sidecar: OcctMessengerGuard) the default messenger's printer list. In
// the library the printer list is edited only by worker::route_occt_diagnostics
// under its OWN mutex, so starting a worker never waits on a STEP call that may
// be wedged holding this one. Hold it for the WHOLE STEP call,
// declared before the guards so their restore runs while it is still held.
// In the sidecar it is uncontended (STEP IO runs on the single kernel lane);
// it exists for the in-process library (capi/onecad_worker.h, P0.3 ONECAD-258),
// where two workers in one process crashed with heap corruption inside
// `Interface_Static::SetIVal` without it. Lock order: this BEFORE ocaf_mutex().
// The STEP entry points that take it are leaves: none calls another.
// Taken ONLY through BoundedLock below, never blocking without a deadline.
std::timed_mutex& occt_globals_mutex();

// Prefix of the error message a STEP / XCAF call returns when it could not get
// a process-wide OCCT lock (occt_globals_mutex, ocaf_mutex) within the wait
// bound. Reported as OP_FAILED (SCHEMA §8's closed taxonomy; the session is
// untouched — nothing ran). Only reachable in the in-process library, where
// another worker in the same process can hold the lock; in the sidecar both
// locks are uncontended and the first try_lock always succeeds.
inline constexpr const char* kOcctGlobalsBusy = "OCCT_GLOBALS_BUSY";

// Upper bound on how long one STEP / XCAF call waits for those locks (ms).
// Process-wide; default 20000 — below the 30 s wedge deadline of the shortest
// STEP verb (InspectStep, SCHEMA §8), so a waiting call fails as busy before
// the host would declare it wedged. The library sets it from its config
// (capi/onecad_worker.h); 0 restores the default.
// Inline (like BoundedLock below) so translation units that are linked without
// OcctStaticGuard.cpp — tests/CMakeLists.txt's standalone occt_persistence_codec
// compiles XcafCodec.cpp alone — still link.
inline constexpr std::uint32_t kDefaultOcctLockWaitMs = 20000;

inline std::atomic<std::uint32_t>& occt_lock_wait_storage() {
    static std::atomic<std::uint32_t> ms{kDefaultOcctLockWaitMs};
    return ms;
}

inline void set_occt_lock_wait_ms(std::uint32_t ms) {
    occt_lock_wait_storage().store(ms == 0 ? kDefaultOcctLockWaitMs : ms,
                                   std::memory_order_relaxed);
}

inline std::uint32_t occt_lock_wait_ms() {
    return occt_lock_wait_storage().load(std::memory_order_relaxed);
}

// Acquires `m` with a deadline instead of blocking forever, so a worker that
// wedged INSIDE a STEP call (holding the lock for the rest of the process's
// life) cannot wedge every other in-process worker's kernel lane with it: their
// STEP calls fail as kOcctGlobalsBusy instead. Waits in try_lock_for slices of
// at most 5 ms (a timed_mutex wakes the waiter as soon as the holder unlocks)
// so the wait also honours `cancel`. Uncontended (always, in the sidecar) it is
// one try_lock — the same cost and behaviour as the lock_guard it replaces.
class BoundedLock {
public:
    BoundedLock(std::timed_mutex& m, const onecad::CancelToken* cancel) : m_(m) {
        if (m_.try_lock()) {
            owns_ = true;
            return;
        }
        const auto start = std::chrono::steady_clock::now();
        const auto deadline = start + std::chrono::milliseconds(occt_lock_wait_ms());
        for (;;) {
            if (cancel != nullptr && cancel->cancelled()) {
                cancelled_ = true;
                break;
            }
            const auto now = std::chrono::steady_clock::now();
            if (now >= deadline) break;
            const auto slice = std::min<std::chrono::steady_clock::duration>(
                deadline - now, std::chrono::milliseconds(5));
            if (m_.try_lock_for(slice)) {
                owns_ = true;
                break;
            }
        }
        waited_ms_ = static_cast<std::uint32_t>(
            std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::steady_clock::now() - start)
                .count());
    }
    ~BoundedLock() {
        if (owns_) m_.unlock();
    }
    BoundedLock(const BoundedLock&) = delete;
    BoundedLock& operator=(const BoundedLock&) = delete;

    bool owns() const { return owns_; }
    // True when the wait ended because `cancel` fired (map to CANCELLED).
    bool cancelled() const { return cancelled_; }
    // "<kOcctGlobalsBusy>: <who> could not get <lock> within N ms ...".
    std::string busy_error(const std::string& who, const char* lock_name) const {
        return std::string(kOcctGlobalsBusy) + ": " + who + " could not get " + lock_name +
               " within " + std::to_string(waited_ms_) +
               " ms (another in-process worker holds it; retry, or relaunch if it is wedged)";
    }

private:
    std::timed_mutex& m_;
    bool owns_ = false;
    bool cancelled_ = false;
    std::uint32_t waited_ms_ = 0;
};

// Sets `Interface_Static` knobs and restores their previous values on scope exit.
// A knob that is not registered (the STEP controller was never initialized) is
// recorded as absent and left alone on restore.
//
// NOT thread-safe, because `Interface_Static` itself is not: only one lane may
// hold a guard at a time. V1 has a single kernel lane driving STEP IO.
class InterfaceStaticGuard {
public:
    InterfaceStaticGuard() = default;
    ~InterfaceStaticGuard();

    InterfaceStaticGuard(const InterfaceStaticGuard&) = delete;
    InterfaceStaticGuard& operator=(const InterfaceStaticGuard&) = delete;

    // Snapshot `name` (if not already snapshotted) and assign `value`.
    void set_int(const char* name, int value);
    void set_real(const char* name, double value);
    // Returns whether the knob is registered AND accepted `value`. An enumerated
    // knob (`write.step.schema`) silently keeps its previous value when handed a
    // name it does not know, so a caller whose whole request hinges on the value
    // landing — "export as AP242" — must be able to see the refusal.
    bool set_cstr(const char* name, const std::string& value);

    // Snapshot `name` WITHOUT changing it — for a knob a callee may write.
    void watch_int(const char* name);
    void watch_real(const char* name);
    void watch_cstr(const char* name);

private:
    enum class Kind { Int, Real, Str };
    struct Saved {
        std::string name;
        Kind kind = Kind::Int;
        bool present = false;  // false ⇒ knob unregistered; restore is a no-op
        int ival = 0;
        double rval = 0.0;
        std::string sval;
    };

    // Snapshots `name` once. Returns false when the knob is not registered.
    bool remember(const char* name, Kind kind);

    std::vector<Saved> saved_;
};

// Routes OCCT's default messenger to std::cerr for the scope and restores the
// caller's printer set afterwards. Idempotent to nest (each level restores the
// set it observed). See the header comment for the stdout-hygiene rationale.
// In the in-process library (ONECAD_WORKER_LIBRARY) it is a no-op: the default
// messenger was routed to the Log.h bridge once at worker start
// (worker::route_occt_diagnostics), fd 1 is not a frame channel, and swapping
// the shared printer list per call would race with any other worker in the
// process printing through it.
class OcctMessengerGuard {
public:
    OcctMessengerGuard();
    ~OcctMessengerGuard();

    OcctMessengerGuard(const OcctMessengerGuard&) = delete;
    OcctMessengerGuard& operator=(const OcctMessengerGuard&) = delete;

private:
    Message_SequenceOfPrinters saved_;
};

}  // namespace onecad::io
