#include "session/Tessellate.h"

#include "session/Session.h"
#include "tess/MeshHandle.h"
#include "tess/Tessellate.h"
#include "util/Hashing.h"

namespace onecad::session {
using protocol::Envelope;

// Tessellate (SCHEMA §7.6): mesh the requested live bodies into MESH1 blobs. Small
// blobs are inlined in the resp binary tail (§5.2 permits inline ≤ chunkSize); the
// result references each by bin section name + carries totalBytes + sha256.
Envelope handle_tessellate(Session& session, const Envelope& req) {
    // OPTIONAL snapshot fence (SCHEMA §7.6, WP-H): refuse BEFORE meshing anything
    // when the caller names a snapshot that is no longer the head. Absent ⇒ the
    // live head, byte-identical to every pre-WP-H request.
    if (auto stale = onecad::session::stale_snapshot_fence(session, req, "Tessellate")) {
        return *stale;
    }
    const nlohmann::json& args = req.args;
    const std::string lod = args.value("lod", std::string("coarse"));
    const bool include_edges = args.value("includeEdges", true);
    const onecad::session::BodyStore bodies = session.bodies_copy();
    const onecad::elementmap::ElementMapPartition part = session.partition_copy();
    const std::uint64_t snapshot_id = session.current_snapshot_id();

    // bodyIds: "all" or an explicit array.
    std::vector<std::string> which;
    if (args.contains("bodyIds") && args["bodyIds"].is_array()) {
        for (const auto& b : args["bodyIds"])
            if (b.is_string()) which.push_back(b.get<std::string>());
    } else {
        which = bodies.ids();  // "all" (or missing) → every body
    }

    nlohmann::json meshes = nlohmann::json::array();
    Envelope resp = Envelope::ok_response(req.id, nlohmann::json::object());
    for (const std::string& bid : which) {
        const onecad::session::BodyRecord* rec = bodies.get(bid);
        if (!rec) continue;
        onecad::tess::BodyMesh bm =
            onecad::tess::tessellate_body(rec->geom, bid, lod, include_edges, &part,
                                          &rec->face_colors);
        if (!bm.ok) {
            return Envelope::error_response(req.id, onecad::protocol::ErrorInfo{
                "OP_FAILED", "Tessellate: incomplete triangulation for body " + bid, false,
                nlohmann::json{{"bodyId", bid}}});
        }
        const std::uint64_t off = resp.out_bin.size();
        resp.out_bin.insert(resp.out_bin.end(), bm.blob.begin(), bm.blob.end());
        const std::string section = "mesh:" + bid;
        resp.bin.push_back(onecad::protocol::BinSection{section, off, bm.blob.size()});
        // Shared §7.6 handle builder (identical shape as ExecutePlan's inline artifact
        // — MeshHandle.h). The inline handle keys the resp-tail section by "bin".
        meshes.push_back(onecad::tess::mesh_handle_json(
            bid, section, lod, bm.blob.size(), bm.triangle_count,
            onecad::hashing::sha256_hex(bm.blob.data(), bm.blob.size()), snapshot_id));
    }
    resp.result = nlohmann::json{{"meshes", std::move(meshes)}};
    return resp;
}

}  // namespace onecad::session
