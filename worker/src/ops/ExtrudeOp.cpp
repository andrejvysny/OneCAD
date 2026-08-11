// ExtrudeOp.cpp — see ExtrudeOp.h. Ports RegenerationEngine.cpp buildExtrude
// (:774-1059) incl. ToFace/ToNext end conditions (:858-894) + draft (:977-1013).
#include "ops/ExtrudeOp.h"

#include <algorithm>
#include <cmath>
#include <memory>
#include <optional>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBndLib.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <IntCurvesFace_ShapeIntersector.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Dir.hxx>
#include <gp_Lin.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "elementmap/Ladder.h"
#include "modeling/BooleanMode.h"
#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;
namespace em = onecad::elementmap;

namespace {

constexpr double kMinValue = 1e-3;   // RegenerationEngine.cpp:61 kMinValue (blind/two-sided distance guard)
constexpr double kToFaceMin = 1e-3;  // RegenerationEngine.cpp:61 kMinValue (ToFace coincidence)
constexpr double kThroughAllFallback = 1.0e5;    // RegenerationEngine.cpp:856
constexpr double kDraftAngleEpsilon = 1e-4;      // RegenerationEngine.cpp:59
constexpr double kSideFaceDotThreshold = 0.9;    // RegenerationEngine.cpp:60

double solid_volume(const TopoDS_Shape& shape) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props);
    return props.Mass();
}

std::string input_body(const json& op, std::size_t index) {
    if (!op.contains("inputs") || !op["inputs"].is_array() || op["inputs"].size() <= index) return "";
    const json& in = op["inputs"][index];
    if (in.is_object() && in.contains("primary") && in["primary"].is_object()) {
        // Only a whole-BODY ref is a valid boolean-target fallback. A face/edge ref
        // (e.g. a ToFace `targetFace` now placed at inputs[0]) must NOT be mistaken
        // for the operated body — binding the ToFace target's body as the boolean
        // target would silently cut/fuse the wrong body (M2 review hazard 6).
        if (read_str(in["primary"], "kind") != "body") return "";
        return read_str(in["primary"], "bodyId");
    }
    return "";
}

std::optional<app::BooleanMode> boolean_mode_of(const std::string& mode) {
    if (mode == "NewBody") return app::BooleanMode::NewBody;
    if (mode == "Add") return app::BooleanMode::Add;
    if (mode == "Cut") return app::BooleanMode::Cut;
    if (mode == "Intersect") return app::BooleanMode::Intersect;
    return std::nullopt;
}

bool valid_end_condition(const std::string& mode) {
    return mode == "Blind" || mode == "Symmetric" || mode == "ThroughAll" ||
           mode == "ToFace" || mode == "ToNext";
}

bool valid_optional_string(const json& params, const char* key) {
    return !params.contains(key) || params[key].is_string();
}

bool valid_optional_bool(const json& params, const char* key) {
    return !params.contains(key) || params[key].is_boolean();
}

bool valid_optional_scalar(const json& params, const char* key) {
    if (!params.contains(key)) return true;
    const json& value = params[key];
    if (value.is_number()) return std::isfinite(value.get<double>());
    return value.is_object() && value.contains("value") &&
           value["value"].is_number() &&
           std::isfinite(value["value"].get<double>());
}

const json* find_sketch(const OpContext& ctx, const json& params) {
    std::string sid = read_str(params, "sketchId");
    if (sid.empty() && ctx.last_sketch_id) sid = *ctx.last_sketch_id;
    if (!ctx.sketches) return nullptr;
    for (const auto& [id, p] : *ctx.sketches) {
        if (id == sid) return &p;
    }
    return nullptr;
}

