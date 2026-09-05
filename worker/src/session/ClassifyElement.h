// ClassifyElement.h — `ClassifyElement` (SCHEMA §7.5, Component Library WP-0.1).
//
// A READ-ONLY kernel query: given a picked face or edge, answers what KIND of
// surface/curve it is (plane, cylinder, cone, sphere, torus / line, circle,
// ellipse) plus the geometric frame a mate-snap solver needs to seat a
// component against it — plane origin+normal, or cylinder/circle axis+radius.
//
// Modeled on ProjectFaceBoundary (SCHEMA §7.6): it does NOT fence, prepare,
// accept, discard, or mint anything, works off `Session::bodies_copy()` /
// `partition_copy()` so it never touches the head lock, and a stale or absent
// reference is reported as `present:false` — an ANSWER, not an error. No
// `snapshotId` — unlike QueryElement's pick-time addressing (Invariant 4),
// this verb serves a continuously re-issued LIVE hover query, so it always
// reads the current head rather than pinning a snapshot.
//
// This is the interactive half of the Component Library's placement-solver
// spike (spec §5.2/§10 P0): the OCCT call itself is cheap (a couple of
// `BRepAdaptor_*` reads on an already-resolved shape); the real latency risk
// is the stdio round-trip, which is what `classify_latency.rs` measures.
#pragma once

#include "nlohmann/json.hpp"
#include "protocol/Envelope.h"
#include "session/Session.h"

class TopoDS_Shape;

namespace onecad::session {

protocol::Envelope handle_classify_element(Session& session, const protocol::Envelope& req);

// The same classification `handle_classify_element` computes (`kind`,
// `surfaceType`/`curveType`, `frame`), callable IN-PROCESS on a bare shape —
// no wire round trip, no `Session::bodies_copy()` snapshot. For a regen-time
// consumer (Component Library WP-3.1 mate re-seating) that must see THIS
// TICK's geometry while an op executor is still running, not a
// previously-published head. Never `present:false`-shaped like the wire
// verb — the caller already has a concrete shape in hand.
nlohmann::json classify_shape(const TopoDS_Shape& shape);

// `classify_shape` PLUS the SCHEMA §7.5 optional `frame.sidedness`
// (`"pin"` | `"hole"`) a CYLINDRICAL FACE carries (kernel-hardening WP-I).
//
// THE one sidedness producer: both callers that hold a body go through it —
// the `ClassifyElement` verb handler and `ComponentOp.cpp`'s mate re-seat.
// It needs the face's INSTANCE in `body_shape` because the measurement is the
// §10 v4 `outward` evaluation, and history lists hand back NEUTRAL orientation;
// a caller with only a bare shape must keep using `classify_shape`, which
// simply omits the key. `sidedness` is also absent when it cannot be measured
// (no outward evidence, or an evaluation point on the axis).
nlohmann::json classify_shape_in_body(const TopoDS_Shape& shape, const TopoDS_Shape& body_shape);

}  // namespace onecad::session
