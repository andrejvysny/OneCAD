// ImportOp.cpp — see ImportOp.h.
#include "ops/ImportOp.h"

#include <cmath>
#include <filesystem>
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
#include "io/StepRead.h"
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
                  std::vector<std::vector<std::uint32_t>>& face_colors, double factor,
                  std::string& err) {
    gp_Trsf trsf;
    trsf.SetScale(gp_Pnt(0.0, 0.0, 0.0), factor);
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
        solids[k] = xf.Shape();
    }
    return true;
}

// BRepCheck over the solids about to be published. Advisory in both codecs — see
// kInvalidShape. The step lane already reports its own copy via read diagnostics,
// so this only runs for the brep lane to keep the two lanes' output identical.
void check_solids(const std::vector<TopoDS_Shape>& solids, OpOutcome& out) {
    for (std::size_t k = 0; k < solids.size(); ++k) {
        BRepCheck_Analyzer analyzer(solids[k]);
        if (analyzer.IsValid() != Standard_True) {
            out.diagnostics.push_back(
                advisory(kInvalidShape, "solid " + std::to_string(k) + " fails BRepCheck"));
        }
    }
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
    std::error_code ec;
    if (!std::filesystem::is_regular_file(path, ec)) {
        return OpOutcome::fail("OP_FAILED", "ImportStep: source path is not a readable file (" +
                                                path + ") for source " + label);
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

    if (codec == "step") {
        const io::StepReadResult read = io::read_step(path, io::StepReadPolicy{}, ctx.cancel);
        if (read.cancelled) return OpOutcome::cancelled();
        if (!read.ok()) {
            return OpOutcome::fail("OP_FAILED",
                                   "ImportStep: " + *read.error + " for source " + label);
        }
        for (const io::StepReadDiagnostic& d : read.diagnostics) {
            out.diagnostics.push_back(advisory(d.code, d.message));
        }
        if (read.solids.empty()) {
            return OpOutcome::fail("OP_FAILED", "ImportStep: no solid recovered from source " + label);
        }
        solids = read.solids;  // already in ops::ordered_solids order (W0)
    } else if (codec == "brep") {
        const std::string bad = check_format(io::kBrepFormatVersion);
        if (!bad.empty()) return OpOutcome::fail("OP_FAILED", bad);
        const io::BrepReadResult read = io::read_brep_solids(path);
        if (!read.ok()) {
            return OpOutcome::fail("OP_FAILED", "ImportStep: " + read.error + " for source " + label);
        }
        solids = read.solids;  // STORED order — never re-sorted (see ImportOp.h)
        check_solids(solids, out);
    } else {
        const std::string bad = check_format(io::kXcafFormatVersion);
        if (!bad.empty()) return OpOutcome::fail("OP_FAILED", bad);
        const io::XcafReadResult read = io::read_xcaf_solids(path);
        if (!read.ok()) {
            return OpOutcome::fail("OP_FAILED", "ImportStep: " + read.error + " for source " + label);
        }
        solids = read.solids;  // STORED order — never re-sorted (see ImportOp.h)
        face_colors.reserve(read.attributes.size());
        for (const io::SolidAttributes& a : read.attributes) face_colors.push_back(a.face_colors);
        check_solids(solids, out);
    }

    if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    if (unit_scale != 1.0) {
        std::string err;
        try {
            if (!scale_solids(solids, face_colors, unit_scale, err)) {
                return OpOutcome::fail("OP_FAILED", "ImportStep: " + err + " for source " + label);
            }
        } catch (const Standard_Failure& f) {
            return OpOutcome::fail("OP_FAILED",
                                   std::string("ImportStep: unitScale transform raised: ") +
                                       (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
        }
    }

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
        // Appearance rides the BODY, not the document (see BodyStore.h): the
        // tessellator copies it into the MESH1 FACE_COLORS section, and nothing
        // mints an ElementId or a TopoKey for it.
        if (k < face_colors.size() && !face_colors[k].empty()) {
            rec.face_colors = face_colors[k];
            ++colored_bodies;
        }
        out.body_events.push_back({"created", bid});
        out.body_ids.push_back(bid);
    }
    if (colored_bodies > 0) {
        WLOG_INFO("ImportStep: %zu of %zu imported body(ies) carry authored face colors",
                  colored_bodies, solids.size());
    }
    return out;
}

}  // namespace onecad::ops
