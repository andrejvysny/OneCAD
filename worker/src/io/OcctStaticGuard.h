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

#include <string>
#include <vector>

#include <Message_SequenceOfPrinters.hxx>

namespace onecad::io {

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
    void set_cstr(const char* name, const std::string& value);

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
