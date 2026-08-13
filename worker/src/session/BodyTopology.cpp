#include "session/BodyTopology.h"

#include <string>

#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "session/BodyStore.h"

namespace onecad::session {
namespace {

std::string body_id(const nlohmann::json& args) {
    if (args.is_object() && args.contains("bodyId") && args["bodyId"].is_string()) {
        return args["bodyId"].get<std::string>();
    }
    return {};
}

}  // namespace

protocol::Envelope handle_query_body_topology(Session& session, const protocol::Envelope& req) {
    const std::string id = body_id(req.args);
    const BodyStore bodies = session.bodies_copy();
    const BodyRecord* body = id.empty() ? nullptr : bodies.get(id);
    if (body == nullptr || body->geom.IsNull()) {
        return protocol::Envelope::error_response(
            req.id, protocol::ErrorInfo{"REF_UNRESOLVED",
                                         "QueryBodyTopology: unknown bodyId '" + id + "'", false});
    }
    TopTools_IndexedMapOfShape solids;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body->geom, TopAbs_SOLID, solids);
    TopExp::MapShapes(body->geom, TopAbs_FACE, faces);
    return protocol::Envelope::ok_response(
        req.id, {{"solidCount", solids.Extent()}, {"faceCount", faces.Extent()}});
}

}  // namespace onecad::session
