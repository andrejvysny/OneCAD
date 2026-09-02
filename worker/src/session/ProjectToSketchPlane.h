// ProjectToSketchPlane.h — `ProjectToSketchPlane` (SCHEMA §7.6, WP-P).
//
// A READ-ONLY, SNAPSHOT-FENCED kernel query. Given a sketch plane and a BATCH of
// picked edges (or picked faces, in `faceOutline` mode), it returns their
// projection into that plane's UV as Line/Circle/Arc/Ellipse entities, each
// carrying the source it came from and a hash of its projected geometry.
//
// It mints nothing, prepares nothing, and touches no head: head COPY only, no
// scratch, no snapshot, no `bodyEvents`, no bin tail.
//
// WHY IT IS FENCED and its model `ProjectFaceBoundary` is not. The answer is
// COMMITTED into a document sketch, and the `projectedHash` it mints becomes the
// baseline every later staleness check compares against. An advisory read would
// let a stale capture write a baseline that can never afterwards be detected as
// wrong — the silent-wrong-bind class this migration exists to fix. A stale
// `snapshotId` is `STALE_PREVIEW`, the `PrepareOffsetFace` rule.
//
// The plane is INPUT and AUTHORITATIVE. `sketchId` is an OPAQUE ECHO and MUST
// NOT be resolved against the worker's solver-lane `SketchStore` to obtain a
// basis: that copy can lag the document, and Rust owns the frame.
//
// Refusals are per-SOURCE ANSWERS (`ok:true`), never whole-call errors: a batch
// has no single presence answer, so one dead pick in a thirty-edge selection
// must not void the other twenty-nine. Only a stale snapshot and a malformed
// request are errors.
#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

protocol::Envelope handle_project_to_sketch_plane(Session& session, const protocol::Envelope& req);

}  // namespace onecad::session
