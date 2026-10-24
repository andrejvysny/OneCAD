#include "ops/LoftOp.h"

#include <algorithm>
#include <cmath>
#include <set>
#include <utility>

#include <Approx_ParametrizationType.hxx>
#include <BRepGProp.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepTools.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <Precision.hxx>
#include <Standard_Failure.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Vec.hxx>

#include "kernel/validation/GeometryPrecision.h"
#include "ops/CancelProgress.h"
#include "ops/OpCommon.h"
#include "ops/TopologyHistory.h"

namespace onecad::ops {
namespace {
using nlohmann::json;

struct Section {
    TopoDS_Wire wire;
    gp_Pln plane;
    double resolution = 0.0;
};

bool finite_numbers(const json& value) {
    if (value.is_number()) return std::isfinite(value.get<double>());
    if (value.is_structured()) {
        for (const auto& child : value) {
            if (!finite_numbers(child)) return false;
        }
    }
    return true;
}

OpOutcome validate_params(const json& params, bool& ruled) {
    if (!finite_numbers(params)) return OpOutcome::fail("OP_FAILED", "Loft: nonfinite parameter");
    const std::string mode = read_str(params, "booleanMode", "NewBody");
    if (params.contains("booleanMode") && !params["booleanMode"].is_string()) {
        return OpOutcome::fail("OP_FAILED", "Loft: malformed booleanMode");
    }
    if (mode != "NewBody") return OpOutcome::unsupported("Loft: only NewBody is supported");
    bool solid = true;
    std::string error;
    if (!read_bool_strict(params, "isSolid", true, solid, error) ||
        !read_bool_strict(params, "isRuled", false, ruled, error)) {
        return OpOutcome::fail("OP_FAILED", "Loft: " + error);
    }
    if (!solid) return OpOutcome::unsupported("Loft: only solid sections are supported");
    for (const char* key : {"sections", "guideRails", "guides", "rails", "spine", "startCondition",
                            "endCondition", "continuity", "points"}) {
        if (params.contains(key)) return OpOutcome::unsupported(std::string("Loft: unsupported ") + key);
    }
    if (!params.contains("profiles") || !params["profiles"].is_array() ||
        params["profiles"].size() < 2 || params["profiles"].size() > 32) {
        return OpOutcome::fail("OP_FAILED", "Loft: profiles must contain 2..32 ordered sketch regions");
    }
    return {};
}

const json* find_sketch(const OpContext& ctx, const std::string& id) {
    if (ctx.sketches) {
        for (const auto& [sketch_id, params] : *ctx.sketches) {
            if (sketch_id == id) return &params;
        }
    }
    return nullptr;
}

OpOutcome section_face(const OpContext& ctx, const json& ref, TopoDS_Face& face,
                       std::vector<json>& diagnostics) {
    if (ref.is_object()) {
        for (const char* key : {"kind", "faceRef", "point", "vertex", "bodyId", "elementId"}) {
            if (ref.contains(key)) return OpOutcome::unsupported("Loft: only sketch region sections are supported");
        }
    }
    if (!ref.is_object() || !ref.contains("sketchId") || !ref["sketchId"].is_string() ||
        ref["sketchId"].get<std::string>().empty() || !ref.contains("regionId") ||
        !ref["regionId"].is_string() || ref["regionId"].get<std::string>().empty()) {
        return OpOutcome::fail("OP_FAILED", "Loft: each profile requires sketchId and regionId strings");
    }
    const json* sketch = find_sketch(ctx, ref["sketchId"].get<std::string>());
    if (!sketch) return OpOutcome::fail("REF_UNRESOLVED", "Loft: profile sketch not found in plan");
    if (!finite_numbers(*sketch)) return OpOutcome::fail("OP_FAILED", "Loft: nonfinite sketch data");
    std::optional<int> version;
    if (ref.contains("regionIdentityVersion")) {
        if (!ref["regionIdentityVersion"].is_number_integer() ||
            (ref["regionIdentityVersion"] != 2 && ref["regionIdentityVersion"] != 3)) {
            return OpOutcome::fail("OP_FAILED", "Loft: regionIdentityVersion must be 2 or 3");
        }
        version = ref["regionIdentityVersion"].get<int>();
    }
    std::optional<std::array<double, 2>> anchor;
    std::string error;
    if (!read_region_anchor(ref, anchor, error)) return OpOutcome::fail("OP_FAILED", "Loft: " + error);
    auto profile = build_profile_face(*sketch, ref["regionId"].get<std::string>(), version,
                                      error, &diagnostics, anchor);
    if (!profile) return OpOutcome::fail("OP_FAILED", "Loft: " + error);
    face = *profile;
    return {};
}

OpOutcome build_section(const OpContext& ctx, const json& ref, Section& section,
                        std::vector<json>& diagnostics) {
    TopoDS_Face face;
    auto result = section_face(ctx, ref, face, diagnostics);
    if (result.status != OpOutcome::Status::Ok) return result;
    int wires = 0;
    for (TopExp_Explorer it(face, TopAbs_WIRE); it.More(); it.Next()) ++wires;
    if (wires != 1) return OpOutcome::unsupported("Loft: sections with holes are not supported");
    section.wire = BRepTools::OuterWire(face);
    gp_Dir normal;
    if (section.wire.IsNull() || !BRep_Tool::IsClosed(section.wire) ||
        !planar_face_plane_normal(face, section.plane, normal)) {
        return OpOutcome::fail("OP_FAILED", "Loft: section must be one closed planar wire");
    }
    section.resolution = kernel::validation::precision_of(face).authoring_resolution();
    GProp_GProps area;
    BRepGProp::SurfaceProperties(face, area);
    if (!std::isfinite(area.Mass()) || area.Mass() <= section.resolution * section.resolution) {
        return OpOutcome::fail("OP_FAILED", "Loft: degenerate section area");
    }
    return {};
}

OpOutcome validate_order(const std::vector<Section>& sections) {
    const auto& first = sections.front();
    const gp_Dir axis = first.plane.Axis().Direction();
    double previous = 0.0;
    int direction = 0;
    for (std::size_t index = 1; index < sections.size(); ++index) {
        const auto& section = sections[index];
        if (!axis.IsParallel(section.plane.Axis().Direction(), Precision::Angular())) {
            return OpOutcome::unsupported("Loft: section planes must be parallel");
        }
        const double offset = gp_Vec(first.plane.Location(), section.plane.Location()).Dot(gp_Vec(axis));
        const double delta = offset - previous;
        const double resolution = std::max(section.resolution, sections[index - 1].resolution);
        if (!std::isfinite(offset) || std::abs(delta) <= resolution) {
            return OpOutcome::fail("OP_FAILED", "Loft: coincident section planes");
        }
        const int next_direction = delta > 0.0 ? 1 : -1;
        if (direction != 0 && direction != next_direction) {
            return OpOutcome::fail("OP_FAILED", "Loft: section offsets must be strictly monotone");
        }
        direction = next_direction;
        previous = offset;
    }
    return {};
}

OpOutcome publish(OpContext& ctx, const TopoDS_Shape& shape, const std::string& op_id) {
    auto policy = kernel::validation::single_solid_policy(
        "Loft", result_validation_tier(ctx, kernel::validation::PublicationTier::TierB));
    policy.maximum_tolerance = kernel::validation::precision_of(shape).authoring_resolution();
    const auto decision = publication_decision(shape, policy, ctx.cancel);
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    if (!decision.publishable()) return publication_refusal(decision, "publication");
    const std::string id = "body_" + op_id;
    ctx.bodies.create(id, op_id, shape);
    OpOutcome out;
    out.body_events.push_back({"created", id});
    out.body_ids.push_back(id);
    // NewBody certifies output births only; no source correspondence is inferred.
    out.topology_history.push_back(created_body_history(id, shape));
    return out;
}

OpOutcome loft_impl(OpContext& ctx, const json& op, const std::string& op_id,
                    std::vector<json>& diagnostics) {
    if (!op.contains("params") || !op["params"].is_object()) {
        return OpOutcome::fail("OP_FAILED", "Loft: params must be an object");
    }
    const auto& params = op["params"];
    bool ruled = false;
    auto result = validate_params(params, ruled);
    if (result.status != OpOutcome::Status::Ok) return result;
    std::set<std::pair<std::string, std::string>> seen;
    std::vector<Section> sections;
    for (const auto& ref : params["profiles"]) {
        if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
        Section section;
        result = build_section(ctx, ref, section, diagnostics);
        if (result.status != OpOutcome::Status::Ok) return result;
        if (!seen.emplace(read_str(ref, "sketchId"), read_str(ref, "regionId")).second) {
            return OpOutcome::fail("OP_FAILED", "Loft: duplicate sketch region section");
        }
        sections.push_back(section);
    }
    result = validate_order(sections);
    if (result.status != OpOutcome::Status::Ok) return result;
    BRepOffsetAPI_ThruSections builder(true, ruled, 1.0e-6);
    builder.SetMutableInput(false);
    builder.CheckCompatibility(true);
    builder.SetSmoothing(false);
    builder.SetParType(Approx_ChordLength);
    builder.SetContinuity(GeomAbs_C2);
    builder.SetMaxDegree(8);
    for (const auto& section : sections) builder.AddWire(section.wire);
    const CancelToken uncancelled;
    Handle(CancelProgress) progress = new CancelProgress(ctx.cancel ? *ctx.cancel : uncancelled);
    builder.Build(progress->Start());
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    if (!builder.IsDone() || builder.Shape().IsNull()) {
        return OpOutcome::fail("OP_FAILED", "Loft: OCCT section construction failed");
    }
    return publish(ctx, builder.Shape(), op_id);
}
}  // namespace

OpOutcome execute_loft(OpContext& ctx, const json& op, const std::string& op_id) {
    std::vector<json> diagnostics;
    OpOutcome out;
    try {
        out = loft_impl(ctx, op, op_id, diagnostics);
    } catch (const Standard_Failure& failure) {
        if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
        out = OpOutcome::fail("OP_FAILED", std::string("Loft failed: ") +
            (failure.GetMessageString() ? failure.GetMessageString() : "OCCT"));
    } catch (const json::exception&) {
        out = OpOutcome::fail("OP_FAILED", "Loft: malformed section data");
    }
    attach_profile_diagnostics(out, diagnostics);
    return out;
}
}  // namespace onecad::ops
