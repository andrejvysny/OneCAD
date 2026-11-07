#include "ops/SweepOp.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <optional>
#include <string>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepTools.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_CurveType.hxx>
#include <ElCLib.hxx>
#include <Precision.hxx>
#include <Standard_Failure.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Elips.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "kernel/validation/GeometryPrecision.h"
#include "loop/FaceBuilder.h"
#include "sketch/WireSketch.h"
#include "ops/CancelProgress.h"
#include "ops/OpCommon.h"
#include "ops/TopologyHistory.h"

namespace onecad::ops {
namespace {
using nlohmann::json;

bool finite_numbers(const json& value) {
    if (value.is_number()) return std::isfinite(value.get<double>());
    if (value.is_structured()) {
        for (const auto& child : value) if (!finite_numbers(child)) return false;
    }
    return true;
}

const json* find_sketch(const OpContext& ctx, const std::string& id) {
    if (ctx.sketches) {
        for (const auto& [key, value] : *ctx.sketches) if (key == id) return &value;
    }
    return nullptr;
}

struct PathRange {
    double start;
    double end;
};

OpOutcome validate_range(const json& params) {
    if (!params.contains("pathRange")) return OpOutcome::ok();
    const auto& range = params["pathRange"];
    if (!range.is_object() || range.size() != 2 || !range.contains("startRadians") ||
        !range.contains("endRadians") || !range["startRadians"].is_number() ||
        !range["endRadians"].is_number())
        return OpOutcome::fail("OP_FAILED", "Sweep: pathRange requires startRadians and endRadians only");
    const double start = range["startRadians"].get<double>();
    const double end = range["endRadians"].get<double>();
    const double turn = 2.0 * std::acos(-1.0);
    if (!std::isfinite(start) || !std::isfinite(end) || start < -turn || start > turn ||
        end < -turn || end > turn || !(start < end) || !(end - start < turn))
        return OpOutcome::fail("OP_FAILED", "Sweep: invalid open pathRange");
    return OpOutcome::ok();
}

OpOutcome validate_params(const json& params) {
    if (params.empty()) return OpOutcome::unsupported("Sweep: missing profile and path");
    if (!finite_numbers(params)) return OpOutcome::fail("OP_FAILED", "Sweep: nonfinite parameter");
    for (auto it = params.begin(); it != params.end(); ++it) {
        const auto& key = it.key();
        if (key != "sketchId" && key != "regionId" && key != "regionIdentityVersion" &&
            key != "regionAnchor" && key != "pathSketchId" && key != "booleanMode" &&
            key != "pathRange")
            return OpOutcome::unsupported("Sweep: unsupported parameter " + key);
    }
    for (const char* key : {"sketchId", "regionId", "pathSketchId"}) {
        if (!params.contains(key) || !params[key].is_string() || params[key].get<std::string>().empty())
            return OpOutcome::fail("OP_FAILED", std::string("Sweep: missing ") + key);
    }
    if (!params.contains("regionIdentityVersion") || !params["regionIdentityVersion"].is_number_integer() ||
        (params["regionIdentityVersion"] != 2 && params["regionIdentityVersion"] != 3))
        return OpOutcome::fail("OP_FAILED", "Sweep: regionIdentityVersion must be 2 or 3");
    if (!params.contains("booleanMode") || !params["booleanMode"].is_string())
        return OpOutcome::fail("OP_FAILED", "Sweep: missing booleanMode");
    if (params["booleanMode"] != "NewBody") return OpOutcome::unsupported("Sweep: only NewBody is supported");
    return validate_range(params);
}

bool read_vector(const json& value, gp_Vec& result) {
    if (!value.is_array() || value.size() != 3) return false;
    for (const auto& item : value) if (!item.is_number() || !std::isfinite(item.get<double>())) return false;
    result = gp_Vec(value[0].get<double>(), value[1].get<double>(), value[2].get<double>());
    return true;
}

OpOutcome validate_plane(const json& sketch) {
    if (!sketch.contains("plane")) return OpOutcome::ok();
    if (!sketch["plane"].is_object()) return OpOutcome::fail("OP_FAILED", "Sweep: malformed sketch plane");
    const auto& plane = sketch["plane"];
    if (!plane.contains("kind") || !plane["kind"].is_string())
        return OpOutcome::fail("OP_FAILED", "Sweep: malformed plane kind");
    const auto kind = plane["kind"].get<std::string>();
    if (kind != "custom") {
        if (kind != "XY" && kind != "XZ" && kind != "YZ")
            return OpOutcome::fail("OP_FAILED", "Sweep: unsupported plane kind");
        const bool has_vectors = plane.contains("origin") || plane.contains("xAxis") ||
                                 plane.contains("yAxis") || plane.contains("normal");
        if (!has_vectors) return OpOutcome::ok();
        gp_Vec origin, x, y, n;
        if (!plane.contains("origin") || !plane.contains("xAxis") || !plane.contains("yAxis") ||
            !plane.contains("normal") || !read_vector(plane["origin"], origin) ||
            !read_vector(plane["xAxis"], x) || !read_vector(plane["yAxis"], y) ||
            !read_vector(plane["normal"], n))
            return OpOutcome::fail("OP_FAILED", "Sweep: incomplete standard plane vectors");
        const gp_Vec expected_x = kind == "YZ" ? gp_Vec(-1, 0, 0) : gp_Vec(0, 1, 0);
        const gp_Vec expected_y = kind == "XY" ? gp_Vec(-1, 0, 0) : gp_Vec(0, 0, 1);
        const gp_Vec expected_n = kind == "XY" ? gp_Vec(0, 0, 1) :
                                  kind == "XZ" ? gp_Vec(1, 0, 0) : gp_Vec(0, 1, 0);
        // WireSketch uses the fixed preset for these kinds and ignores vectors.
        // Refuse any difference rather than silently changing authored geometry.
        if (origin.SquareMagnitude() != 0 || (x - expected_x).SquareMagnitude() != 0 ||
            (y - expected_y).SquareMagnitude() != 0 || (n - expected_n).SquareMagnitude() != 0)
            return OpOutcome::fail("OP_FAILED", "Sweep: standard plane vectors do not match preset");
        return OpOutcome::ok();
    }
    gp_Vec origin, x, y, n;
    if (!plane.contains("origin") || !plane.contains("xAxis") || !plane.contains("yAxis") ||
        !plane.contains("normal") || !read_vector(plane["origin"], origin) ||
        !read_vector(plane["xAxis"], x) || !read_vector(plane["yAxis"], y) ||
        !read_vector(plane["normal"], n))
        return OpOutcome::fail("OP_FAILED", "Sweep: custom plane requires four finite vectors");
    constexpr double kUnitDrift = 1e-8;
    if (std::abs(x.Magnitude() - 1) > kUnitDrift || std::abs(y.Magnitude() - 1) > kUnitDrift ||
        std::abs(n.Magnitude() - 1) > kUnitDrift || std::abs(x.Dot(y)) > kUnitDrift ||
        (x.Crossed(y) - n).Magnitude() > kUnitDrift)
        return OpOutcome::fail("OP_FAILED", "Sweep: custom plane must be right-handed and orthonormal");
    return OpOutcome::ok();
}

OpOutcome validate_path_sketch(const json& sketch) {
    if (!finite_numbers(sketch) || !sketch.contains("entities") || !sketch["entities"].is_array())
        return OpOutcome::fail("OP_FAILED", "Sweep: malformed path sketch");
    int curves = 0;
    for (const auto& entity : sketch["entities"]) {
        if (!entity.is_object() || !entity.contains("type") || !entity["type"].is_string())
            return OpOutcome::fail("OP_FAILED", "Sweep: malformed path entity");
        if (entity.contains("construction") && !entity["construction"].is_boolean())
            return OpOutcome::fail("OP_FAILED", "Sweep: malformed path construction flag");
        const auto type = entity["type"].get<std::string>();
        if (type == "Point") continue;
        if ((type != "Circle" && type != "Ellipse") || entity.value("construction", false))
            return OpOutcome::unsupported("Sweep: path must be one non-construction circle or ellipse");
        ++curves;
    }
    if (curves != 1) return OpOutcome::unsupported("Sweep: path must contain exactly one curve");
    return OpOutcome::ok();
}

OpOutcome one_closed_edge(const TopoDS_Face& face, TopoDS_Wire& wire, TopoDS_Edge& edge) {
    int wires = 0;
    for (TopExp_Explorer it(face, TopAbs_WIRE); it.More(); it.Next()) ++wires;
    if (wires != 1) return OpOutcome::unsupported("Sweep: holes and multiple wires are unsupported");
    wire = BRepTools::OuterWire(face);
    if (wire.IsNull() || !BRep_Tool::IsClosed(wire))
        return OpOutcome::fail("OP_FAILED", "Sweep: wire must be closed");
    int edges = 0;
    for (TopExp_Explorer it(wire, TopAbs_EDGE); it.More(); it.Next()) {
        ++edges;
        edge = TopoDS::Edge(it.Current());
    }
    if (edges != 1 || edge.IsNull()) return OpOutcome::unsupported("Sweep: exactly one analytic edge required");
    return OpOutcome::ok();
}

OpOutcome profile_and_path(const OpContext& ctx, const json& params, TopoDS_Face& profile,
                           TopoDS_Face& path, std::vector<json>& diagnostics) {
    const auto* profile_sketch = find_sketch(ctx, params["sketchId"].get<std::string>());
    const auto* path_sketch = find_sketch(ctx, params["pathSketchId"].get<std::string>());
    if (!profile_sketch || !path_sketch) return OpOutcome::fail("REF_UNRESOLVED", "Sweep: sketch not found in plan");
    if (!finite_numbers(*profile_sketch)) return OpOutcome::fail("OP_FAILED", "Sweep: nonfinite profile sketch");
    auto result = validate_plane(*profile_sketch);
    if (result.status != OpOutcome::Status::Ok) return result;
    result = validate_plane(*path_sketch);
    if (result.status != OpOutcome::Status::Ok) return result;
    result = validate_path_sketch(*path_sketch);
    if (result.status != OpOutcome::Status::Ok) return result;
    std::optional<std::array<double, 2>> anchor;
    std::string error;
    if (!read_region_anchor(params, anchor, error)) return OpOutcome::fail("OP_FAILED", "Sweep: " + error);
    auto p = build_profile_face(*profile_sketch, params["regionId"].get<std::string>(),
                                params["regionIdentityVersion"].get<int>(), error, &diagnostics, anchor);
    if (!p) return OpOutcome::fail("OP_FAILED", "Sweep: " + error);
    auto q = build_profile_face(*path_sketch, "", std::nullopt, error, &diagnostics);
    if (!q) return OpOutcome::fail("OP_FAILED", "Sweep: path " + error);
    profile = *p;
    path = *q;
    return OpOutcome::ok();
}

OpOutcome rephase_circle(const gp_Circ& circle, const gp_Pnt& center, double resolution,
                         TopoDS_Wire& path_wire) {
    // A circle's seam can be moved without changing its geometry. Reorient its
    // axis toward the authored section and build one full, untrimmed edge;
    // trimming a periodic edge across its original seam inflated OCCT vertex
    // tolerances on rotated circles.
    const gp_Vec radial(circle.Location(), center);
    if (radial.Magnitude() <= resolution)
        return OpOutcome::fail("OP_FAILED", "Sweep: degenerate circle seam");
    const gp_Circ rephased(gp_Ax2(circle.Location(), circle.Axis().Direction(), gp_Dir(radial)), circle.Radius());
    BRepBuilderAPI_MakeEdge edge(rephased);
    if (!edge.IsDone()) return OpOutcome::fail("OP_FAILED", "Sweep: cannot place analytic seam");
    BRepBuilderAPI_MakeWire wire(edge.Edge());
    if (!wire.IsDone() || !BRep_Tool::IsClosed(wire.Wire()))
        return OpOutcome::fail("OP_FAILED", "Sweep: analytic seam must produce a closed wire");
    path_wire = wire.Wire();
    return OpOutcome::ok();
}

OpOutcome open_path(const BRepAdaptor_Curve& curve, const PathRange& range,
                    const std::optional<gp_Circ>& circle, TopoDS_Wire& path_wire) {
    BRepBuilderAPI_MakeEdge edge = circle
        ? BRepBuilderAPI_MakeEdge(*circle, range.start, range.end)
        : BRepBuilderAPI_MakeEdge(curve.Ellipse(), range.start, range.end);
    if (!edge.IsDone()) return OpOutcome::fail("OP_FAILED", "Sweep: cannot build open analytic path");
    BRepBuilderAPI_MakeWire wire(edge.Edge());
    if (!wire.IsDone() || BRep_Tool::IsClosed(wire.Wire()))
        return OpOutcome::fail("OP_FAILED", "Sweep: pathRange must produce an open wire");
    path_wire = wire.Wire();
    return OpOutcome::ok();
}

OpOutcome ranged_circle(const OpContext& ctx, const json& params, const TopoDS_Edge& spine,
                        std::optional<gp_Circ>& circle) {
    if (!params.contains("pathRange")) return OpOutcome::ok();
    BRepAdaptor_Curve curve(spine);
    if (curve.GetType() != GeomAbs_Circle) return OpOutcome::ok();
    const auto* path = find_sketch(ctx, params["pathSketchId"].get<std::string>());
    if (!path) return OpOutcome::fail("REF_UNRESOLVED", "Sweep: path sketch not found");
    auto translated = wire::translate(*path);
    if (!translated.ok || !translated.sketch)
        return OpOutcome::fail("OP_FAILED", "Sweep: cannot translate path sketch plane");
    const gp_Pln plane = core::loop::FaceBuilder::sketchPlaneToGpPln(translated.sketch->getPlane());
    const gp_Circ original = curve.Circle();
    circle.emplace(gp_Ax2(original.Location(), plane.Axis().Direction(), plane.Position().XDirection()),
                   original.Radius());
    return OpOutcome::ok();
}

OpOutcome validate_geometry(const TopoDS_Face& profile, const TopoDS_Edge& section,
                            const TopoDS_Face& path, const TopoDS_Edge& spine,
                            TopoDS_Wire& path_wire, const std::optional<PathRange>& range,
                            const std::optional<gp_Circ>& circle) {
    BRepAdaptor_Curve section_curve(section);
    BRepAdaptor_Curve path_curve(spine);
    if (section_curve.GetType() != GeomAbs_Circle ||
        (path_curve.GetType() != GeomAbs_Circle && path_curve.GetType() != GeomAbs_Ellipse))
        return OpOutcome::unsupported("Sweep: section must be a circle and path a circle or ellipse");
    const double radius = section_curve.Circle().Radius();
    const double major = path_curve.GetType() == GeomAbs_Circle ? path_curve.Circle().Radius()
                                                                : path_curve.Ellipse().MajorRadius();
    const double minor = path_curve.GetType() == GeomAbs_Circle ? major : path_curve.Ellipse().MinorRadius();
    const double resolution = std::max(kernel::validation::precision_of(profile).authoring_resolution(),
                                       kernel::validation::precision_of(path).authoring_resolution());
    if (!std::isfinite(radius) || !std::isfinite(major) || !std::isfinite(minor) ||
        radius <= resolution || minor <= resolution || major < minor ||
        radius + resolution >= minor * (minor / major))
        return OpOutcome::fail("OP_FAILED", "Sweep: section radius violates path curvature clearance");
    gp_Pln plane;
    gp_Dir normal;
    if (!planar_face_plane_normal(profile, plane, normal))
        return OpOutcome::fail("OP_FAILED", "Sweep: profile plane is invalid");
    const gp_Pnt center = section_curve.Circle().Location();
    const double parameter = range ? range->start : (path_curve.GetType() == GeomAbs_Circle
        ? ElCLib::Parameter(path_curve.Circle(), center)
        : ElCLib::Parameter(path_curve.Ellipse(), center));
    if (!std::isfinite(parameter)) return OpOutcome::fail("OP_FAILED", "Sweep: invalid path parameter");
    gp_Pnt start;
    gp_Vec tangent;
    if (range && circle) ElCLib::D1(parameter, *circle, start, tangent);
    else path_curve.D1(parameter, start, tangent);
    if (tangent.Magnitude() <= resolution || center.Distance(start) > resolution ||
        !normal.IsParallel(gp_Dir(tangent), Precision::Angular()))
        return OpOutcome::fail("OP_FAILED", "Sweep: profile center and normal must match a path point and tangent");
    if (range) return open_path(path_curve, *range, circle, path_wire);
    if (path_curve.GetType() == GeomAbs_Ellipse) {
        const gp_Pnt original_start = path_curve.Value(path_curve.FirstParameter());
        if (center.Distance(original_start) > resolution)
            return OpOutcome::unsupported("Sweep: ellipse profile must start at its positive major-axis point");
        return OpOutcome::ok();
    }
    return rephase_circle(path_curve.Circle(), center, resolution, path_wire);
}

OpOutcome publish(OpContext& ctx, const TopoDS_Shape& shape, const std::string& op_id) {
    auto policy = kernel::validation::single_solid_policy(
        "Sweep", result_validation_tier(ctx, kernel::validation::PublicationTier::TierB));
    policy.maximum_tolerance = kernel::validation::precision_of(shape).authoring_resolution();
    const auto decision = publication_decision(shape, policy, ctx.cancel);
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    if (!decision.publishable()) return publication_refusal(decision, "publication");
    const std::string id = "body_" + op_id;
    ctx.bodies.create(id, op_id, shape);
    OpOutcome out;
    out.body_events.push_back({"created", id, std::nullopt, std::nullopt});
    out.body_ids.push_back(id);
    out.topology_history.push_back(created_body_history(id, shape));
    return out;
}

OpOutcome sweep_impl(OpContext& ctx, const json& op, const std::string& op_id,
                     std::vector<json>& diagnostics) {
    if (!op.contains("params") || !op["params"].is_object())
        return OpOutcome::fail("OP_FAILED", "Sweep: params must be an object");
    const auto& params = op["params"];
    if (op.contains("inputs") && (!op["inputs"].is_array() || !op["inputs"].empty()))
        return OpOutcome::unsupported("Sweep: semantic input references are unsupported");
    auto result = validate_params(params);
    if (result.status != OpOutcome::Status::Ok) return result;
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    TopoDS_Face profile, path;
    result = profile_and_path(ctx, params, profile, path, diagnostics);
    if (result.status != OpOutcome::Status::Ok) return result;
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    TopoDS_Wire section_wire, path_wire;
    TopoDS_Edge section, spine;
    result = one_closed_edge(profile, section_wire, section);
    if (result.status != OpOutcome::Status::Ok) return result;
    result = one_closed_edge(path, path_wire, spine);
    if (result.status != OpOutcome::Status::Ok) return result;
    std::optional<PathRange> range;
    if (params.contains("pathRange")) {
        const auto& value = params["pathRange"];
        range = PathRange{value["startRadians"].get<double>(), value["endRadians"].get<double>()};
    }
    std::optional<gp_Circ> circle;
    result = ranged_circle(ctx, params, spine, circle);
    if (result.status != OpOutcome::Status::Ok) return result;
    result = validate_geometry(profile, section, path, spine, path_wire, range, circle);
    if (result.status != OpOutcome::Status::Ok) return result;
    BRepOffsetAPI_MakePipeShell builder(path_wire);
    // Fixed Frenet frame; rotating a circular section about its own normal does
    // not change the swept set. No user-controlled orientation law is accepted.
    builder.SetMode(true);
    // Tightens OCCT's default 1e-4 mm approximation; authoring and output
    // publication still use the shared source-precision context, not this value.
    builder.SetTolerance(1e-7, 1e-7, 1e-2);
    builder.Add(section_wire, false, false);
    const CancelToken uncancelled;
    Handle(CancelProgress) progress = new CancelProgress(ctx.cancel ? *ctx.cancel : uncancelled);
    builder.Build(progress->Start());
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    if (!builder.IsDone() || !builder.MakeSolid() || builder.Shape().IsNull())
        return OpOutcome::fail("OP_FAILED", "Sweep: OCCT closed solid construction failed");
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    return publish(ctx, builder.Shape(), op_id);
}
}  // namespace

OpOutcome execute_sweep(OpContext& ctx, const json& op, const std::string& op_id) {
    std::vector<json> diagnostics;
    OpOutcome out;
    try {
        out = sweep_impl(ctx, op, op_id, diagnostics);
    } catch (const Standard_Failure& failure) {
        if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
        out = OpOutcome::fail("OP_FAILED", std::string("Sweep failed: ") +
            (failure.what() ? failure.what() : "OCCT"));
    } catch (const json::exception&) {
        out = OpOutcome::fail("OP_FAILED", "Sweep: malformed data");
    }
    attach_profile_diagnostics(out, diagnostics);
    return out;
}
}  // namespace onecad::ops
