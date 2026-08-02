// InspectStep.h — the `InspectStep` verb (SCHEMA §7.8, STEP-IMPORT WP-A W1).
//
// A READ-ONLY preflight probe over a STEP file at a Rust-provided temp path. It
// takes no `Session`: like the §7.5 identity verbs it touches no head, no scratch
// and no fence, and — unlike the removed `ImportStep` verb it replaces — publishes
// nothing. The import itself is the §7.3 `ImportStep` OP, so that it lives on the
// timeline and survives full-replay regen.
//
// Two jobs:
//   * PROBE — `solidCount`, `sourceUnit`, `bbox`, `productNames`, `brepFormat`,
//     `diagnostics`: enough for the UI to describe the file before committing an
//     op to the document.
//   * CONVERT (`includeBrep:true`) — the healed, mm-normalized, UNSCALED result as
//     one BinTools compound in the response's binary tail under section "brep",
//     solids in the §7.3 ordinal order. This is what makes `sourceCodec:"brep"`
//     replay possible: Rust probes once at import-command time, persists the brep
//     bytes next to the STEP source, and authors the record against the brep.
//
// A malformed file is an `OP_FAILED`-class error response (recoverable), never a
// PROTOCOL_ERROR — a hostile file must not tear the worker down. Cancel is
// honored through `io::read_step`.
#pragma once

#include "protocol/Envelope.h"
#include "util/Cancel.h"

namespace onecad::io {

protocol::Envelope handle_inspect_step(const protocol::Envelope& req,
                                       const onecad::CancelToken& cancel);

}  // namespace onecad::io
