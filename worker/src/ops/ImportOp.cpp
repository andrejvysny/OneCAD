// ImportOp.cpp — see ImportOp.h.
#include "ops/ImportOp.h"

#include <cmath>
#include <filesystem>
#include <optional>
#include <string>
#include <vector>

#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>

#include "io/BrepCodec.h"
#include "io/OcctBusy.h"
#include "io/StepRead.h"
#include "io/SourceCapture.h"
#include "io/SourceIdentity.h"
#include "util/PolicyVersions.h"
#include "io/XcafCodec.h"
#include "ops/OpCommon.h"
#include "util/Log.h"

namespace onecad::ops {

using nlohmann::json;

namespace {

// The §7.3 diagnostics vocabulary entry for a solid that survives healing but
// fails BRepCheck. Emitted (not failed on) by BOTH codecs so the brep replay of a
// marginal file reports exactly what the step read of it reported.
constexpr const char* kInvalidShape = onecad::io::step_diag::kInvalidShape;

json advisory(const std::string& code, const std::string& message) {
    return json{{"severity", "warning"}, {"code", code}, {"message", message}};
}

// The source's content address, for failure messages. A path is a Rust-side temp
// detail; the sha is what identifies the bytes in the document container, so it is
// the useful half of "which import failed".
std::string source_label(const json& params) {
    const std::string sha = read_str(params, "sourceSha256");
    const std::string name = read_str(params, "sourceName");
    std::string label = sha.empty() ? std::string("<no sourceSha256>") : sha;
    if (!name.empty()) label += " (" + name + ")";
    return label;
}

// Refuse shared, merged and multiply claimed faces without quadratic scans.
std::vector<std::size_t> claim_owners(const std::vector<TopoDS_Shape>& solids,
                                      std::span<const io::SourceFaceClaim> claims) {
    TopTools_IndexedMapOfShape all;
    std::vector<std::size_t> counts(1, 0), body(1, 0);
    for (std::size_t k = 0; k < solids.size(); ++k) {
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(solids[k], TopAbs_FACE, faces);
        for (int i = 1; i <= faces.Extent(); ++i) {
            const int index = all.Add(faces(i));
            if (static_cast<std::size_t>(index) >= counts.size()) {
                counts.resize(static_cast<std::size_t>(index) + 1, 0);
                body.resize(static_cast<std::size_t>(index) + 1, 0);
            }
            ++counts[index];
            body[index] = k;
        }
    }
    std::vector<std::size_t> claim_counts(counts.size(), 0);
    for (const auto& claim : claims) {
        const int index = claim.face.IsNull() ? 0 : all.FindIndex(claim.face);
        if (index > 0) ++claim_counts[index];
    }
    std::vector<std::size_t> result(claims.size(), solids.size());
    for (std::size_t j = 0; j < claims.size(); ++j) {
        const int index = claims[j].face.IsNull() ? 0 : all.FindIndex(claims[j].face);
        if (index > 0 && counts[index] == 1 && claim_counts[index] == 1 &&
            io::oriented_source_member(all, claims[j].face)) result[j] = body[index];
    }
    return result;
}

// Uniform scale about the origin. Copy=true so the canonical (unscaled) shape the
// caller holds is never mutated — re-editing `unitScale` must re-scale from the
// canonical geometry, not compound onto an already-scaled one.
//
// `face_colors` (parallel to `solids`, may be empty) is REMAPPED through the
// transform's own history rather than assumed index-stable: the color index space
// is the `TopExp::MapShapes` order of the shape it describes, and that shape is
// being replaced. `ModifiedShape` is the only authority on which new face is which
// old face.
bool scale_solids(std::vector<TopoDS_Shape>& solids,
                  std::vector<std::vector<std::uint32_t>>& face_colors,
                  std::span<io::SourceFaceClaim> claims, double factor, std::string& err) {
    gp_Trsf trsf;
    trsf.SetScale(gp_Pnt(0.0, 0.0, 0.0), factor);
    TopTools_IndexedMapOfShape claim_faces;
    std::vector<std::vector<std::size_t>> claim_indices(1);
    for (std::size_t j = 0; j < claims.size(); ++j) {
        if (claims[j].face.IsNull()) continue;
        const int index = claim_faces.Add(claims[j].face);
        if (static_cast<std::size_t>(index) >= claim_indices.size()) claim_indices.resize(index + 1);
        claim_indices[index].push_back(j);
    }
    for (std::size_t k = 0; k < solids.size(); ++k) {
        BRepBuilderAPI_Transform xf(solids[k], trsf, Standard_True);
        if (!xf.IsDone() || xf.Shape().IsNull()) {
            err = "unitScale transform failed";
            return false;
        }
        if (k < face_colors.size() && !face_colors[k].empty()) {
            TopTools_IndexedMapOfShape before;
            TopTools_IndexedMapOfShape after;
            TopExp::MapShapes(solids[k], TopAbs_FACE, before);
            TopExp::MapShapes(xf.Shape(), TopAbs_FACE, after);
            std::vector<std::uint32_t> remapped(static_cast<std::size_t>(after.Extent()),
                                                io::kUnsetColor);
            for (int i = 1; i <= before.Extent(); ++i) {
                const std::size_t src = static_cast<std::size_t>(i - 1);
                if (src >= face_colors[k].size() || face_colors[k][src] == io::kUnsetColor) {
                    continue;
                }
                const TopoDS_Shape moved = xf.ModifiedShape(before(i));
                const int dst = moved.IsNull() ? 0 : after.FindIndex(moved);
                if (dst >= 1) remapped[static_cast<std::size_t>(dst - 1)] = face_colors[k][src];
            }
            face_colors[k] = std::move(remapped);
        }
        TopTools_IndexedMapOfShape before_claim_faces;
        TopTools_IndexedMapOfShape after_claim_faces;
        TopExp::MapShapes(solids[k], TopAbs_FACE, before_claim_faces);
        TopExp::MapShapes(xf.Shape(), TopAbs_FACE, after_claim_faces);
        for (int i = 1; i <= before_claim_faces.Extent(); ++i) {
            const int index = claim_faces.FindIndex(before_claim_faces(i));
            if (index == 0) continue;
            TopoDS_Shape moved = xf.ModifiedShape(before_claim_faces(i));
            // Positive uniform scaling preserves the oriented source occurrence;
            // OCCT's ModifiedShape returns its canonical orientation instead.
            if (!moved.IsNull()) moved.Orientation(before_claim_faces(i).Orientation());
            const bool valid = io::oriented_source_member(after_claim_faces, moved);
            for (std::size_t j : claim_indices[index]) claims[j].face = valid ? moved : TopoDS_Shape{};
        }
        solids[k] = xf.Shape();
    }
    return true;
}

// Classify every imported solid after all transforms. Unhealthy geometry is still
// visible/exportable, but is admitted as QUARANTINED and cannot enter modeling.
// Analyzer exceptions are UNKNOWN and therefore quarantine rather than pass.
std::vector<bool> classify_solids(const std::vector<TopoDS_Shape>& solids,
                                  OpOutcome& out) {
    std::vector<bool> quarantined(solids.size(), false);
    for (std::size_t k = 0; k < solids.size(); ++k) {
        std::string reason;
        try {
            BRepCheck_Analyzer analyzer(solids[k]);
            if (analyzer.IsValid() != Standard_True)
                reason = "fails BRepCheck";
        } catch (const Standard_Failure& failure) {
            reason = std::string("validity is unknown: ") +
                     (failure.GetMessageString() ? failure.GetMessageString()
                                                 : "OCCT analyzer raised");
        } catch (...) {
            reason = "validity is unknown: analyzer raised";
        }
        if (reason.empty()) continue;
        quarantined[k] = true;
        out.diagnostics.push_back(advisory(
            kInvalidShape, "solid " + std::to_string(k) + " " + reason));
        out.diagnostics.push_back(advisory(
            "IMPORT_QUARANTINED",
            "solid " + std::to_string(k) +
                " is visible/exportable but unavailable to modeling until repaired"));
    }
    return quarantined;
}

}  // namespace

OpOutcome execute_import_step(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();
    const std::string label = source_label(params);

    const std::string codec = read_str(params, "sourceCodec", "step");
    if (codec != "step" && codec != "brep" && codec != io::kXcafCodecName) {
        return OpOutcome::fail("OP_FAILED", "ImportStep: unknown sourceCodec '" + codec +
                                                "' for source " + label);
    }
    // A heal policy this worker does not implement must fail LOUDLY: silently
    // reading a `v2` record with the v1 pipeline would hand back different
    // geometry under the same hash.
    const std::string heal_policy = read_str(params, "healPolicy", "v1");
    if (heal_policy != "v1") {
        return OpOutcome::fail("OP_FAILED", "ImportStep: unsupported healPolicy '" + heal_policy +
                                                "' (this worker implements v1) for source " + label);
    }

    const double unit_scale = read_scalar(params, "unitScale", 1.0);
    if (!std::isfinite(unit_scale) || unit_scale <= 0.0) {
        return OpOutcome::fail("OP_FAILED", "ImportStep: unitScale must be a positive finite number");
    }

    const std::string path = read_str(params, "path");
    if (path.empty()) {
        return OpOutcome::fail("OP_FAILED",
                               "ImportStep: no source path supplied for source " + label);
    }
    if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    OpOutcome out;
    std::vector<TopoDS_Shape> solids;
    // Parallel to `solids` (or empty). The step/brep lanes leave it empty: neither
    // byte form carries appearance, which is exactly why the conversion lane moved
    // to xbf (SCHEMA §14, 2026-08-02).
    std::vector<std::vector<std::uint32_t>> face_colors;

    // `brepFormat` REQUIRED for every CONVERTED codec (SCHEMA §7.3): it pins the
    // binary format version the bytes were written in. A record pinned to a version
    // this worker does not write is a data problem, not a parse problem — report it
    // before OCCT gets a chance to fail obscurely.
    const auto check_format = [&](int expected) -> std::string {
        if (!params.contains("brepFormat") || !params["brepFormat"].is_number_integer()) {
            return "ImportStep: sourceCodec '" + codec +
                   "' requires an integer brepFormat for source " + label;
        }
        const int got = params["brepFormat"].get<int>();
        if (got != expected) {
            return "ImportStep: brepFormat " + std::to_string(got) +
                   " is not the version this worker writes (" + std::to_string(expected) +
                   ") for source " + label;
        }
        return {};
    };

    if (codec == "brep" || codec == io::kXcafCodecName) {
        const std::string bad = check_format(codec == "brep" ? io::kBrepFormatVersion : io::kXcafFormatVersion);
        if (!bad.empty()) return OpOutcome::fail("OP_FAILED", bad);
    }
    const io::CapturedSource source = io::capture_source(path, read_str(params, "sourceSha256"));
    if (!source.ok()) return OpOutcome::fail("OP_FAILED", "ImportStep: " + source.error + " for source " + label);
    io::SourceClaimBatch claims;

    if (codec == "step") {
        io::StepReadResult read = io::read_step_bytes(source.bytes, io::StepReadPolicy{}, ctx.cancel);
        if (read.cancelled) return OpOutcome::cancelled();
        if (!read.ok() && io::is_occt_busy(*read.error)) {
            return OpOutcome::busy(io::busy_message(*read.error, "ImportStep source " + label));
        }
        if (!read.ok()) {
            auto failure = OpOutcome::fail("OP_FAILED", "ImportStep: " + *read.error + " for source " + label);
            if (read.error->starts_with(io::kAmbiguousImportOrder)) {
                failure.diagnostics.push_back(json{{"severity", "error"}, {"code", "OP_FAILED"},
                    {"reasonCode", io::kAmbiguousImportOrder}, {"stage", "import"}, {"message", *read.error}});
            }
            return failure;
        }
        for (const io::StepReadDiagnostic& d : read.diagnostics) {
            out.diagnostics.push_back(advisory(d.code, d.message));
        }
        if (read.solids.empty()) {
            return OpOutcome::fail("OP_FAILED", "ImportStep: no solid recovered from source " + label);
        }
        solids = read.solids;  // already in ops::ordered_solids order (W0)
        claims = std::move(read.face_claims);
    } else if (codec == "brep") {
        const io::BrepReadResult read = io::read_brep_solids_bytes(source.bytes);
        if (!read.ok()) {
            return OpOutcome::fail("OP_FAILED", "ImportStep: " + read.error + " for source " + label);
        }
        solids = read.solids;  // STORED order — never re-sorted (see ImportOp.h)
        out.diagnostics.push_back(advisory("SOURCE_FACE_IDENTITY_UNAVAILABLE", "BinTools serialized entity slots are not yet proven"));
    } else {
        io::XcafReadResult read = io::read_xcaf_solids_bytes(source.bytes);
        if (!read.ok() && io::is_occt_busy(read.error)) {
            return OpOutcome::busy(io::busy_message(read.error, "ImportStep source " + label));
        }
        if (!read.ok()) {
            return OpOutcome::fail("OP_FAILED", "ImportStep: " + read.error + " for source " + label);
        }
        solids = read.solids;  // STORED order — never re-sorted (see ImportOp.h)
        claims = std::move(read.face_claims);
        if (read.unclaimed_faces != 0) {
            out.diagnostics.push_back(advisory("SOURCE_FACE_IDENTITY_PARTIAL",
                "XBF has faces without unique explicit subshape labels"));
        }
        face_colors.reserve(read.attributes.size());
        for (const io::SolidAttributes& a : read.attributes) face_colors.push_back(a.face_colors);
    }

    if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    if (!io::source_producer_scope(op_id, "body_" + op_id)) {
        claims.clear();
        out.diagnostics.push_back(advisory("SOURCE_FACE_IDENTITY_PARTIAL", "producer identity is not a canonical UUID"));
    }
    const auto initial_owners = claim_owners(solids, claims.items());
    for (std::size_t j = 0; j < claims.size(); ++j) {
        if (initial_owners[j] == solids.size()) claims[j].face.Nullify();
    }

    if (unit_scale != 1.0) {
        std::string err;
        try {
            if (!scale_solids(solids, face_colors, claims.mutable_items(), unit_scale, err)) {
                return OpOutcome::fail("OP_FAILED", "ImportStep: " + err + " for source " + label);
            }
        } catch (const Standard_Failure& f) {
            return OpOutcome::fail("OP_FAILED",
                                   std::string("ImportStep: unitScale transform raised: ") +
                                       (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
        }
    }

    const auto body_index = claim_owners(solids, claims.items());
    const std::vector<bool> quarantined = classify_solids(solids, out);
    for (std::size_t j = 0; j < claims.size(); ++j) {
        if (body_index[j] == solids.size() || quarantined[body_index[j]]) {
            claims[j].face.Nullify();
            continue;
        }
        auto& claim = claims[j];
        claim.source_sha256 = source.sha256;
        claim.reader_policy = codec == "step" ? "step-v1" : "xbf-v12";
        claim.kernel_fingerprint = policy::occt_fingerprint();
        claim.producer_id = op_id;
        claim.body_id = solids.size() == 1 ? "body_" + op_id
                                          : "body_" + op_id + ":" + std::to_string(body_index[j]);
        claim.unit_scale = unit_scale;
        if (!io::SourceClaimBatch::bounded(claim)) claim.face.Nullify();
    }
    const auto old_count = claims.size();
    claims.erase_if([](const io::SourceFaceClaim& claim) { return claim.face.IsNull(); });
    if (codec != "brep" && claims.size() != old_count) {
        out.diagnostics.push_back(advisory("SOURCE_FACE_IDENTITY_PARTIAL", "ambiguous source faces were omitted"));
    }
    if (!claims.empty()) out.source_face_claims = std::make_shared<io::SourceClaimBatch>(std::move(claims));
    // Minting (SCHEMA §2/§7.3, D1) — the same ordered-children rule
    // `ops::publish_boolean_result` applies to a split, minus the deleted parent:
    // an import creates ex nihilo, so there is nothing to delete and no partition
    // entry to relabel (the delta stays empty; ElementIds are minted on demand).
    std::size_t colored_bodies = 0;
    for (std::size_t k = 0; k < solids.size(); ++k) {
        const std::string bid = solids.size() == 1
                                    ? "body_" + op_id
                                    : "body_" + op_id + ":" + std::to_string(k);
        onecad::session::BodyRecord& rec = ctx.bodies.create(bid, op_id, solids[k]);
        std::optional<std::string> health;
        if (k < quarantined.size() && quarantined[k]) {
            rec.health = session::BodyHealth::Quarantined;
            rec.health_reason = "imported geometry failed strict B-Rep validation";
            health = "quarantined";
        }
        // Appearance rides the BODY, not the document (see BodyStore.h): the
        // tessellator copies it into the MESH1 FACE_COLORS section, and nothing
        // mints an ElementId or a TopoKey for it.
        if (k < face_colors.size() && !face_colors[k].empty()) {
            rec.face_colors = face_colors[k];
            ++colored_bodies;
        }
        out.body_events.push_back({"created", bid, {}, std::move(health)});
        out.body_ids.push_back(bid);
    }
    if (colored_bodies > 0) {
        WLOG_INFO("ImportStep: %zu of %zu imported body(ies) carry authored face colors",
                  colored_bodies, solids.size());
    }
    return out;
}

}  // namespace onecad::ops
