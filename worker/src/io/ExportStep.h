// ExportStep.h — the ExportStep verb (SCHEMA §7.8, D2).
//
// Writes the requested live bodies to a STEP file at the Rust-provided temp path.
// All IO is worker-side (the webview has zero fs capability). OCCT failures are
// guarded into a recoverable OP_FAILED (SCHEMA §8; session intact).
//
// ── Why XCAF and not the plain writer (DI-5) ────────────────────────────────
// `STEPControl_Writer` can express geometry and nothing else, so every export
// dropped the body NAMES and the per-face COLOURS the document carries — a file
// that came in coloured through `ImportStep` went out grey and anonymous, and a
// user-authored face colour never left the app at all. This builds a real
// `TDocStd_Document` (`XCAFDoc_ShapeTool` + `XCAFDoc_ColorTool`) and hands it to
// `STEPCAFControl_Writer`, which is the only writer that emits `product` names and
// `surface_style_usage`. Colours are converted with `io::unpack_srgb`, the exact
// inverse of the `pack_srgb` the import lane reads with, so `import(export(x))` is
// identity on the colour bytes.
//
// Appearance inputs, in precedence order:
//   1. `BodyRecord::face_colors` — import-derived, ordinal-indexed, worker-owned;
//   2. `args.bodyColors[bodyId]` — the authored WHOLE-BODY colour, written on the
//      body's own label (what XcafRead reads back as a part colour);
//   3. `args.faceColors[bodyId][topoKey]` — the authored PER-FACE table. Rust
//      resolves each persistent ElementId to its current TopoKey before sending;
//      a key that does not address a face of that body is dropped and counted in
//      `unresolvedFaceColors`, never nudged onto a neighbouring ordinal.
//
// Returns { written, bytes, namedBodies, coloredFaces, unresolvedFaceColors }.
#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::io {

protocol::Envelope handle_export_step(session::Session& session, const protocol::Envelope& req);

}  // namespace onecad::io
