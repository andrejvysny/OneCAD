// RevolveOp.cpp — see RevolveOp.h. Ports RegenerationEngine.cpp buildRevolve.
#include "ops/RevolveOp.h"

#include <array>
#include <cmath>
#include <memory>
#include <optional>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepBuilderAPI_MakeShape.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <gp_Vec.hxx>
#include <gp_Pln.hxx>
#include <BRep_Tool.hxx>
#include <TopExp_Explorer.hxx>
#include <GeomAbs_CurveType.hxx>
#include <gp_Circ.hxx>
#include <gp_Elips.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Ax1.hxx>
#include <gp_Dir.hxx>
#include <gp_Lin.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "kernel/validation/GeometryPrecision.h"
#include "modeling/BooleanMode.h"
#include "ops/OpCommon.h"
#include "sketch/Sketch.h"
#include "sketch/SketchLine.h"
#include "sketch/SketchPoint.h"
#include "sketch/WireSketch.h"

namespace onecad::ops {

using nlohmann::json;
namespace em = onecad::elementmap;

namespace {

constexpr double kMinAngleDeg = 1e-3;  // RegenerationEngine.cpp:62 kMinAngleDeg

// An unknown mode is a MALFORMED RECORD, not a NewBody request: falling back would
// mint a fresh body where the user asked for a cut. Mirrors ExtrudeOp.cpp.
std::optional<app::BooleanMode> boolean_mode_of(const std::string& s) {
    if (s == "NewBody") return app::BooleanMode::NewBody;
    if (s == "Add") return app::BooleanMode::Add;
    if (s == "Cut") return app::BooleanMode::Cut;
    if (s == "Intersect") return app::BooleanMode::Intersect;
    return std::nullopt;
}

const json* find_sketch(const OpContext& ctx, const std::string& sid_in,
                        const std::string& fallback_last) {
    std::string sid = sid_in;
    if (sid.empty()) sid = fallback_last;
    if (!ctx.sketches) return nullptr;
    for (const auto& [id, p] : *ctx.sketches) {
        if (id == sid) return &p;
    }
    return nullptr;
}

std::string input_body(const json& op, std::size_t index) {
    if (!op.contains("inputs") || !op["inputs"].is_array() || op["inputs"].size() <= index) return "";
    const json& in = op["inputs"][index];
    if (in.is_object() && in.contains("primary") && in["primary"].is_object()) {
        // Only a whole-BODY ref is a valid boolean-target fallback — a face/edge ref
        // must never be mistaken for the operated body (M2 review hazard 6; mirrors
        // ExtrudeOp::input_body).
        if (read_str(in["primary"], "kind") != "body") return "";
        return read_str(in["primary"], "bodyId");
    }
    return "";
}

// Axis from a sketch line: map the line's 2D endpoints through the sketch plane
// into world space (RegenerationEngine.cpp:1134-1172). Returns false + fills `err`.
bool axis_from_sketch_line(const OpContext& ctx, const std::string& sketch_id,
                           const std::string& line_id, gp_Ax1& axis_out, std::string& err) {
    const json* sk_params = find_sketch(ctx, sketch_id, ctx.last_sketch_id ? *ctx.last_sketch_id : "");
    if (!sk_params) {
        err = "Revolve: axis sketch not found in plan";
        return false;
    }
    wire::TranslateResult tr = wire::translate(*sk_params);
    if (!tr.ok) {
        err = "Revolve: axis sketch translate failed: " + tr.error;
        return false;
    }
    // An unconverged solve leaves the seed positions in place, so the axis would be
    // built from geometry that does not satisfy the sketch's own constraints — and a
    // wrong-but-nonzero axis sails past the degenerate-length guard below. Same
    // discipline the profile already gets in OpCommon::build_profile_face.
    const core::sketch::SolveResult solve = tr.sketch->solve();
    if (!solve.success) {
        err = "Revolve: axis sketch solve failed: " +
              (solve.errorMessage.empty() ? std::string("constraint solve did not converge")
                                          : solve.errorMessage);
        return false;
    }
    auto it = tr.index.wire_to_internal.find(line_id);
    if (it == tr.index.wire_to_internal.end()) {
        err = "Revolve: axis line '" + line_id + "' not found in sketch";
        return false;
    }
    const auto* line = tr.sketch->getEntityAs<core::sketch::SketchLine>(it->second);
    if (!line) {
        err = "Revolve: axis reference '" + line_id + "' is not a line";
        return false;
    }
    const auto* sp = tr.sketch->getEntityAs<core::sketch::SketchPoint>(line->startPointId());
    const auto* ep = tr.sketch->getEntityAs<core::sketch::SketchPoint>(line->endPointId());
    if (!sp || !ep) {
        err = "Revolve: axis line has no endpoints";
        return false;
    }
    const core::sketch::Vec3d ws = tr.sketch->toWorld({sp->position().X(), sp->position().Y()});
    const core::sketch::Vec3d we = tr.sketch->toWorld({ep->position().X(), ep->position().Y()});
    const gp_Pnt origin(ws.x, ws.y, ws.z);
    const gp_Vec dir(we.x - ws.x, we.y - ws.y, we.z - ws.z);
    if (dir.Magnitude() < 1e-6) {
        err = "Revolve: degenerate axis line";
        return false;
    }
    axis_out = gp_Ax1(origin, gp_Dir(dir));
    return true;
}

// Axis from a straight body edge (RegenerationEngine.cpp:1173-1191).
bool axis_from_straight_edge(const TopoDS_Shape& sub, gp_Ax1& axis_out, std::string& err) {
    if (sub.IsNull() || sub.ShapeType() != TopAbs_EDGE) {
        err = "Revolve: axis edge is missing or deleted";
        return false;
    }
    BRepAdaptor_Curve curve(TopoDS::Edge(sub));
    if (curve.GetType() != GeomAbs_Line) {
        err = "Revolve: axis edge must be a straight line";
        return false;
    }
    const gp_Lin lin = curve.Line();
    axis_out = gp_Ax1(lin.Location(), lin.Direction());
    return true;
}

// The typed companion is authoritative whenever it is present. In particular, do
// NOT use `edgeId` as an ordinal fallback after the ladder misses: an upstream
// edit could make that ordinal name a different edge. Legacy payloads still take
// the separate, byte-compatible route below.
enum class TypedAxisStatus { Resolved, NeedsRepair, Refused };

TypedAxisStatus axis_from_typed_edge(const OpContext& ctx, const json& axis,
                                     const std::string& op_id, gp_Ax1& axis_out,
                                     json& needs_repair, std::string& err) {
    const json& edge_ref = axis["edgeRef"];
    if (!edge_ref.is_object() || !edge_ref.contains("primary") || !edge_ref["primary"].is_object()) {
        err = "Revolve: typed axis edgeRef requires primary edge binding";
        return TypedAxisStatus::Refused;
    }
    const json& primary = edge_ref["primary"];
    const std::string body_id = read_str(axis, "bodyId");
    const std::string ref_body = read_str(primary, "bodyId");
    if (body_id.empty() || ref_body.empty() || ref_body != body_id) {
        err = "Revolve: typed axis edge belongs to a different body";
        return TypedAxisStatus::Refused;
    }
    if (read_str(primary, "kind") != "edge") {
        err = "Revolve: typed axis ref must be an edge";
        return TypedAxisStatus::Refused;
    }

    em::LadderRef ref = em::ladder_ref_from_input(edge_ref, op_id + ".input0");
    if (ref.element_id.empty()) {
        err = "Revolve: typed axis edgeRef requires elementId";
        return TypedAxisStatus::Refused;
    }
    const session::BodyRecord* rec = ctx.bodies.get(body_id);
    if (!rec || rec->geom.IsNull()) {
        err = "Revolve: typed axis edge body not found: " + body_id;
        return TypedAxisStatus::Refused;
    }
    if (const em::PartitionEntry* entry = ctx.partition.find(ref.element_id)) {
        if (entry->body_id != body_id || entry->kind != em::km::ElementKind::Edge) {
            err = "Revolve: typed axis edge binding is foreign or not an edge";
            return TypedAxisStatus::Refused;
        }
        const TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(rec->geom, entry->topo_key);
        if (sub.IsNull() || sub.ShapeType() != TopAbs_EDGE) {
            needs_repair = json{{"refId", op_id + ".input0"},
                                {"elementId", ref.element_id},
                                {"ladderFailed", "history"},
                                {"reason", "no-candidates"},
                                {"candidates", json::array()},
                                {"anchor", ref.anchor_json},
                                {"uiLabel", "Revolve axis edge was deleted"}};
            return TypedAxisStatus::NeedsRepair;
        }
        return axis_from_straight_edge(sub, axis_out, err) ? TypedAxisStatus::Resolved
                                                           : TypedAxisStatus::Refused;
    }

    const std::vector<em::LadderResolution> resolved = em::resolve_descriptor_stage(
        rec->geom, body_id, {ref}, em::LadderEditContext{ctx.post_upstream_edit, ctx.from_zero_replay});
    if (resolved.empty() || resolved[0].outcome != em::LadderOutcome::AutoBind) {
        needs_repair = resolved.empty() ? json{{"refId", op_id + ".input0"},
                                               {"elementId", ref.element_id},
                                               {"ladderFailed", "descriptor"},
                                               {"reason", "no-candidates"},
                                               {"candidates", json::array()},
                                               {"anchor", ref.anchor_json},
                                               {"uiLabel", "Revolve axis edge could not be resolved"}}
                                         : resolved[0].to_needs_repair_json();
        return TypedAxisStatus::NeedsRepair;
    }
    return axis_from_straight_edge(resolved[0].bound_shape, axis_out, err)
               ? TypedAxisStatus::Resolved
               : TypedAxisStatus::Refused;
}

bool axis_from_legacy_edge(const OpContext& ctx, const std::string& body_id, const std::string& edge_id,
                           gp_Ax1& axis_out, std::string& err) {
    const session::BodyRecord* rec = ctx.bodies.get(body_id);
    if (!rec) {
        err = "Revolve: axis edge body not found: " + body_id;
        return false;
    }
    return axis_from_straight_edge(
        elementmap::ElementMapPartition::shape_for_topokey(rec->geom, edge_id), axis_out, err);
}

// WP-B: the profile may rebind by `regionAnchor`, which publishes a step warning.
// This function has many early returns, so it COLLECTS profile advisories and
// `execute_revolve` attaches them once, on a non-refused outcome only.
OpOutcome revolve_impl(OpContext& ctx, const json& op, const std::string& op_id,
                       std::vector<json>& profile_diagnostics) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    // Refuse a malformed boolean mode BEFORE any OCCT work — and never coerce a
    // non-string through `read_str`'s default.
    if (params.contains("booleanMode") && !params["booleanMode"].is_string()) {
        return OpOutcome::fail("OP_FAILED", "Revolve: malformed booleanMode");
    }
    const std::string boolean_mode_str = read_str(params, "booleanMode", "NewBody");
    const std::optional<app::BooleanMode> boolean_mode_opt = boolean_mode_of(boolean_mode_str);
    if (!boolean_mode_opt) {
        return OpOutcome::fail("OP_FAILED",
                               "Revolve: unknown boolean mode '" + boolean_mode_str + "'");
    }

