// MassProperties.h — `QueryMassProperties` (SCHEMA §7.5, WP-C1).
//
// Exact kernel mass properties for ONE body: volume, surface area, centroid and
// the principal inertia frame, straight from `BRepGProp`/`GProp_GProps` on the
// real BRep — never re-derived from a tessellation.
//
// READ-ONLY, like every other §7.5 verb: it resolves against a COPY of the head
// (`Session::bodies_copy`), takes no fence, opens no scratch, mints nothing and
// publishes nothing. `GetWorkerHead` is byte-identical before and after (pinned
// in `tests/test_mass_properties.cpp`).
//
// DENSITY-FREE by contract: the app has no material system, so `principalMoments`
// are pure volume integrals (mm^5) about the CENTROID. A consumer that wants a
// mass moment multiplies by density itself.
#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

// SCHEMA §7.5 `QueryMassProperties`. Unknown/absent `bodyId` ⇒ a recoverable
// `REF_UNRESOLVED` error response (an absent body is a caller mistake, not the
// "that element is gone" answer `QueryElement` gives — a mass query names a
// whole body, and there is no partial reading to report).
protocol::Envelope handle_query_mass_properties(Session& session, const protocol::Envelope& req);

}  // namespace onecad::session
