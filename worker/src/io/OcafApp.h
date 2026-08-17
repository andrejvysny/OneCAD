// OcafApp.h — the ONE OCAF application the worker uses, and its lock.
//
// ── Why a shared singleton ──────────────────────────────────────────────────
// `TDocStd_Application` owns a document SESSION: every `NewDocument`/`Open`
// registers the document with the application until `Close`. Two applications
// would mean two sessions over one process, and a document opened on one could
// never be closed on the other. One instance, one place that binds the BinXCAF
// drivers.
//
// ── Why it is NOT just `new TDocStd_Application()` ──────────────────────────
// STDOUT HYGIENE (SCHEMA §2 — fd 1 carries OCW1 frames and nothing else).
// `TDocStd_Application` does NOT use `Message::DefaultMessenger()`: it owns its own
// `Message_Messenger` (`CDM_Application::MessageDriver()`), whose default printer
// writes to **std::cout**. So main.cpp's `redirect_occt_to_stderr()` — which only
// re-points the DEFAULT messenger — does not cover this path, and any malformed or
// truncated `.xbf` would make the retrieval driver print
// `"0x… : Storage_StreamTypeMismatchError"` straight into the middle of a frame.
// Measured, not theorised: 58 bytes on fd 1 per failed open before this redirect
// existed. `ocaf_application()` re-points that messenger at stderr exactly once.
//
// ── Why a lock ──────────────────────────────────────────────────────────────
// The session is process-global mutable state. `InspectStep` (conversion) and the
// `ImportStep` op (replay) are both dispatched on the kernel lane and therefore
// never overlap today, but a future lane split must not turn that into a data race.
#pragma once

#include <mutex>

#include <TDocStd_Application.hxx>
#include <TDocStd_Document.hxx>

namespace onecad::io {

// The shared application: BinXCAF drivers bound, message driver on stderr.
Handle(TDocStd_Application) ocaf_application();

// Guards every use of the application's document session.
std::mutex& ocaf_mutex();

// RAII close for a document opened on `ocaf_application()`. A document left open
// stays registered in the application's SESSION and keeps its whole shape tree
// alive for the life of the process, so every exit path — early return,
// `Standard_Failure`, a C++ exception thrown past the scope — must close it.
//
// Lives here rather than in one .cpp because both the xbf codec and the STEP
// exporter open documents on the same application.
class OcafDocGuard {
public:
    explicit OcafDocGuard(const Handle(TDocStd_Document) & doc) : doc_(doc) {}
    ~OcafDocGuard();

    OcafDocGuard(const OcafDocGuard&) = delete;
    OcafDocGuard& operator=(const OcafDocGuard&) = delete;

private:
    Handle(TDocStd_Document) doc_;
};

}  // namespace onecad::io
