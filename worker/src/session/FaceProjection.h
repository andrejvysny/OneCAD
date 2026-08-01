// FaceProjection.h — `ProjectFaceBoundary` (SCHEMA §7.6).
//
// A READ-ONLY kernel query: it resolves a picked planar face inside the current
// head, and returns (a) that face's kernel-exact plane frame and (b) its boundary
// — plus, in `coplanarBody` scope, the boundary of every OTHER face of the same
// body coplanar with the SUPPLIED plane — as 2D points + Line/Circle/Arc entities
// in that plane's UV.
//
// It does NOT fence, prepare, accept, discard, or mint anything. Like the §7.5
// identity verbs it works off `Session::bodies_copy()` / `partition_copy()`, so it
// never touches the head lock while it runs, and a stale or absent reference is
// reported as `present:false` — an ANSWER, not an error.
//
// The plane is INPUT and AUTHORITATIVE (SKETCH-ON-FACE decision D1). Rust is the
// basis authority; the worker must never substitute a basis of its own, or a
// sketch's saved coordinates would disagree with the frame the document holds.
// `frameOnly:true` is the first half of that handshake: it returns ONLY `exact`
// (the kernel `gp_Pln` origin + orientation-corrected normal) so Rust can build
// the basis, which it then sends back on the second call.
//
// Identity: the returned `points[].ref` values are RESPONSE-LOCAL (`p0`, `p1`, …).
// They are not ElementIds and carry no persistence — Rust mints the sketch entity
// ids and authors the `Fixed` rows (decision D3').
#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

protocol::Envelope handle_project_face_boundary(Session& session, const protocol::Envelope& req);

}  // namespace onecad::session