double through_all_distance(double blind_sign_source, const gp_Pnt& origin, const gp_Dir& ref_dir,
                            const TopoDS_Shape* target) {
    const double sign = blind_sign_source >= 0.0 ? 1.0 : -1.0;
    const gp_Dir ray_dir = sign > 0.0 ? ref_dir : ref_dir.Reversed();
    if (target && !target->IsNull()) {
        Bnd_Box box;
        BRepBndLib::Add(*target, box);
        if (!box.IsVoid()) {
            Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
            box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
            double max_proj = 0.0;
            for (int corner = 0; corner < 8; ++corner) {
                const gp_Pnt p((corner & 1) ? xmax : xmin, (corner & 2) ? ymax : ymin,
                               (corner & 4) ? zmax : zmin);
                max_proj = std::max(max_proj, gp_Vec(origin, p).Dot(gp_Vec(ray_dir)));
            }
            const double diag = gp_Pnt(xmin, ymin, zmin).Distance(gp_Pnt(xmax, ymax, zmax));
            return sign * (std::max(max_proj, kMinValue) + 0.01 * diag + 1.0);
        }
    }
    return sign * kThroughAllFallback;
}

// Smallest positive distance along `dir` at which the PROFILE actually reaches a
// bounded face of `body`. The legacy port (RegenerationEngine.cpp:223-241) took the
// nearest ray-PLANE distance from one origin, which could bind a face plane the
// profile never crosses (review defect: ToNext onto a laterally offset pillar).
// Rays are cast from every profile wire vertex plus the area centroid against the
// body's bounded faces; -1 if nothing ahead is ever hit.
double to_next_distance(const TopoDS_Face& profile, const gp_Dir& dir, const TopoDS_Shape& body) {
    std::vector<gp_Pnt> samples;
    for (TopExp_Explorer exp(profile, TopAbs_VERTEX); exp.More(); exp.Next()) {
        samples.push_back(BRep_Tool::Pnt(TopoDS::Vertex(exp.Current())));
    }
    GProp_GProps props;
    BRepGProp::SurfaceProperties(profile, props);
    samples.push_back(props.CentreOfMass());

    IntCurvesFace_ShapeIntersector inter;
    inter.Load(body, 1e-7);
    double best = -1.0;
    for (const gp_Pnt& p : samples) {
        // Start past 1e-4 so a profile lying ON a body face skips its host face.
        inter.Perform(gp_Lin(p, dir), 1e-4, RealLast());
        if (!inter.IsDone()) continue;
        for (int i = 1; i <= inter.NbPnt(); ++i) {
            const double t = inter.WParameter(i);
            if (t > 1e-4 && (best < 0.0 || t < best)) best = t;
        }
    }
    return best;
}

TopoDS_Shape make_prism(const TopoDS_Shape& profile, const gp_Dir& dir, double signed_distance,
                        std::string& err) {
    gp_Vec vec(dir.X() * signed_distance, dir.Y() * signed_distance, dir.Z() * signed_distance);
    BRepPrimAPI_MakePrism prism(profile, vec, Standard_True);
    if (prism.Shape().IsNull()) {
        err = "Extrude prism produced null shape";
        return {};
    }
    return prism.Shape();
}

std::string invalid_shape_reason(const TopoDS_Shape& shape, const char* label) {
    if (shape.IsNull()) return std::string(label) + " produced null geometry";
    BRepCheck_Analyzer analyzer(shape);
    if (!analyzer.IsValid()) return std::string(label) + " produced invalid geometry";
    for (TopExp_Explorer exp(shape, TopAbs_SOLID); exp.More(); exp.Next()) return {};
    return std::string(label) + " produced no solid";
}

// ToFace target-distance resolution via the ladder (SCHEMA §7.3 typed targetFace).
struct ToFaceResolve {
    std::optional<double> distance;    // signed extrude distance to the target face
    std::optional<json> needs_repair;  // §9 STATE when the ref does not resolve
    std::string error;                 // hard error (e.g. non-planar / coincident)
};

