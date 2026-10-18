#pragma once

#include "protocol/Envelope.h"

namespace onecad::session {
class Session;

// Shared sidecar/library Tessellate verb handler. Failure returns no partial tail.
protocol::Envelope handle_tessellate(Session& session, const protocol::Envelope& request);
}  // namespace onecad::session
