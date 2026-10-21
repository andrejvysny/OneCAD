// PreviewMesh.h — the inline MESH1 budget rule shared by the two preview verbs
// (`PreviewOp`, `PreviewPlan`; SCHEMA §7.6).
//
// A preview inlines its meshes in the terminal response's binary tail, so every
// blob must fit the transport limits this worker ADVERTISES in its hello
// (`chunkSize` per blob, `initialBulkCredit` aggregate; SCHEMA §5.2/§6). The rule
// DEGRADES rather than refuses: fine → medium → coarse, and fails only when even
// `coarse` cannot fit. The failure is structured so each verb can map it to its
// own §8 code without re-parsing a message.
#pragma once

#include <cstdint>
#include <optional>
#include <string>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"

namespace onecad::session {

struct PreviewMeshFailure {
    enum class Kind { Tessellate, Budget };
    Kind kind = Kind::Tessellate;
    std::string body_id;
    std::uint64_t bytes = 0;  // Budget: blob size at the coarsest tier
    std::uint64_t limit = 0;  // Budget: the limit it exceeded
    std::string message;      // "<verb>: …", the text PreviewOp has always sent
};

// Coarser tier, or empty when `lod` is already the coarsest (SCHEMA §7.6).
std::string coarser_lod(const std::string& lod);

// Tessellates `body`, degrading the tier until the blob fits, appends it to
// `response`'s binary tail as `mesh:<body_id>` and its §7.6 handle (naming the
// ACTUAL tier) to `meshes`. `verb` prefixes the failure message.
std::optional<PreviewMeshFailure> append_preview_mesh(
    const char* verb, const std::string& body_id, const BodyRecord& body,
    const std::string& lod, std::uint64_t snapshot_id,
    const elementmap::ElementMapPartition& partition, protocol::Envelope& response,
    nlohmann::json& meshes);

}  // namespace onecad::session
