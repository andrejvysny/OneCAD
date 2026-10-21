// OcctBusy.h — the one rule for a STEP / XCAF call that was refused because a
// process-wide OCCT lock (OcctStaticGuard.h: occt_globals_mutex, ocaf_mutex)
// stayed busy past its bounded wait.
//
// Only reachable in the in-process library, where a sibling worker in the same
// process can hold the lock (capi/onecad_worker.h CONCURRENCY); in the sidecar
// both locks are uncontended. Nothing ran when a call is refused, so the refusal
// is RETRIABLE everywhere it surfaces — a verb's terminal, or a whole ExecutePlan
// / PreviewPlan / PreviewOp (never a per-step `opFailed` a document would record
// as a model error). The message keeps BoundedLock::busy_error's text FIRST, so
// every refusal starts with kOcctGlobalsBusy — the prefix contract a host tests.
#pragma once

#include <cstdint>
#include <string>
#include <string_view>

#include "io/OcctStaticGuard.h"
#include "protocol/Envelope.h"

namespace onecad::io {

// True iff `error` is a busy refusal (prefix contract).
inline bool is_occt_busy(std::string_view error) {
    return error.substr(0, std::string_view(kOcctGlobalsBusy).size()) == kOcctGlobalsBusy;
}

// `busy` (a BoundedLock::busy_error text) with the refused verb/op appended as
// context, so the prefix stays first. An empty context returns `busy` as is.
inline std::string busy_message(const std::string& busy, const std::string& context) {
    return context.empty() ? busy : busy + " (" + context + ")";
}

// The terminal for a busy refusal: OP_FAILED, retriable (SCHEMA §8 keeps its
// closed code set; `retriable` is what tells a caller to try again).
inline protocol::Envelope busy_response(std::uint64_t id, const std::string& busy,
                                        const std::string& context = {}) {
    return protocol::Envelope::error_response(
        id, protocol::ErrorInfo{"OP_FAILED", busy_message(busy, context), /*retriable=*/true});
}

}  // namespace onecad::io
