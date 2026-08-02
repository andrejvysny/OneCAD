// ImportOp.cpp — see ImportOp.h.
#include "ops/ImportOp.h"

#include <cmath>
#include <filesystem>
#include <string>
#include <vector>

#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>

#include "io/BrepCodec.h"
#include "io/StepRead.h"
#include "ops/OpCommon.h"

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
bool scale_solids(std::vector<TopoDS_Shape>& solids, double factor, std::string& err) {
    gp_Trsf trsf;
    trsf.SetScale(gp_Pnt(0.0, 0.0, 0.0), factor);
    for (TopoDS_Shape& s : solids) {
        BRepBuilderAPI_Transform xf(s, trsf, Standard_True);
        if (!xf.IsDone() || xf.Shape().IsNull()) {
            err = "unitScale transform failed";
            return false;
        }
        s = xf.Shape();
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
    if (codec != "step" && codec != "brep") {
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
    } else {
        // `brepFormat` REQUIRED for this codec (SCHEMA §7.3): it pins the BinTools
        // version the bytes were written in. A record pinned to a version this
        // worker does not write is a data problem, not a parse problem — report it
        // before OCCT gets a chance to fail obscurely.
        if (!params.contains("brepFormat") || !params["brepFormat"].is_number_integer()) {
            return OpOutcome::fail("OP_FAILED",
                                   "ImportStep: sourceCodec 'brep' requires an integer brepFormat "
                                   "for source " + label);
        }
        const int brep_format = params["brepFormat"].get<int>();
        if (brep_format != io::kBrepFormatVersion) {
            return OpOutcome::fail("OP_FAILED",
                                   "ImportStep: brepFormat " + std::to_string(brep_format) +
                                       " is not the version this worker writes (" +
                                       std::to_string(io::kBrepFormatVersion) + ") for source " +
                                       label);
        }
        const io::BrepReadResult read = io::read_brep_solids(path);
        if (!read.ok()) {
            return OpOutcome::fail("OP_FAILED", "ImportStep: " + read.error + " for source " + label);
        }
        solids = read.solids;  // STORED order — never re-sorted (see ImportOp.h)
        check_solids(solids, out);
    }

    if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    if (unit_scale != 1.0) {
        std::string err;
        try {
            if (!scale_solids(solids, unit_scale, err)) {
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
    for (std::size_t k = 0; k < solids.size(); ++k) {
        const std::string bid = solids.size() == 1
                                    ? "body_" + op_id
                                    : "body_" + op_id + ":" + std::to_string(k);
        ctx.bodies.create(bid, op_id, solids[k]);
        out.body_events.push_back({"created", bid});
        out.body_ids.push_back(bid);
    }
    return out;
}

}  // namespace onecad::ops
