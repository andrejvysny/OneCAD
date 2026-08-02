// InspectStep.h — the `InspectStep` verb (SCHEMA §7.8, STEP-IMPORT WP-A W1,
// codec-neutral rename + XCAF fidelity W4.5).
//
// A READ-ONLY preflight probe over a STEP file at a Rust-provided temp path. It
// takes no `Session`: like the §7.5 identity verbs it touches no head, no scratch
// and no fence, and — unlike the removed `ImportStep` verb it replaces — publishes
// nothing. The import itself is the §7.3 `ImportStep` OP, so that it lives on the
// timeline and survives full-replay regen.
//
// Two jobs:
//   * PROBE — `solidCount`, `sourceUnit`, `bbox`, `productNames`, `geometryCodec`,
//     `geometryFormat`, `diagnostics`: enough for the UI to describe the file before
//     committing an op to the document.
//   * CONVERT (`includeGeometry:true`) — the healed, mm-normalized, UNSCALED result
//     serialized in `geometryCodec` in the response's binary tail under section
//     "geometry", solids in the §7.3 ordinal order. This is what makes replay
//     possible without re-parsing STEP: Rust probes once at import-command time,
//     persists the geometry bytes next to the STEP source, and authors the record
//     against them.
//
// ── Why the codec is `xbf` and not `brep` (W4.5) ────────────────────────────
// BinTools brep bytes carry topology only, so the brep replay lane silently dropped
// every imported product name and face color on reopen. The lane now emits BinXCAF
// (`io/XcafCodec`), which round-trips shapes + names + colors. `brep` stays a valid
// §7.3 `sourceCodec` for documents already authored against it.
//
// `productNames` is PER ORDINAL SOLID (not per transfer root): Rust zips it against
// the minted bodies by ordinal, so a per-root list of a different length would be
// discarded wholesale. The names come from the XCAF attribute pass (`io/XcafRead`),
// correlated onto the ordered solids geometrically — see XcafRead.h.
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