ToFaceResolve resolve_to_face(OpContext& ctx, const json& face_ref, const gp_Pnt& origin,
                              const gp_Dir& ref_dir, const std::string& ref_id) {
    ToFaceResolve out;
    if (!face_ref.is_object()) {
        out.error = "ToFace requires a targetFace semantic ref";
        return out;
    }
    em::LadderRef r = em::ladder_ref_from_input(face_ref, ref_id);
    if (r.kind == em::km::ElementKind::Unknown) r.kind = em::km::ElementKind::Face;
    const std::string bid = (face_ref.contains("primary") && face_ref["primary"].is_object())
                                ? read_str(face_ref["primary"], "bodyId")
                                : "";
    const session::BodyRecord* rec = bid.empty() ? nullptr : ctx.bodies.get(bid);
    if (!rec) {
        // Unresolvable targetFace ⇒ NeedsRepair (Invariants 2/3; SCHEMA §7.3 rationale).
        out.needs_repair = json{{"refId", r.ref_id},
                                {"elementId", r.element_id},
                                {"ladderFailed", "descriptor"},
                                {"reason", "no-candidates"},
                                {"scoringVersion", em::kResolverVersion},
                                {"candidates", json::array()},
                                {"anchor", r.anchor_json.is_null() ? json::object() : r.anchor_json},
                                {"uiLabel", "ToFace target body not found: " + bid}};
        return out;
    }
    std::vector<em::LadderRef> refs{r};
    const std::vector<em::LadderResolution> res = em::resolve_descriptor_stage(
        rec->geom, bid, refs, em::LadderEditContext{ctx.post_upstream_edit, ctx.from_zero_replay});
    if (res.empty() || res[0].outcome != em::LadderOutcome::AutoBind || res[0].bound_shape.IsNull()) {
        out.needs_repair = res.empty() ? json::object() : res[0].to_needs_repair_json();
        return out;
    }
    gp_Pln target_pln;
    gp_Dir target_n;
    if (res[0].bound_shape.ShapeType() != TopAbs_FACE ||
        !planar_face_plane_normal(TopoDS::Face(res[0].bound_shape), target_pln, target_n)) {
        out.error = "ToFace target face is not planar";
        return out;
    }
    const double numerator =
        gp_Vec(origin, target_pln.Location()).Dot(gp_Vec(target_n));
    const double denominator = ref_dir.Dot(target_n);
    if (std::abs(denominator) < 1e-9) {
        out.error = "ToFace target plane is parallel to the extrude direction";
        return out;
    }
    const double distance = numerator / denominator;
    if (std::abs(distance) < kToFaceMin) {
        out.error = "ToFace target coincides with the sketch plane";
        return out;
    }
    out.distance = distance;
    return out;
}

// Apply draft to side faces. A requested draft must succeed; silently returning
// an undrafted solid would make preview/commit claim parameters were honored.
std::optional<TopoDS_Shape> apply_draft(const TopoDS_Shape& shape,
                                        double draft_angle_deg,
                                        const gp_Pln& plane,
                                        const gp_Dir& direction,
                                        double distance,
                                        std::string& error) {
    if (std::abs(draft_angle_deg) <= kDraftAngleEpsilon) return shape;
    try {
        const double angle_rad = draft_angle_deg * M_PI / 180.0;
        gp_Dir draft_dir = direction;
        if (distance < 0.0) draft_dir.Reverse();

        BRepOffsetAPI_DraftAngle draft(shape);
        const gp_Pln neutral_plane = plane;
        std::size_t eligible_faces = 0;
        std::size_t added_faces = 0;
        for (TopExp_Explorer exp(shape, TopAbs_FACE); exp.More(); exp.Next()) {
            const TopoDS_Face face = TopoDS::Face(exp.Current());
            BRepAdaptor_Surface surf(face, true);
            if (surf.GetType() != GeomAbs_Plane) continue;
            gp_Dir face_normal = surf.Plane().Axis().Direction();
            if (face.Orientation() == TopAbs_REVERSED) face_normal.Reverse();
            if (std::abs(face_normal.Dot(draft_dir)) > kSideFaceDotThreshold) continue;  // top/bottom
            ++eligible_faces;
            draft.Add(face, draft_dir, angle_rad, neutral_plane, true);
            if (!draft.AddDone()) {
                draft.Remove(face);
            } else {
                ++added_faces;
            }
        }
        if (eligible_faces == 0) {
            error = "Extrude draft refused: no eligible planar side faces";
            return std::nullopt;
        }
        if (added_faces == 0) {
            error = "Extrude draft refused: no eligible side faces accepted";
            return std::nullopt;
        }
        draft.Build();
        if (draft.IsDone() && !draft.Shape().IsNull()) {
            const double before = solid_volume(shape);
            const double after = solid_volume(draft.Shape());
            const double tolerance = std::max(1e-9, std::abs(before) * 1e-10);
            if (std::abs(after - before) > tolerance) return draft.Shape();
            error = "Extrude draft refused: draft left shape unchanged";
            return std::nullopt;
        }
        error = "Extrude draft failed";
    } catch (const Standard_Failure& failure) {
        error = std::string("Extrude draft failed: ") +
                (failure.GetMessageString() ? failure.GetMessageString() : "OCCT");
    } catch (...) {
        error = "Extrude draft failed";
    }
    return std::nullopt;
}

}  // namespace