    double angle_deg = 360.0;
    std::string angle_error;
    if (!read_scalar_strict(params, "angleDeg", 360.0, angle_deg, angle_error)) {
        return OpOutcome::fail("OP_FAILED", "Revolve " + angle_error);
    }
    if (std::abs(angle_deg) < kMinAngleDeg) {
        return OpOutcome::fail("OP_FAILED", "Revolve angle too small");
    }
    if (std::abs(angle_deg) > 360.0) {
        return OpOutcome::fail("OP_FAILED", "Revolve angleDeg must be within [-360, 360]");
    }
    const double angle_rad = angle_deg * M_PI / 180.0;  // no 360 special-case (parity)

    // --- profile face ---
    const json* sketch_params = find_sketch(ctx, read_str(params, "sketchId"),
                                            ctx.last_sketch_id ? *ctx.last_sketch_id : "");
    if (!sketch_params) {
        return OpOutcome::fail("REF_UNRESOLVED", "Revolve: profile sketch not found in plan");
    }
    std::optional<int> region_identity_version;
    if (params.contains("regionIdentityVersion")) {
        if (!params["regionIdentityVersion"].is_number_integer()) {
            return OpOutcome::fail("OP_FAILED", "Revolve: regionIdentityVersion must be an integer");
        }
        region_identity_version = params["regionIdentityVersion"].get<int>();
    }
    std::optional<std::array<double, 2>> region_anchor;
    std::string anchor_error;
    if (!read_region_anchor(params, region_anchor, anchor_error)) {
        return OpOutcome::fail("OP_FAILED", "Revolve: " + anchor_error);
    }
    std::string perr;
    std::optional<TopoDS_Face> profile =
        build_profile_face(*sketch_params, read_str(params, "regionId"),
                           region_identity_version, perr, &profile_diagnostics,
                           region_anchor);
    if (!profile) return OpOutcome::fail("OP_FAILED", perr);

