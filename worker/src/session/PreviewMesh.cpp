// PreviewMesh.cpp — see PreviewMesh.h. Moved verbatim from PreviewOp.cpp; only
// the failure became structured (PreviewOp's wire output is unchanged).
#include "session/PreviewMesh.h"

#include <utility>

#include "protocol/Limits.h"
#include "tess/MeshHandle.h"
#include "tess/Tessellate.h"
#include "util/Hashing.h"

namespace onecad::session {

using nlohmann::json;

std::string coarser_lod(const std::string& lod) {
    if (lod == "fine") return "medium";
    if (lod == "medium") return "coarse";
    return {};
}

std::optional<PreviewMeshFailure> append_preview_mesh(
    const char* verb, const std::string& body_id, const BodyRecord& body,
    const std::string& lod, std::uint64_t snapshot_id,
    const elementmap::ElementMapPartition& partition, protocol::Envelope& response,
    json& meshes) {
    /*
     * SCHEMA §5.2/§6: a control response may not exceed the transport limits this
     * worker ADVERTISES in its hello. DEGRADE rather than refuse: a preview is an
     * illustration, so a coarser tier is a perfectly good answer, while an error
     * would blank it. Only when even `coarse` cannot fit does this fail, and then
     * it says which limit and by how much rather than emitting an over-budget
     * frame the client is entitled to treat as a protocol violation.
     */
    std::string tier = lod;
    tess::BodyMesh mesh;
    for (;;) {
        mesh = tess::tessellate_body(body.geom, body_id, tier, /*include_edges=*/true, &partition,
                                     &body.face_colors);
        if (!mesh.ok || mesh.triangle_count == 0) {
            PreviewMeshFailure failure;
            failure.kind = PreviewMeshFailure::Kind::Tessellate;
            failure.body_id = body_id;
            failure.message =
                std::string(verb) + ": changed body produced no preview mesh: " + body_id;
            return failure;
        }
        const std::uint64_t blob = mesh.blob.size();
        const bool fits = blob <= protocol::kChunkSize &&
                          response.out_bin.size() + blob <= protocol::kInitialBulkCredit;
        if (fits) break;
        const std::string next = coarser_lod(tier);
        if (next.empty()) {
            PreviewMeshFailure failure;
            failure.kind = PreviewMeshFailure::Kind::Budget;
            failure.body_id = body_id;
            failure.bytes = blob;
            failure.limit = blob > protocol::kChunkSize
                                ? protocol::kChunkSize
                                : protocol::kInitialBulkCredit - response.out_bin.size();
            failure.message = std::string(verb) + ": preview mesh for " + body_id + " is " +
                              std::to_string(blob) +
                              " bytes at the coarsest LOD, over the advertised chunkSize " +
                              std::to_string(protocol::kChunkSize) + " / remaining bulk credit";
            return failure;
        }
        tier = next;
    }
    const std::uint64_t offset = response.out_bin.size();
    response.out_bin.insert(response.out_bin.end(), mesh.blob.begin(), mesh.blob.end());
    const std::string section = "mesh:" + body_id;
    response.bin.push_back(protocol::BinSection{section, offset, mesh.blob.size()});
    // `tier`, not `lod`: the handle must name the LOD the bytes were actually
    // built at, or a degraded preview would claim a fidelity it does not have.
    meshes.push_back(tess::mesh_handle_json(
        body_id, section, tier, mesh.blob.size(), mesh.triangle_count,
        hashing::sha256_hex(mesh.blob.data(), mesh.blob.size()), snapshot_id));
    return std::nullopt;
}

}  // namespace onecad::session
