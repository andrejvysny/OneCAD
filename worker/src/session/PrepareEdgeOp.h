#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

protocol::Envelope handle_prepare_edge_op(Session &session,
                                          const protocol::Envelope &req);

} // namespace onecad::session