    // --- axis ---
    gp_Ax1 axis;
    std::string aerr;
    bool axis_ok = false;
    if (params.contains("axis") && params["axis"].is_object()) {
        const json& ax = params["axis"];
        const std::string kind = read_str(ax, "kind", "none");
        if (kind == "sketchLine") {
            axis_ok = axis_from_sketch_line(ctx, read_str(ax, "sketchId"), read_str(ax, "lineId"),
                                            axis, aerr);
        } else if (kind == "edge") {
            if (ax.contains("edgeRef")) {
                json repair;
                const TypedAxisStatus status = axis_from_typed_edge(ctx, ax, op_id, axis, repair, aerr);
                if (status == TypedAxisStatus::NeedsRepair) {
                    OpOutcome out;
                    out.needs_repair.push_back(std::move(repair));
                    return out;
                }
                axis_ok = status == TypedAxisStatus::Resolved;
            } else {
                axis_ok = axis_from_legacy_edge(ctx, read_str(ax, "bodyId"), read_str(ax, "edgeId"),
                                                axis, aerr);
            }
        }
    }
    if (!axis_ok) {
        return OpOutcome::fail("OP_FAILED",
                               aerr.empty() ? "Revolve: could not resolve revolution axis" : aerr);
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    // --- profile-vs-axis classification (WP-E) ---
    // A profile that CROSSES its axis sweeps through itself: OCCT builds the
    // figure-8 solid, BRepCheck accepts it face by face, and the volume is positive,
    // so only a global self-interference pass would ever notice. Refuse it by name
    // before any kernel work: every point of the profile boundary must lie on ONE
    // side of the axis (measured in the profile plane; touching the axis is legal —
    // it makes the degenerate pole edges of a shaft or a sphere).
    {
        gp_Pln profile_pln;
        gp_Dir profile_n;
        if (planar_face_plane_normal(*profile, profile_pln, profile_n)) {
            const double resolution =
                kernel::validation::precision_of(*profile).authoring_resolution();
            // side(P) = ((P − O) × a) · n = (P − O) · w with w = a × n: the signed
            // in-plane offset from the axis, linear in P. Vertices alone are NOT
            // enough — a full circle has one vertex (its seam) and an arc's extremum
            // can lie between its endpoints — so every edge contributes its endpoints
            // plus the analytic extrema of a Circle/Ellipse (the parameters where
            // d side/dt = 0 that fall inside the edge's range), and a free-form curve
            // is sampled. Adversarial review 2026-09-02 measured the vertex-only form
            // missing every crossing circle profile.
            const gp_Vec w = gp_Vec(axis.Direction()).Crossed(gp_Vec(profile_n));
            auto side_of = [&](const gp_Pnt& p) { return gp_Vec(axis.Location(), p).Dot(w); };
            double lo = 0.0, hi = 0.0;
            bool any = false;
            auto take = [&](double side) {
                lo = any ? std::min(lo, side) : side;
                hi = any ? std::max(hi, side) : side;
                any = true;
            };
            constexpr double kTwoPi = 2.0 * 3.14159265358979323846;
            for (TopExp_Explorer exp(*profile, TopAbs_EDGE); exp.More(); exp.Next()) {
                const TopoDS_Edge edge = TopoDS::Edge(exp.Current());
                if (BRep_Tool::Degenerated(edge)) continue;
                BRepAdaptor_Curve curve(edge);
                const double first = curve.FirstParameter();
                const double last = curve.LastParameter();
                take(side_of(curve.Value(first)));
                take(side_of(curve.Value(last)));
                // Extremum parameters of a conic: side(t) = s_c + A cos t + B sin t.
                auto take_conic_extrema = [&](const gp_Pnt& centre, const gp_Dir& xdir,
                                              const gp_Dir& ydir, double rx, double ry) {
                    const double a = gp_Vec(xdir).Dot(w) * rx;
                    const double b = gp_Vec(ydir).Dot(w) * ry;
                    const double t_star = std::atan2(b, a);
                    for (const double t : {t_star, t_star + kTwoPi / 2.0}) {
                        // Fold t into [first, first + 2π) and keep it if it lies on the arc.
                        double tt = first + std::fmod(std::fmod(t - first, kTwoPi) + kTwoPi, kTwoPi);
                        if (tt <= last + 1.0e-9) take(side_of(curve.Value(tt)));
                    }
                    (void)centre;
                };
                switch (curve.GetType()) {
                    case GeomAbs_Line: break;
                    case GeomAbs_Circle: {
                        const gp_Circ c = curve.Circle();
                        take_conic_extrema(c.Location(), c.XAxis().Direction(),
                                           c.YAxis().Direction(), c.Radius(), c.Radius());
                        break;
                    }
                    case GeomAbs_Ellipse: {
                        const gp_Elips e = curve.Ellipse();
                        take_conic_extrema(e.Location(), e.XAxis().Direction(),
                                           e.YAxis().Direction(), e.MajorRadius(), e.MinorRadius());
                        break;
                    }
                    default: {
                        constexpr int kSamples = 128;
                        for (int k = 1; k < kSamples; ++k) {
                            const double t = first + (last - first) * k / kSamples;
                            take(side_of(curve.Value(t)));
                        }
                        break;
                    }
                }
            }
            if (any && lo < -resolution && hi > resolution) {
                const std::string message =
                    "Revolve profile crosses its axis (the sweep would pass through itself)";
                OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
                failure.diagnostics.push_back(
                    {{"severity", "error"},
                     {"code", "REVOLVE_PROFILE_CROSSES_AXIS"},
                     {"message", message},
                     {"stage", "classify"},
                     {"evidence", {{"revolve", {{"minSignedOffset", lo}, {"maxSignedOffset", hi},
                                                {"authoringResolution", resolution}}}}}});
                return failure;
            }
        }
    }

    // --- build the revolved tool shape ---
    TopoDS_Shape tool_shape;
    try {
        // A profile touching the axis raises Standard_ConstructionError here.
        BRepPrimAPI_MakeRevol revol(*profile, axis, angle_rad, Standard_True);
        if (!revol.IsDone() || revol.Shape().IsNull()) {
            return OpOutcome::fail("OP_FAILED", "Revolve operation failed");
        }
        tool_shape = revol.Shape();
    } catch (const Standard_Failure& f) {
        return OpOutcome::fail("OP_FAILED", std::string("Revolve failed: ") +
                                               (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
    } catch (...) {
        return OpOutcome::fail("OP_FAILED", "Revolve failed");
    }

    const app::BooleanMode boolean_mode = *boolean_mode_opt;

    OpOutcome out;
    if (boolean_mode == app::BooleanMode::NewBody) {
        // WP-E: committed NewBody at Tier B (self-interference), preview at Tier A.
        kernel::validation::PublicationPolicy policy = kernel::validation::single_solid_policy(
            "Revolve", result_validation_tier(ctx, kernel::validation::PublicationTier::TierB));
        policy.maximum_tolerance =
            kernel::validation::precision_of(tool_shape).authoring_resolution();
        const kernel::validation::PublicationDecision decision =
            publication_decision(tool_shape, policy, ctx.cancel);
        if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
        if (!decision.publishable()) {
            return publication_refusal(decision, "publication");
        }
        const std::string bid = "body_" + op_id;  // D1 worker-minted NewBody id
        ctx.bodies.create(bid, op_id, tool_shape);
        out.body_events.push_back({"created", bid});
        out.body_ids.push_back(bid);
        return out;
    }

    // Add / Cut / Intersect into a target body (id preserved).
    std::string target_id = read_str(params, "targetBodyId");
    if (target_id.empty()) target_id = input_body(op, 0);
    if (target_id.empty()) return OpOutcome::fail("OP_FAILED", "Revolve boolean requires a target body");
    const session::BodyRecord* target_rec = ctx.bodies.get(target_id);
    if (!target_rec) return OpOutcome::fail("REF_UNRESOLVED", "Revolve target body not found: " + target_id);
    if (auto invalid = validate_modeling_body(*target_rec, "Revolve", "target")) return *invalid;
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
    BooleanResult br = checked_boolean(target_rec->geom, tool_shape, boolean_mode, ctx.parallel,
                                       ctx.occt_options, ctx.cancel, builder);
    if (br.error_code == "CANCELLED") return OpOutcome::cancelled();
    if (!br.error_code.empty()) return OpOutcome::fail(br.error_code, br.error_message);
    if (auto refusal = boolean_result_policy(boolean_mode, target_rec->geom, br.shape, "Revolve",
                                             target_id, "REVOLVE_ADD_DISJOINT",
                                             "REVOLVE_EMPTY_RESULT")) {
        return *refusal;
    }
    kernel::validation::PublicationPolicy policy;
    policy.name = "Revolve boolean";
    policy.allowed_top_level_shapes = kernel::validation::TopLevelShapePolicy::SolidSet;
    policy.max_solid_count = -1;
    policy.tier = result_validation_tier(
        ctx, kernel::validation::PublicationTier::TierB);
    policy.require_closed_manifold =
        policy.tier == kernel::validation::PublicationTier::TierB;
    // WP-E: the same tolerance budget Extrude's boolean applies — grow from the
    // target's tolerance, ×2 plus slack, never unbounded.
    {
        const auto prec = kernel::validation::precision_of(target_rec->geom);
        policy.maximum_tolerance = prec.tolerance_ceiling(prec.input_tolerance, 2.0, 1.0e-6);
    }
    policy.allow_empty_lifecycle = true;
    const kernel::validation::PublicationDecision decision =
        publication_decision(br.shape, policy, ctx.cancel);
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    if (!decision.publishable() && !decision.lifecycle_only()) {
        return publication_refusal(decision, "publication");
    }

    // Publish the successor: a single-solid result modifies the target in place; a
    // multi-solid boolean result splits into deterministic children `body_<opId>:<k>`
    // (SCHEMA §2, §7.2, D1 — parity with ExtrudeOp/BooleanOp).
    if (publish_boolean_result(ctx, op_id, target_id, br.shape, builder.get(), out) ==
        BooleanPublishResult::Empty) {
        // Unreachable after boolean_result_policy; kept as a defensive terminal.
        return OpOutcome::fail("OP_FAILED", "Revolve boolean produced no solids");
    }
    return out;
}

}  // namespace

OpOutcome execute_revolve(OpContext& ctx, const json& op, const std::string& op_id) {
    std::vector<json> profile_diagnostics;
    OpOutcome out = revolve_impl(ctx, op, op_id, profile_diagnostics);
    attach_profile_diagnostics(out, profile_diagnostics);
    return out;
}

}  // namespace onecad::ops
