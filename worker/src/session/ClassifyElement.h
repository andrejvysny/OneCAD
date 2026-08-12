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

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

protocol::Envelope handle_classify_element(Session& session, const protocol::Envelope& req);

}  // namespace onecad::session
