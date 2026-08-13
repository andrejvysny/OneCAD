// ExportGeometry.h — the ExportGeometry verb (SCHEMA §7.8).
//
// Writes the requested live bodies to the Rust-provided temp path in one of the
// worker's own REPLAY codecs (`brep` / `xbf`), i.e. the byte forms §7.3
// `ImportStep.sourceCodec` already reads back. This is the inverse of the
// `InspectStep` conversion lane: that one converts a foreign STEP file INTO the
// replay form, this one converts a body ALREADY IN THE SESSION into it.
//
// Why it exists: the Component Library's `embedded` / `document` source kinds
// (spec §2.1) resolve to a baked solid cached in the placing document, and
// "Save as Component" (spec §7) has to produce that solid from what the user
// modelled. Nothing else in the protocol can get geometry OUT of a session as
// bytes — `GetBodies` (§7.6) is specified but unimplemented, and its
// bulk-stream shape is heavier than a file hop the §7.8 lane already does for
// every other export.
//
// The response reports `codec` + `format` back rather than making Rust hardcode
// them, so the `brepFormat` a record pins is always the version this worker
// actually wrote (the same discipline `InspectStep` follows with
// `geometryCodec`/`geometryFormat`).
#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::io {

protocol::Envelope handle_export_geometry(session::Session& session, const protocol::Envelope& req);

}  // namespace onecad::io
