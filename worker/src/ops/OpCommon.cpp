// OpCommon.cpp — see OpCommon.h. Ports from OneCAD-CPP RegenerationEngine.cpp.
#include "ops/OpCommon.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <optional>

#include <BOPAlgo_Operation.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_BooleanOperation.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Message_ProgressRange.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <gp_Pnt.hxx>

#include "loop/FaceBuilder.h"
#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "ops/CancelProgress.h"
#include "sketch/WireSketch.h"

namespace onecad::ops {

namespace sk = onecad::core::sketch;
namespace loop = onecad::core::loop;
using nlohmann::json;

double read_scalar(const json& params, const char* key, double dflt) {
    if (!params.is_object() || !params.contains(key)) return dflt;
    const json& v = params[key];
    if (v.is_number()) return v.get<double>();
    if (v.is_object() && v.contains("value") && v["value"].is_number()) {
        return v["value"].get<double>();
    }
    return dflt;
}

std::string read_str(const json& o, const char* key, const std::string& dflt) {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return dflt;
}

std::optional<TopoDS_Face> build_profile_face(const json& sketch_params,
                                              const std::string& region_id, std::string& err) {
    // Sketch params → live Sketch (plane + entities + constraints). Mirrors
    // RegenerationEngine.cpp:1639-1667 buildFaceFromSketchRegion, but the sketch
    // is supplied inline in the plan (deterministic replay) rather than looked up
    // from a document.
    wire::TranslateResult tr = wire::translate(sketch_params);
    if (!tr.ok) {
        err = "profile: " + tr.error;
        return std::nullopt;
    }
    const sk::SolveResult solve = tr.sketch->solve();
    if (!solve.success) {
        err = "profile: solve failed: " +
              (solve.errorMessage.empty() ? std::string("constraint solve did not converge")
                                          : solve.errorMessage);
        return std::nullopt;
    }

    loop::LoopDetector detector;
    detector.setConfig(loop::makeRegionDetectionConfig());
    const loop::LoopDetectionResult det = detector.detect(*tr.sketch);
    const auto map_edge = [&](const sk::EntityID& internalId) {
        const auto it = tr.index.internal_edge_to_wire.find(internalId);
        return it != tr.index.internal_edge_to_wire.end() ? it->second : internalId;
    };
    const loop::RegionTable table = loop::buildRegionTable(
        det, map_edge, sk::constants::COINCIDENCE_TOLERANCE);
    if (!table.success) {
        err = "profile: " + table.errorMessage;
        return std::nullopt;
    }
    if (table.regions.empty()) {
        err = "profile: no closed region detected in sketch";
        return std::nullopt;
    }

    const loop::RegionDefinition* selected = &table.regions.front();
    if (!region_id.empty()) {
        std::vector<const loop::RegionDefinition*> matches;
        std::vector<std::string> available;
        available.reserve(table.regions.size());
        for (const loop::RegionDefinition& region : table.regions) {
            available.push_back(region.id);
            if (region.id == region_id || region.legacyId == region_id) {
                matches.push_back(&region);
            }
        }
        if (matches.size() != 1) {
            std::string avail;
            for (std::size_t i = 0; i < available.size(); ++i) {
                if (i) avail += ", ";
                avail += available[i];
            }
            const std::string reason =
                matches.empty() ? "matched no selectable region" : "is ambiguous";
            err = "profile: regionId '" + region_id + "' " + reason +
                  " (available: [" + avail + "])";
            return std::nullopt;
        }
        selected = matches.front();
    }

    loop::Face face;
    face.outerLoop = selected->outerLoop;
    face.innerLoops = selected->holes;

    loop::FaceBuilder builder;
    const loop::FaceBuildResult fr = builder.buildFace(face, *tr.sketch);
    if (!fr.success || fr.face.IsNull()) {
        err = "profile: " + (fr.errorMessage.empty() ? std::string("face build failed") : fr.errorMessage);
        return std::nullopt;
    }
    return fr.face;
}

bool planar_face_plane_normal(const TopoDS_Face& face, gp_Pln& plane_out, gp_Dir& normal_out) {
    // RegenerationEngine.cpp:201-219 planarFacePlaneAndNormal.
    try {
        if (face.IsNull()) return false;
        BRepAdaptor_Surface surface(face, true);
        if (surface.GetType() != GeomAbs_Plane) return false;
        plane_out = surface.Plane();
        normal_out = plane_out.Axis().Direction();
        if (face.Orientation() == TopAbs_REVERSED) normal_out.Reverse();
        return true;
    } catch (...) {
        return false;
    }
}

namespace {
BOPAlgo_Operation bop_of(app::BooleanMode mode) {
    switch (mode) {
        case app::BooleanMode::Add: return BOPAlgo_FUSE;
        case app::BooleanMode::Cut: return BOPAlgo_CUT;
        case app::BooleanMode::Intersect: return BOPAlgo_COMMON;
        default: return BOPAlgo_UNKNOWN;
    }
}
}  // namespace

BooleanResult checked_boolean(const TopoDS_Shape& target, const TopoDS_Shape& tool,
                              app::BooleanMode mode, bool parallel, const json& occt_options,
                              const onecad::CancelToken* cancel,
                              std::shared_ptr<BRepBuilderAPI_MakeShape>& builder_out) {
    BooleanResult out;
    if (target.IsNull() || tool.IsNull()) {
        out.error_code = "OP_FAILED";
        out.error_message = "boolean input is null";
        return out;
    }
    const BOPAlgo_Operation bop = bop_of(mode);
    if (bop == BOPAlgo_UNKNOWN) {
        out.error_code = "OP_FAILED";
        out.error_message = "unsupported boolean mode";
        return out;
    }

    // General boolean via BRepAlgoAPI_BooleanOperation (SetOperation) so we can
    // apply determinism + occtOptions BEFORE Build and keep the builder alive for
    // OCCT history. Semantics match RegenerationEngine.cpp:144-199 (IsDone → fail,
    // invalid → fail), plus cancellation via CancelProgress.
    auto algo = std::make_shared<BRepAlgoAPI_BooleanOperation>();
    TopTools_ListOfShape args, tools;
    args.Append(target);
    tools.Append(tool);
    algo->SetArguments(args);
    algo->SetTools(tools);
    algo->SetOperation(bop);
    // Determinism: single-threaded in determinism mode (Invariant 5). §7.3
    // occtOptions apply to both modes.
    algo->SetRunParallel(parallel ? Standard_True : Standard_False);
    if (occt_options.is_object()) {
        if (occt_options.contains("fuzzyValue") && occt_options["fuzzyValue"].is_number()) {
            const double fuzz = occt_options["fuzzyValue"].get<double>();
            if (fuzz > 0.0) algo->SetFuzzyValue(fuzz);
        }
        if (occt_options.contains("useOBB") && occt_options["useOBB"].is_boolean()) {
            algo->SetUseOBB(occt_options["useOBB"].get<bool>() ? Standard_True : Standard_False);
        }
    }

    try {
        Message_ProgressRange range;
        Handle(CancelProgress) pi;
        if (cancel) {
            pi = new CancelProgress(*cancel);
            range = pi->Start();
        }
        algo->Build(range);

        if (cancel && cancel->cancelled()) {
            out.error_code = "CANCELLED";
            out.error_message = "boolean cancelled";
            return out;
        }
        if (!algo->IsDone() || algo->HasErrors()) {
            out.error_code = "OP_FAILED";
            out.error_message = "boolean failed";
            return out;
        }
        const TopoDS_Shape result = algo->Shape();
        if (result.IsNull()) {
            out.error_code = "GEOMETRY_INVALID";
            out.error_message = "boolean produced null shape";
            return out;
        }
        BRepCheck_Analyzer analyzer(result);
        if (!analyzer.IsValid()) {
            out.error_code = "GEOMETRY_INVALID";
            out.error_message = "boolean produced invalid shape";
            return out;
        }
        out.shape = result;
        builder_out = algo;  // keep alive for history (upcast to MakeShape)
        return out;
    } catch (const Standard_Failure& f) {
        if (cancel && cancel->cancelled()) {
            out.error_code = "CANCELLED";
            out.error_message = "boolean cancelled";
        } else {
            out.error_code = "OP_FAILED";
            out.error_message = std::string("boolean raised: ") +
                                (f.GetMessageString() ? f.GetMessageString() : "OCCT failure");
        }
        return out;
    } catch (...) {
        out.error_code = "OP_FAILED";
        out.error_message = "boolean raised an unknown exception";
        return out;
    }
}

std::vector<RankedSolid> ranked_solids(const TopoDS_Shape& shape) {
    std::vector<RankedSolid> ranked;
    if (shape.IsNull()) return ranked;
    // Quantized geometric sort key (1e-6, matching the ElementMap quantization) so a
    // symmetric bisection (equal volumes) breaks the tie on centroid deterministically.
    //
    // VF-B6: the key is computed ONCE per solid here and RETAINED (it used to live
    // inside the comparator and was discarded), so the ordinal a child is minted at
    // can be published as tripwire evidence. `std::array` compares lexicographically
    // exactly like the `std::tuple` this replaced, and `stable_sort` + the `* 1e6`
    // quantization are unchanged — the resulting ordinal assignment is byte-identical
    // to the pre-VF-B6 order.
    auto q = [](double v) { return static_cast<std::int64_t>(std::llround(v * 1e6)); };
    for (TopExp_Explorer exp(shape, TopAbs_SOLID); exp.More(); exp.Next()) {
        const TopoDS_Shape& s = exp.Current();
        GProp_GProps props;
        BRepGProp::VolumeProperties(s, props);
        const gp_Pnt c = props.CentreOfMass();
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(s, TopAbs_FACE, faces);
        ranked.push_back(RankedSolid{
            s, session::RankKey{q(props.Mass()), q(c.X()), q(c.Y()), q(c.Z()),
                                static_cast<std::int64_t>(faces.Extent())}});
    }
    std::stable_sort(ranked.begin(), ranked.end(),
                     [](const RankedSolid& a, const RankedSolid& b) { return a.key < b.key; });
    return ranked;
}

std::vector<TopoDS_Shape> ordered_solids(const TopoDS_Shape& shape) {
    std::vector<TopoDS_Shape> solids;
    for (const RankedSolid& r : ranked_solids(shape)) solids.push_back(r.shape);
    return solids;
}

void publish_boolean_result(OpContext& ctx, const std::string& op_id,
                            const std::string& target_id, const TopoDS_Shape& result,
                            BRepBuilderAPI_MakeShape* builder, OpOutcome& out) {
    const std::vector<RankedSolid> solids = ranked_solids(result);
    if (solids.size() <= 1) {
        // Single body: modify the target in place (BodyId PRESERVED — corpus invariant).
        ctx.bodies.create(target_id, op_id, result);
        if (builder) {
            ctx.partition.apply_history(target_id, result, *builder, out.delta, &out.needs_repair);
        }
        // The rank key rides even the single-solid case (VF-B6): it costs the GProps
        // pass already done above, and it lets Rust anchor the op's stamp BEFORE a
        // later edit turns the same op into a split.
        std::optional<session::RankKey> key;
        if (solids.size() == 1) key = solids[0].key;
        out.body_events.push_back({"modified", target_id, key});
        out.body_ids.push_back(target_id);
        return;
    }
    // Split: the target is REPLACED by k deterministic children `body_<opId>:<k>`
    // (SCHEMA §2, D1). Emit a Deleted for the parent + a Created per child. The
    // parent's referenced-element partition entries are dropped (see the header).
    ctx.partition.remove_body(target_id, out.delta);
    ctx.bodies.erase(target_id);
    out.body_events.push_back({"deleted", target_id});
    for (std::size_t k = 0; k < solids.size(); ++k) {
        const std::string child_id = "body_" + op_id + ":" + std::to_string(k);
        ctx.bodies.create(child_id, op_id, solids[k].shape);
        // VF-B6: `k` IS this solid's geometric rank, so the key it was ranked by is
        // the evidence Rust needs to notice the rank flipping under a parametric edit.
        out.body_events.push_back({"created", child_id, solids[k].key});
        out.body_ids.push_back(child_id);
    }
}

}  // namespace onecad::ops