OpOutcome execute_extrude(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    if (!valid_optional_string(params, "extrudeMode") ||
        !valid_optional_string(params, "extrudeMode2") ||
        !valid_optional_string(params, "booleanMode") ||
        !valid_optional_bool(params, "twoDirections") ||
        !valid_optional_scalar(params, "distance") ||
        !valid_optional_scalar(params, "distance2") ||
        !valid_optional_scalar(params, "draftAngleDeg")) {
        return OpOutcome::fail("OP_FAILED", "Extrude: malformed parameter type");
    }
    const std::string mode_str = read_str(params, "extrudeMode", "Blind");
    const std::string mode2_str = read_str(params, "extrudeMode2", "Blind");
    const bool two_dirs = params.value("twoDirections", false);
    const std::string boolean_mode_str = read_str(params, "booleanMode", "NewBody");
    const std::optional<app::BooleanMode> boolean_mode =
        boolean_mode_of(boolean_mode_str);
    if (!boolean_mode) {
        return OpOutcome::fail("OP_FAILED",
                               "Extrude: unknown boolean mode '" +
                                   boolean_mode_str + "'");
    }
    if (!valid_end_condition(mode_str)) {
        return OpOutcome::fail(
            "OP_FAILED", "Extrude: unknown end condition '" + mode_str + "'");
    }
    if (!valid_end_condition(mode2_str)) {
        return OpOutcome::fail(
            "OP_FAILED", "Extrude: unknown second end condition '" +
                             mode2_str + "'");
    }

    // --- profile face ---
    const json* sketch_params = find_sketch(ctx, params);
    if (!sketch_params) {
        return OpOutcome::fail("REF_UNRESOLVED", "Extrude: profile sketch not found in plan");
    }
    std::string perr;
    std::optional<TopoDS_Face> profile =
        build_profile_face(*sketch_params, read_str(params, "regionId"), perr);
    if (!profile) return OpOutcome::fail("OP_FAILED", perr);

    gp_Pln plane;
    gp_Dir direction(0, 0, 1);
    if (!planar_face_plane_normal(*profile, plane, direction)) {
        return OpOutcome::fail("OP_FAILED", "Extrude: only planar profile faces supported");
    }
    const gp_Pnt origin = plane.Location();
    // Boolean/ThroughAll/ToNext reference body (explicit param, else input body ref).
    std::string target_id = read_str(params, "targetBodyId");
    if (target_id.empty()) target_id = input_body(op, 0);
    const session::BodyRecord* ref_rec = target_id.empty() ? nullptr : ctx.bodies.get(target_id);
    const session::BodyRecord* target_rec =
        (*boolean_mode != app::BooleanMode::NewBody) ? ref_rec : nullptr;
    const TopoDS_Shape* ref_shape = ref_rec ? &ref_rec->geom : nullptr;

    const double distance = read_scalar(params, "distance", 10.0);
    const double distance2 = read_scalar(params, "distance2", 0.0);
    const double draft_angle = read_scalar(params, "draftAngleDeg", 0.0);
    if (!std::isfinite(distance) || !std::isfinite(distance2) ||
        !std::isfinite(draft_angle)) {
        return OpOutcome::fail("OP_FAILED", "Extrude parameters must be finite");
    }
    const bool distance_driven = !two_dirs && (mode_str == "Blind" || mode_str == "Symmetric");
    if (distance_driven && std::abs(distance) < kMinValue) {
        return OpOutcome::fail("OP_FAILED", "Extrude distance too small");
    }
    if (two_dirs && mode_str == "Blind" && std::abs(distance) < kMinValue) {
        return OpOutcome::fail("OP_FAILED", "Extrude first distance too small");
    }
    if (two_dirs && mode2_str == "Blind" && std::abs(distance2) < kMinValue) {
        return OpOutcome::fail("OP_FAILED", "Extrude second distance too small");
    }

    // Resolve a signed extrude distance for one end condition + direction. ToFace
    // resolution can raise NeedsRepair (surfaced via `nr`) or a hard error (`err`).
    auto effective_distance = [&](const std::string& m, double blind, const gp_Dir& ref_dir,
                                  const json& face_ref, const std::string& ref_id,
                                  std::optional<json>& nr, std::string& err) -> std::optional<double> {
        if (m == "Blind" || m == "Symmetric") return blind;
        if (m == "ThroughAll") return through_all_distance(blind, origin, ref_dir, ref_shape);
        if (m == "ToFace") {
            ToFaceResolve tf = resolve_to_face(ctx, face_ref, origin, ref_dir, ref_id);
            if (tf.needs_repair) { nr = tf.needs_repair; return std::nullopt; }
            if (!tf.distance) { err = tf.error; return std::nullopt; }
            return tf.distance;
        }
        if (m == "ToNext") {
            if (!ref_shape) { err = "ToNext requires an existing target body"; return std::nullopt; }
            const double d = to_next_distance(*profile, ref_dir, *ref_shape);
            if (d <= 0.0) { err = "ToNext: no face found ahead of the extrude direction"; return std::nullopt; }
            return d;
        }
        err = "Extrude: unknown end condition '" + m + "'";
        return std::nullopt;
    };

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    // --- build the extrude tool shape ---
    TopoDS_Shape tool_shape;
    std::string err;
    std::optional<json> nr;
    try {
        if (two_dirs) {
            if (mode_str == "Symmetric" || mode2_str == "Symmetric") {
                return OpOutcome::fail("OP_FAILED", "Symmetric is not valid with two directions");
            }
            const gp_Dir dir2 = direction.Reversed();
            auto d1 = effective_distance(mode_str, distance, direction, params.value("targetFace", json()),
                                         op_id + ".targetFace", nr, err);
            if (nr) { OpOutcome o; o.needs_repair.push_back(std::move(*nr)); return o; }
            if (!d1) return OpOutcome::fail("OP_FAILED", err.empty() ? "Extrude: bad end condition" : err);
            auto d2 = effective_distance(mode2_str, distance2, dir2,
                                         params.value("targetFace2", json()), op_id + ".targetFace2", nr, err);
            if (nr) { OpOutcome o; o.needs_repair.push_back(std::move(*nr)); return o; }
            if (!d2) return OpOutcome::fail("OP_FAILED", err.empty() ? "Extrude: bad end condition" : err);
            TopoDS_Shape p1 = make_prism(*profile, direction, *d1, err);
            if (p1.IsNull()) return OpOutcome::fail("OP_FAILED", err);
            TopoDS_Shape p2 = make_prism(*profile, dir2, *d2, err);
            if (p2.IsNull()) return OpOutcome::fail("OP_FAILED", err);
            BRepAlgoAPI_Fuse fuse(p1, p2);
            fuse.Build();
            if (!fuse.IsDone()) return OpOutcome::fail("OP_FAILED", "Two-direction extrude fuse failed");
            tool_shape = fuse.Shape();
        } else if (mode_str == "Symmetric") {
            const double half = distance * 0.5;
            gp_Vec fwd(direction.X() * half, direction.Y() * half, direction.Z() * half);
            gp_Vec bwd = fwd.Reversed();
            BRepPrimAPI_MakePrism fwd_prism(*profile, fwd, Standard_True);
            BRepPrimAPI_MakePrism bwd_prism(*profile, bwd, Standard_True);
            if (fwd_prism.Shape().IsNull() || bwd_prism.Shape().IsNull()) {
                return OpOutcome::fail("OP_FAILED", "Symmetric extrude prism produced null shape");
            }
            BRepAlgoAPI_Fuse fuse(fwd_prism.Shape(), bwd_prism.Shape());
            fuse.Build();
            if (!fuse.IsDone()) return OpOutcome::fail("OP_FAILED", "Symmetric extrude fuse failed");
            tool_shape = fuse.Shape();
        } else {
            auto d1 = effective_distance(mode_str, distance, direction, params.value("targetFace", json()),
                                         op_id + ".targetFace", nr, err);
            if (nr) { OpOutcome o; o.needs_repair.push_back(std::move(*nr)); return o; }
            if (!d1) return OpOutcome::fail("OP_FAILED", err.empty() ? "Extrude: bad end condition" : err);
            tool_shape = make_prism(*profile, direction, *d1, err);
            if (tool_shape.IsNull()) return OpOutcome::fail("OP_FAILED", err);
        }

        const std::string shape_error =
            invalid_shape_reason(tool_shape, "Extrude");
        if (!shape_error.empty()) {
            return OpOutcome::fail("GEOMETRY_INVALID", shape_error);
        }

        // Draft (side faces only) — applied to the prism before the boolean.
        std::optional<TopoDS_Shape> drafted =
            apply_draft(tool_shape, draft_angle, plane, direction, distance, err);
        if (!drafted) return OpOutcome::fail("OP_FAILED", err);
        tool_shape = std::move(*drafted);
        const std::string drafted_error =
            invalid_shape_reason(tool_shape, "Extrude draft");
        if (!drafted_error.empty()) {
            return OpOutcome::fail("GEOMETRY_INVALID", drafted_error);
        }
    } catch (const Standard_Failure& f) {
        return OpOutcome::fail("OP_FAILED", std::string("Extrude failed: ") +
                                               (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
    } catch (...) {
        return OpOutcome::fail("OP_FAILED", "Extrude failed");
    }

    OpOutcome out;

    // --- boolean mode dispatch ---
    if (*boolean_mode == app::BooleanMode::NewBody) {
        const std::string bid = "body_" + op_id;
        ctx.bodies.create(bid, op_id, tool_shape);
        out.body_events.push_back({"created", bid, {}});  // no rankKey: no ordinal ranked
        out.body_ids.push_back(bid);
        return out;  // new body: no pre-existing partition entries → empty delta
    }

    if (target_id.empty()) {
        return OpOutcome::fail("OP_FAILED", "Extrude boolean requires a target body");
    }
    if (!target_rec) {
        return OpOutcome::fail("REF_UNRESOLVED", "Extrude target body not found: " + target_id);
    }
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    const TopoDS_Shape old_target = target_rec->geom;
    std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
    BooleanResult br = checked_boolean(old_target, tool_shape, *boolean_mode, ctx.parallel,
                                       ctx.occt_options, ctx.cancel, builder);
    if (br.error_code == "CANCELLED") return OpOutcome::cancelled();
    if (!br.error_code.empty()) return OpOutcome::fail(br.error_code, br.error_message);

    // A complete Cut/Intersect removes the target. Publishing an empty compound as
    // a modified body would leave an unmeshable ghost in the document.
    if (ordered_solids(br.shape).empty()) {
        ctx.partition.remove_body(target_id, out.delta);
        ctx.bodies.erase(target_id);
        out.body_events.push_back({"deleted", target_id, {}});  // no rankKey: no ordinal ranked
        return out;
    }

    // Publish the successor: a single-solid result modifies the target in place; a
    // multi-solid boolean-Cut splits into deterministic children (SCHEMA §2, D1).
    publish_boolean_result(ctx, op_id, target_id, br.shape, builder.get(), out);
    return out;
}

}  // namespace onecad::ops
