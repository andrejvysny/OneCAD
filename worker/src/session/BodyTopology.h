// Read-only whole-body topology counts (SCHEMA §7.5 QueryBodyTopology).
#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

// Unknown or absent bodyId is REF_UNRESOLVED. Counts come from the real BRep,
// never MESH1: tessellation LOD must not change a corpus oracle.
protocol::Envelope handle_query_body_topology(Session& session, const protocol::Envelope& req);

}  // namespace onecad::session
