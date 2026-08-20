// ExtrudeOp.cpp — see ExtrudeOp.h. Ports RegenerationEngine.cpp buildExtrude
// (:774-1059) incl. ToFace/ToNext end conditions (:858-894) + draft (:977-1013).
#include "ops/ExtrudeOp.h"

#include <algorithm>
#include <cmath>
#include <memory>
#include <optional>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBndLib.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepGProp.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "elementmap/Ladder.h"
#include "kernel/validation/GeometryPrecision.h"
#include "modeling/BooleanMode.h"
#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;
namespace em = onecad::elementmap;

namespace {

// The blind/two-sided distance guard and the ToFace coincidence guard both now
// come from the precision context — `GeometryPrecisionContext::authoring_resolution()`.
// The value is unchanged (1e-3 mm, RegenerationEngine.cpp:61 kMinValue); it is no
// longer restated here, and the two guards no longer state it independently.
constexpr double kThroughAllFallback = 1.0e5;    // RegenerationEngine.cpp:856
constexpr double kDraftAngleEpsilon = 1e-4;      // RegenerationEngine.cpp:59
constexpr double kSideFaceDotThreshold = 0.9;    // RegenerationEngine.cpp:60
constexpr double kDraftSemanticAngleTolerance = 1.0e-6;  // radians
// The parallel/tilted SPLIT, not a precision budget: at or under this the target
// plane is perpendicular to the extrude direction and the constant-distance V1
// path applies verbatim; above it the height varies across the profile and the
// half-space trim below is the only exact answer.
constexpr double kToFaceAngularTolerance = 1.0e-7;        // radians
constexpr double kToFaceCoverageRelativeTolerance = 1.0e-8;
// How far the ThroughAll prism is built PAST the furthest point of the target
// plane before the trim cuts it back. Relative so a 0.01 mm feature is not swept
// a metre, with the authoring resolution as the floor so the overshoot is always
// a length the kernel can distinguish. A synthetic tool EXTENT, not a resolution
// (GeometryPrecision.h names `kThroughAllFallback` in the same exclusion).
constexpr double kToFaceTrimOvershootFactor = 0.05;
// Contact within this of the sketch plane is the profile's OWN SEAT (a sketch drawn
// on a face of the target body), not the "next" face. Absolute mm: this is a
// start-exclusion threshold, not a feature-resolution one, so it does not carry the
// scale sensitivity the model-wide precision contract still owes.
constexpr double kToNextContactEpsilon = 1.0e-4;

double solid_volume(const TopoDS_Shape& shape) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props);
    return props.Mass();
}

double face_area(const TopoDS_Shape& shape) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(shape, props);
    return std::abs(props.Mass());
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
            const double min_value =
                kernel::validation::precision_of(*target).authoring_resolution();
            return sign * (std::max(max_proj, min_value) + 0.01 * diag + 1.0);
        }
    }
    return sign * kThroughAllFallback;
}

// EXACT minimum of the linear functional `(p - origin)·dir` over `shape`, including
// extrema that fall in an EDGE or FACE interior. A `TopAbs_VERTEX` scan answers this
// only for polyhedra: on a curved face the lowest point along `dir` is generally
// interior (a horizontal cylinder's bottom generatrix carries no vertex at all), so
// the vertex-only form reported a LATER first contact than the geometry has.
//
// Reduced to a distance extremum against a gauge plane placed BEHIND the shape:
// every shape point is ahead of the plane, so `distance-to-plane == projection -
// planeLevel`, and `BRepExtrema_DistShapeShape` solves that minimum exactly for
// analytic and NURBS geometry alike. The gauge face is deliberately over-sized so the
// perpendicular foot of the extremum is provably inside its trim.
//
// `nullopt` means UNKNOWN — never a silent fallback to the weaker vertex answer.
// Committed modeling refuses UNKNOWN.
std::optional<double> minimum_directional_projection(const TopoDS_Shape& shape,
                                                     const gp_Pnt& origin,
                                                     const gp_Dir& dir) {
    Bnd_Box box;
    BRepBndLib::Add(shape, box);
    if (box.IsVoid()) return std::nullopt;
    Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    const gp_Vec axis(dir);
    const gp_Pnt centre((xmin + xmax) * 0.5, (ymin + ymax) * 0.5, (zmin + zmax) * 0.5);
    double lo = 0.0;
    for (int corner = 0; corner < 8; ++corner) {
        const gp_Pnt p((corner & 1) ? xmax : xmin, (corner & 2) ? ymax : ymin,
                       (corner & 4) ? zmax : zmin);
        const double proj = gp_Vec(origin, p).Dot(axis);
        if (corner == 0 || proj < lo) lo = proj;
    }
    const double diag = gp_Pnt(xmin, ymin, zmin).Distance(gp_Pnt(xmax, ymax, zmax));
    if (!std::isfinite(lo) || !std::isfinite(diag)) return std::nullopt;

    const double margin = 0.01 * diag + 1.0;
    const double gauge_level = lo - margin;
    const double centre_level = gp_Vec(origin, centre).Dot(axis);
    const gp_Pnt gauge_origin = centre.Translated(gp_Vec(dir) * (gauge_level - centre_level));
    const double half = diag + margin;

    try {
        BRepBuilderAPI_MakeFace gauge(gp_Pln(gauge_origin, dir), -half, half, -half, half);
        if (!gauge.IsDone()) return std::nullopt;
        BRepExtrema_DistShapeShape extrema(shape, gauge.Face());
        if (!extrema.IsDone() || extrema.NbSolution() < 1) return std::nullopt;
        const double value = extrema.Value();
        if (!std::isfinite(value)) return std::nullopt;

        // A vertex projection is an UPPER bound on the true minimum, so folding the
        // scan in can only tighten the answer. It is defence in depth, never the
        // primary evidence.
        double best = gauge_level + value;
        for (TopExp_Explorer exp(shape, TopAbs_VERTEX); exp.More(); exp.Next()) {
            const gp_Pnt point = BRep_Tool::Pnt(TopoDS::Vertex(exp.Current()));
            best = std::min(best, gp_Vec(origin, point).Dot(axis));
        }
        return best;
    } catch (const Standard_Failure&) {
        return std::nullopt;
    }
}

// The same extremum with the axis reversed: max_p (p−origin)·d == −min_p (p−origin)·(−d).
std::optional<double> maximum_directional_projection(const TopoDS_Shape& shape,
                                                     const gp_Pnt& origin,
                                                     const gp_Dir& dir) {
    const std::optional<double> reversed =
        minimum_directional_projection(shape, origin, dir.Reversed());
    if (!reversed) return std::nullopt;
    return -*reversed;
}

enum class ToNextStatus {
    Contact,     // `distance` is the proven termination
    NoContact,   // nothing lies ahead of the profile — an honest negative
    Unprovable,  // the extremum could not be established; refuse, never guess
};

struct ToNextResult {
    ToNextStatus status = ToNextStatus::NoContact;
    double distance = -1.0;
};

// The intersection run is SEATED when it starts at the sketch plane: the profile was
// drawn on (or inside) the target body. A run whose minimum is this close to the lift
// plane began there rather than being reached.
constexpr double kToNextSeatThreshold = 2.0 * kToNextContactEpsilon;

// Distance along `dir` at which the swept profile reaches the NEXT face of the bounded
// target body. A whole-profile boolean probe, not a finite set of rays: a tiny interior
// ledge must be found even when no profile vertex or centroid lies above it, AND the
// termination is measured exactly (see `minimum_directional_projection`) rather than
// sampled at the common's vertices.
//
// Two cases, because "next" is not simply "nearest":
//   * the profile starts in FREE SPACE — terminate at the first contact, the minimum of
//     the intersection;
//   * the profile is SEATED on the body (a sketch drawn on one of its faces, which is
//     how a through-pocket is normally authored) — the intersection begins at the
//     sketch plane, so its minimum is the seat itself. The next face is then where the
//     sweep LEAVES that material: the maximum of the seated run. A void that spans the
//     profile footprint splits the intersection into separate solids, so the seated run
//     ends at the void's near wall; a void the material wraps laterally leaves the run
//     CONNECTED, so the feature passes through it (pinned by
//     test_to_next_seated_over_a_closed_internal_void).
// The old vertex scan reproduced the seated case only by accident — the seat's vertices
// sit exactly at the sketch plane and were dropped by its epsilon filter, leaving the
// exit vertices as the smallest survivor.
ToNextResult to_next_distance(const TopoDS_Face& profile, const gp_Dir& dir,
                              const TopoDS_Shape& body) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(profile, props);
    const gp_Pnt origin = props.CentreOfMass();
    const double sweep_distance = through_all_distance(1.0, origin, dir, &body);
    if (!std::isfinite(sweep_distance) ||
        sweep_distance <= kernel::validation::precision_of(body).authoring_resolution())
        return {ToNextStatus::NoContact, -1.0};

    try {
        // Start the sweep just PAST the sketch plane. Excluding the profile's own seat
        // geometrically — rather than by discarding common vertices under an epsilon —
        // keeps the rule true for curved contact, where the relevant point is not a
        // vertex at all.
        gp_Trsf lift;
        lift.SetTranslation(gp_Vec(dir) * kToNextContactEpsilon);
        BRepBuilderAPI_Transform lifted(profile, lift, Standard_True);
        if (!lifted.IsDone() || lifted.Shape().IsNull())
            return {ToNextStatus::Unprovable, -1.0};
        BRepPrimAPI_MakePrism sweep(lifted.Shape(), gp_Vec(dir) * sweep_distance,
                                    Standard_True);
        if (sweep.Shape().IsNull()) return {ToNextStatus::Unprovable, -1.0};
        BRepAlgoAPI_Common common(sweep.Shape(), body);
        common.SetRunParallel(Standard_False);
        common.Build();
        if (!common.IsDone() || common.HasErrors() || common.Shape().IsNull())
            return {ToNextStatus::Unprovable, -1.0};

        // An empty intersection is an honest "nothing ahead", not a failure.
        TopExp_Explorer probe(common.Shape(), TopAbs_VERTEX);
        if (!probe.More()) return {ToNextStatus::NoContact, -1.0};

        // Each solid of the intersection is one contiguous material run. A tangential
        // contact yields no solid at all, so the whole shape is then the single run.
        std::vector<TopoDS_Shape> runs;
        for (TopExp_Explorer exp(common.Shape(), TopAbs_SOLID); exp.More(); exp.Next())
            runs.push_back(exp.Current());
        if (runs.empty()) runs.push_back(common.Shape());

        double seat_exit = -1.0;
        double first_contact = -1.0;
        for (const TopoDS_Shape& run : runs) {
            const std::optional<double> entry =
                minimum_directional_projection(run, origin, dir);
            if (!entry) return {ToNextStatus::Unprovable, -1.0};
            if (*entry > kToNextSeatThreshold) {
                if (first_contact < 0.0 || *entry < first_contact) first_contact = *entry;
                continue;
            }
            // Seated run — the exit is what terminates the feature, so this is the
            // only case that needs the far extremum too.
            const std::optional<double> exit =
                maximum_directional_projection(run, origin, dir);
            if (!exit) return {ToNextStatus::Unprovable, -1.0};
            seat_exit = std::max(seat_exit, *exit);
        }

        const double distance = seat_exit > 0.0 ? seat_exit : first_contact;
        if (!(distance > kToNextSeatThreshold)) return {ToNextStatus::NoContact, -1.0};
        return {ToNextStatus::Contact, distance};
    } catch (const Standard_Failure&) {
        return {ToNextStatus::Unprovable, -1.0};
    }
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

// Why a ToFace refusal carries its own code and evidence: the top-level failure
// is the §8 `OP_FAILED` taxonomy value for every one of them, so a caller that
// needs to tell "you picked a cylinder" from "your profile hangs off the edge of
// the face you picked" would otherwise have to match on message TEXT. Same shape
// as `DraftFailure` below, same reason.
struct ToFaceFailure {
    std::string code;
    std::string message;
    nlohmann::json evidence;
    std::string stage = "classify";
};

// A TILTED planar target terminates the prism on a plane that is not
// perpendicular to the extrude direction, so there is no single distance to
// extrude: the height varies linearly across the profile. The construction is
// build-long-then-trim — a ThroughAll-length prism cut back by the target
// plane's half-space — and this is the half-space, carried from resolution to
// the point the prism exists.
//
// WHY `BRepAlgoAPI_Common` WITH THE KEPT SIDE, NOT `Cut` WITH THE DISCARDED ONE.
// Both put an exact planar cap on the target plane (the trim solid's base face
// IS the target plane's `Geom_Plane`, so the cap OCCT reports is that surface,
// not a fitted one). They differ in what an UNDER-SIZED trim solid does. With
// `Common` the result is a subset of the prism by construction, so no fragment
// of the ThroughAll far cap can survive; the only residual failure is losing
// material, which the cap-area identity below detects exactly. With `Cut` the
// failure is the other way — the far cap 10^5 mm away survives while the
// on-plane cap still measures full size — and that needs a separate, weaker
// "no other cap exists" proof. One provable failure mode beats two.
struct ToFaceTrim {
    gp_Pln plane;                // the target face's support plane
    gp_Dir normal;               // its outward normal
    gp_Dir keep;                 // half-space normal pointing at the PROFILE side
    TopoDS_Face face;            // the BOUNDED target face — the containment proof
    double obliquity = 1.0;      // |extrudeDir · normal|: the exact section factor
    double profile_area = 0.0;   // area of the profile face, mm²
    std::string ref_id;          // evidence only
};

// ToFace target-distance resolution via the ladder (SCHEMA §7.3 typed targetFace).
struct ToFaceResolve {
    std::optional<double> distance;    // signed extrude distance / prism length
    std::optional<ToFaceTrim> trim;    // set ⇒ tilted: trim the prism to this plane
    std::optional<json> needs_repair;  // §9 STATE when the ref does not resolve
    std::string error;                 // hard error with no stable code
    std::optional<ToFaceFailure> failure;  // named refusal (code + evidence)
};

nlohmann::json to_face_evidence(const std::string& ref_id, nlohmann::json fields) {
    fields["refId"] = ref_id;
    return nlohmann::json{{"toFace", std::move(fields)}};
}

ToFaceResolve resolve_to_face(OpContext& ctx, const json& face_ref,
                              const TopoDS_Face& profile, const gp_Pnt& origin,
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
    if (res[0].bound_shape.ShapeType() != TopAbs_FACE) {
        out.error = "ToFace target did not resolve to a face";
        return out;
    }
    const TopoDS_Face target_face = TopoDS::Face(res[0].bound_shape);
    if (!planar_face_plane_normal(target_face, target_pln, target_n)) {
        const std::string message = "ToFace target face is not planar";
        out.failure = ToFaceFailure{"EXTRUDE_TO_FACE_TARGET_NOT_PLANAR", message,
                                    to_face_evidence(ref_id, {{"targetPlanar", false}}),
                                    "classify"};
        out.error = message;
        return out;
    }
    const double numerator =
        gp_Vec(origin, target_pln.Location()).Dot(gp_Vec(target_n));
    const double denominator = ref_dir.Dot(target_n);
    if (std::abs(denominator) < 1e-9) {
        const std::string message =
            "ToFace target plane is parallel to the extrude direction";
        out.failure = ToFaceFailure{
            "EXTRUDE_TO_FACE_DEGENERATE", message,
            to_face_evidence(ref_id, {{"directionDotNormal", denominator}}), "classify"};
        out.error = message;
        return out;
    }
    const double target_angle = std::acos(std::clamp(std::abs(denominator), 0.0, 1.0));
    if (target_angle > kToFaceAngularTolerance) {
        // ── TILTED PLANAR TARGET: exact variable-height termination ──────────
        //
        // The terminating height at a profile point p is
        //     h(p) = ((T0 − p)·n) / (d·n),
        // affine in p, so its extrema over the profile are the extrema of the
        // linear functional (p−origin)·n — which `minimum/maximum_directional_
        // projection` answer EXACTLY, edge and face interiors included. A curved
        // profile edge therefore cannot hide a point whose height sits outside
        // the range measured here, which is what the prism length depends on.
        const std::optional<double> proj_min =
            minimum_directional_projection(profile, origin, target_n);
        const std::optional<double> proj_max =
            maximum_directional_projection(profile, origin, target_n);
        if (!proj_min || !proj_max) {
            out.error = "ToFace could not measure the profile against the target plane";
            return out;
        }
        const double h_a = (numerator - *proj_min) / denominator;
        const double h_b = (numerator - *proj_max) / denominator;
        const double h_min = std::min(h_a, h_b);
        const double h_max = std::max(h_a, h_b);
        const double resolution =
            kernel::validation::precision_of(target_face, h_max - h_min)
                .authoring_resolution();
        // The whole profile must terminate on ONE side of the plane. A plane the
        // profile straddles, sits on, or has already passed has no variable-height
        // solid to build — going "backwards" over part of the footprint is not a
        // termination, it is an ambiguity, and the only honest answer is a refusal.
        const bool forward = h_min >= resolution;
        const bool backward = h_max <= -resolution;
        if (!forward && !backward) {
            const std::string message =
                "ToFace target plane does not lie wholly ahead of or behind the profile";
            out.failure = ToFaceFailure{
                "EXTRUDE_TO_FACE_DEGENERATE", message,
                to_face_evidence(ref_id, {{"minHeight", h_min},
                                          {"maxHeight", h_max},
                                          {"authoringResolution", resolution}}),
                "classify"};
            out.error = message;
            return out;
        }
        const double sign = forward ? 1.0 : -1.0;
        const double reach = std::max(std::abs(h_min), std::abs(h_max));
        // s(p) = (p−T0)·n = −h(p)·(d·n), so with the sign of h fixed above the
        // profile lies wholly on this side of the plane. The trim keeps it.
        const bool keep_along_normal = (-sign * denominator) > 0.0;
        out.distance = sign * (reach + kToFaceTrimOvershootFactor * reach + resolution);
        out.trim = ToFaceTrim{target_pln,
                              target_n,
                              keep_along_normal ? target_n : target_n.Reversed(),
                              target_face,
                              std::abs(denominator),
                              face_area(profile),
                              ref_id};
        return out;
    }
    const double distance = numerator / denominator;
    if (std::abs(distance) <
        kernel::validation::precision_of(target_face, distance).authoring_resolution()) {
        out.error = "ToFace target coincides with the sketch plane";
        return out;
    }

    // "Up to selected face" means the translated profile must be contained by the
    // bounded target face, not merely coplanar with its underlying surface.
    try {
        gp_Trsf translation;
        translation.SetTranslation(gp_Vec(ref_dir) * distance);
        BRepBuilderAPI_Transform moved(profile, translation, Standard_True);
        if (!moved.IsDone() || moved.Shape().IsNull()) {
            out.error = "ToFace could not project the profile onto the selected face";
            return out;
        }
        BRepAlgoAPI_Common common(moved.Shape(), target_face);
        common.SetRunParallel(Standard_False);
        common.Build();
        if (!common.IsDone() || common.HasErrors()) {
            out.error = "ToFace could not prove bounded-face coverage";
            return out;
        }
        const double expected_area = face_area(profile);
        const double covered_area = face_area(common.Shape());
        const double area_tolerance =
            std::max(1.0e-8, expected_area * kToFaceCoverageRelativeTolerance);
        if (!(expected_area > 0.0) ||
            std::abs(covered_area - expected_area) > area_tolerance) {
            // Message frozen (the V1 refusal text); the CODE is what a caller
            // routes on, and it is the same defect the tilted cap proof reports.
            const std::string message =
                "ToFace selected face does not cover the entire projected profile";
            out.failure = ToFaceFailure{
                "EXTRUDE_TO_FACE_NOT_COVERED", message,
                to_face_evidence(ref_id, {{"capArea", expected_area},
                                          {"coveredArea", covered_area},
                                          {"areaTolerance", area_tolerance}}),
                "classify"};
            out.error = message;
            return out;
        }
    } catch (const Standard_Failure& failure) {
        out.error = std::string("ToFace bounded-face proof failed: ") +
                    (failure.GetMessageString() ? failure.GetMessageString() : "OCCT");
        return out;
    }
    out.distance = distance;
    return out;
}

/// Wraps a named ToFace refusal in the `OP_FAILED` outcome plus its diagnostic.
OpOutcome to_face_refusal(const ToFaceFailure& failure) {
    OpOutcome out = OpOutcome::fail("OP_FAILED", failure.message);
    out.diagnostics.push_back({{"severity", "error"},
                               {"code", failure.code},
                               {"message", failure.message},
                               {"stage", failure.stage},
                               {"evidence", failure.evidence}});
    return out;
}

// The half-space solid that trims a ThroughAll prism back to the target plane.
// Sized from the prism's BOUNDING BOX, whose eight corners bound every affine
// functional over the prism — so "the trim solid contains the whole kept side"
// is proven arithmetic, not a generous constant.
TopoDS_Shape make_trim_half_space(const TopoDS_Shape& prism, const ToFaceTrim& trim) {
    Bnd_Box box;
    BRepBndLib::Add(prism, box);
    if (box.IsVoid()) return {};
    Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    const double diag = gp_Pnt(xmin, ymin, zmin).Distance(gp_Pnt(xmax, ymax, zmax));
    if (!std::isfinite(diag)) return {};

    const gp_Pnt anchor = trim.plane.Location();
    const gp_Vec keep(trim.keep);
    double depth_extent = 0.0;
    double lateral_extent = 0.0;
    for (int corner = 0; corner < 8; ++corner) {
        const gp_Pnt p((corner & 1) ? xmax : xmin, (corner & 2) ? ymax : ymin,
                       (corner & 4) ? zmax : zmin);
        const gp_Vec offset(anchor, p);
        const double along = offset.Dot(keep);
        depth_extent = std::max(depth_extent, along);
        lateral_extent = std::max(lateral_extent, (offset - keep * along).Magnitude());
    }
    const double margin =
        0.01 * diag + kernel::validation::precision_of(prism).authoring_resolution();
    const double half = lateral_extent + margin;
    const double depth = depth_extent + margin;
    if (!std::isfinite(half) || !std::isfinite(depth)) return {};

    BRepBuilderAPI_MakeFace base(gp_Pln(anchor, trim.keep), -half, half, -half, half);
    if (!base.IsDone()) return {};
    BRepPrimAPI_MakePrism slab(base.Face(), keep * depth, Standard_True);
    return slab.Shape();
}

// The terminating cap: every face of the trimmed solid whose support plane IS the
// target plane, within the SEMANTIC budgets (an angular and a length one — the
// question "did the requested modelling change happen?", not "what did OCCT
// manage to build"). Returned as one compound so the containment proof runs once.
struct CapEvidence {
    TopoDS_Shape shape;
    double area = 0.0;
    std::size_t face_count = 0;
};

CapEvidence collect_cap(const TopoDS_Shape& solid, const ToFaceTrim& trim,
                        const kernel::validation::GeometryPrecisionContext& precision) {
    CapEvidence out;
    TopoDS_Compound compound;
    BRep_Builder builder;
    builder.MakeCompound(compound);
    for (TopExp_Explorer exp(solid, TopAbs_FACE); exp.More(); exp.Next()) {
        const TopoDS_Face face = TopoDS::Face(exp.Current());
        gp_Pln candidate_plane;
        gp_Dir candidate_normal;
        if (!planar_face_plane_normal(face, candidate_plane, candidate_normal)) continue;
        if (!candidate_normal.IsParallel(trim.normal, precision.semantic_angular())) continue;
        if (std::abs(trim.plane.Distance(candidate_plane.Location())) >
            precision.semantic_length()) {
            continue;
        }
        builder.Add(compound, face);
        out.area += face_area(face);
        ++out.face_count;
    }
    out.shape = compound;
    return out;
}

// Build the exact variable-height solid for a tilted planar target and PROVE it.
//
// Three claims, in the order a failure is cheapest to explain:
//   1. material exists at all — an empty trim is the degenerate case;
//   2. the terminating cap lies ON the target plane and is the FULL oblique
//      section of the profile. `capArea · |d·n| == profileArea` is exact for a
//      planar section of a prism (the section projects onto the profile with
//      factor |d·n|), so it simultaneously proves the cap is on-plane, complete,
//      and that the trim solid was large enough;
//   3. the cap is contained by the BOUNDED target face — "up to the face you
//      picked", not "up to the infinite surface behind it". Same area-coverage
//      proof the parallel path runs, moved onto the oblique section.
std::optional<TopoDS_Shape> trim_prism_to_target(const TopoDS_Shape& prism,
                                                 const ToFaceTrim& trim,
                                                 std::optional<ToFaceFailure>& failure,
                                                 std::string& error) {
    try {
        const TopoDS_Shape half_space = make_trim_half_space(prism, trim);
        if (half_space.IsNull()) {
            error = "ToFace could not build the target half-space";
            return std::nullopt;
        }
        BRepAlgoAPI_Common trimmed(prism, half_space);
        trimmed.SetRunParallel(Standard_False);
        trimmed.Build();
        if (!trimmed.IsDone() || trimmed.HasErrors() || trimmed.Shape().IsNull()) {
            error = "ToFace could not trim the prism to the target plane";
            return std::nullopt;
        }
        const TopoDS_Shape result = trimmed.Shape();
        if (ordered_solids(result).empty()) {
            const std::string message = "ToFace termination leaves no material";
            failure = ToFaceFailure{"EXTRUDE_TO_FACE_DEGENERATE", message,
                                    to_face_evidence(trim.ref_id, {{"solids", 0}}), "build"};
            error = message;
            return std::nullopt;
        }

        const kernel::validation::GeometryPrecisionContext precision =
            kernel::validation::precision_of(result);
        const CapEvidence cap = collect_cap(result, trim, precision);
        const double expected_cap = trim.profile_area / trim.obliquity;
        const double cap_tolerance =
            std::max(1.0e-8, expected_cap * kToFaceCoverageRelativeTolerance);
        if (!(trim.profile_area > 0.0) ||
            std::abs(cap.area - expected_cap) > cap_tolerance) {
            // Not a user error and not a named refusal: the construction did not
            // produce the section it is defined to produce, so there is nothing
            // to advise the user to change.
            error = "ToFace could not prove the terminating cap is the full section "
                    "of the profile on the target plane";
            return std::nullopt;
        }

        BRepAlgoAPI_Common coverage(cap.shape, trim.face);
        coverage.SetRunParallel(Standard_False);
        coverage.Build();
        if (!coverage.IsDone() || coverage.HasErrors()) {
            error = "ToFace could not prove bounded-face coverage";
            return std::nullopt;
        }
        const double covered_area = face_area(coverage.Shape());
        if (std::abs(covered_area - cap.area) > cap_tolerance) {
            const std::string message =
                "ToFace selected face does not cover the entire terminating cap";
            failure = ToFaceFailure{
                "EXTRUDE_TO_FACE_NOT_COVERED", message,
                to_face_evidence(trim.ref_id, {{"capArea", cap.area},
                                               {"coveredArea", covered_area},
                                               {"areaTolerance", cap_tolerance},
                                               {"capFaces", cap.face_count}}),
                "build"};
            error = message;
            return std::nullopt;
        }
        return result;
    } catch (const Standard_Failure& f) {
        error = std::string("ToFace tilted termination failed: ") +
                (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return std::nullopt;
    }
}

// Why a draft refusal carries its own code and evidence: the top-level failure is
// the §8 `OP_FAILED` taxonomy value for every one of these, so a caller that needs
// to tell "this profile has no planar wall to draft" from "OCCT rejected the faces
// I offered" would otherwise have to match on message TEXT — the routing the
// diagnostics contract forbids. Codes follow the `EDGE_OP_*` precedent: stable
// per-defect string, `stage`, and bounded evidence naming the parameters involved.
struct DraftFailure {
    std::string code;
    std::string message;
    nlohmann::json evidence;
};

// Apply draft to side faces. A requested draft must succeed; silently returning
// an undrafted solid would make preview/commit claim parameters were honored.
std::optional<TopoDS_Shape> apply_draft(const TopoDS_Shape& shape,
                                        double draft_angle_deg,
                                        const gp_Pln& plane,
                                        const gp_Dir& direction,
                                        double distance,
                                        DraftFailure& failure) {
    if (std::abs(draft_angle_deg) <= kDraftAngleEpsilon) return shape;
    try {
        const double angle_rad = draft_angle_deg * M_PI / 180.0;
        gp_Dir draft_dir = direction;
        if (distance < 0.0) draft_dir.Reverse();

        BRepOffsetAPI_DraftAngle draft(shape);
        const gp_Pln neutral_plane = plane;
        std::vector<TopoDS_Face> eligible_faces;
        std::size_t added_faces = 0;
        for (TopExp_Explorer exp(shape, TopAbs_FACE); exp.More(); exp.Next()) {
            const TopoDS_Face face = TopoDS::Face(exp.Current());
            BRepAdaptor_Surface surf(face, true);
            if (surf.GetType() != GeomAbs_Plane) continue;
            gp_Dir face_normal = surf.Plane().Axis().Direction();
            if (face.Orientation() == TopAbs_REVERSED) face_normal.Reverse();
            if (std::abs(face_normal.Dot(draft_dir)) > kSideFaceDotThreshold) continue;  // top/bottom
            eligible_faces.push_back(face);
            draft.Add(face, draft_dir, angle_rad, neutral_plane, true);
            if (!draft.AddDone()) {
                draft.Remove(face);
            } else {
                ++added_faces;
            }
        }
        // Every refusal below reports the same three facts, because they are what
        // separates the cases: the angle asked for, how many side faces were
        // eligible, and how many the builder accepted.
        const auto counts = [&](double angle) {
            return nlohmann::json{{"draft",
                                   {{"angleDeg", angle},
                                    {"eligibleFaces", eligible_faces.size()},
                                    {"addedFaces", added_faces}}}};
        };
        if (eligible_faces.empty()) {
            // No planar wall exists at all — a circular profile, for instance.
            failure = {"EXTRUDE_DRAFT_NO_PLANAR_FACE",
                       "Extrude draft refused: no eligible planar side faces",
                       counts(draft_angle_deg)};
            return std::nullopt;
        }
        if (added_faces == 0) {
            // Walls existed; OCCT rejected every one of them. A different defect
            // from the above, and the user's next move differs too.
            failure = {"EXTRUDE_DRAFT_NO_FACE_ACCEPTED",
                       "Extrude draft refused: no eligible side faces accepted",
                       counts(draft_angle_deg)};
            return std::nullopt;
        }
        if (added_faces != eligible_faces.size()) {
            failure = {"EXTRUDE_DRAFT_PARTIAL_ACCEPTANCE",
                       "Extrude draft refused: OCCT accepted only part of the eligible side set",
                       counts(draft_angle_deg)};
            return std::nullopt;
        }
        draft.Build();
        if (draft.IsDone() && !draft.Shape().IsNull()) {
            std::vector<gp_Dir> result_wall_normals;
            for (TopExp_Explorer exp(draft.Shape(), TopAbs_FACE); exp.More(); exp.Next()) {
                gp_Pln candidate_plane;
                gp_Dir candidate_normal;
                if (!planar_face_plane_normal(TopoDS::Face(exp.Current()), candidate_plane,
                                              candidate_normal)) {
                    continue;
                }
                if (std::abs(candidate_normal.Dot(draft_dir)) >
                    kSideFaceDotThreshold) {
                    continue;
                }
                result_wall_normals.push_back(candidate_normal);
            }

            std::vector<bool> used(result_wall_normals.size(), false);
            std::size_t measured_faces = 0;
            double maximum_angle_error = 0.0;
            for (const TopoDS_Face& original : eligible_faces) {
                gp_Pln before_plane;
                gp_Dir before_normal;
                if (!planar_face_plane_normal(original, before_plane, before_normal)) {
                    failure = {"EXTRUDE_DRAFT_SEMANTIC_MISMATCH",
                               "Extrude draft refused: an eligible wall became unmeasurable",
                               counts(draft_angle_deg)};
                    return std::nullopt;
                }
                std::size_t best = result_wall_normals.size();
                double best_error = std::numeric_limits<double>::infinity();
                for (std::size_t i = 0; i < result_wall_normals.size(); ++i) {
                    if (used[i]) continue;
                    const double measured_angle = before_normal.Angle(result_wall_normals[i]);
                    const double error =
                        std::abs(measured_angle - std::abs(angle_rad));
                    if (error < best_error) {
                        best = i;
                        best_error = error;
                    }
                }
                if (best == result_wall_normals.size() ||
                    best_error > kDraftSemanticAngleTolerance) {
                    nlohmann::json evidence = counts(draft_angle_deg);
                    evidence["draft"]["measuredFaces"] = measured_faces;
                    evidence["draft"]["minimumUnmatchedAngleErrorRad"] = best_error;
                    evidence["draft"]["angleToleranceRad"] =
                        kDraftSemanticAngleTolerance;
                    failure = {"EXTRUDE_DRAFT_SEMANTIC_MISMATCH",
                               "Extrude draft refused: an eligible wall has no successor at the requested angle",
                               std::move(evidence)};
                    return std::nullopt;
                }
                used[best] = true;
                ++measured_faces;
                maximum_angle_error = std::max(maximum_angle_error, best_error);
            }
            const double before = solid_volume(shape);
            const double after = solid_volume(draft.Shape());
            const double tolerance = std::max(1e-9, std::abs(before) * 1e-10);
            if (std::abs(after - before) > tolerance) return draft.Shape();
            // The builder completed and changed nothing: the semantic check that
            // stops a claimed angle from being silently dropped.
            nlohmann::json evidence = counts(draft_angle_deg);
            evidence["draft"]["volumeBefore"] = before;
            evidence["draft"]["volumeAfter"] = after;
            failure = {"EXTRUDE_DRAFT_NO_CHANGE",
                       "Extrude draft refused: draft left shape unchanged", evidence};
            return std::nullopt;
        }
        failure = {"EXTRUDE_DRAFT_BUILD_FAILED", "Extrude draft failed",
                   counts(draft_angle_deg)};
    } catch (const Standard_Failure& error) {
        failure = {"EXTRUDE_DRAFT_BUILD_FAILED",
                   std::string("Extrude draft failed: ") +
                       (error.GetMessageString() ? error.GetMessageString() : "OCCT"),
                   nlohmann::json{{"draft", {{"angleDeg", draft_angle_deg}}}}};
    } catch (...) {
        failure = {"EXTRUDE_DRAFT_BUILD_FAILED", "Extrude draft failed",
                   nlohmann::json{{"draft", {{"angleDeg", draft_angle_deg}}}}};
    }
    return std::nullopt;
}

/// Wraps a draft refusal in the `OP_FAILED` outcome plus its stable diagnostic.
OpOutcome draft_refusal(const DraftFailure& failure) {
    OpOutcome out = OpOutcome::fail("OP_FAILED", failure.message);
    out.diagnostics.push_back({{"severity", "error"},
                               {"code", failure.code},
                               {"message", failure.message},
                               {"stage", "build"},
                               {"evidence", failure.evidence}});
    return out;
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
    std::optional<int> region_identity_version;
    if (params.contains("regionIdentityVersion")) {
        if (!params["regionIdentityVersion"].is_number_integer()) {
            return OpOutcome::fail("OP_FAILED", "Extrude: regionIdentityVersion must be an integer");
        }
        region_identity_version = params["regionIdentityVersion"].get<int>();
    }
    std::string perr;
    std::optional<TopoDS_Face> profile =
        build_profile_face(*sketch_params, read_str(params, "regionId"),
                           region_identity_version, perr);
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
    if (ref_rec) {
        if (auto invalid = validate_modeling_body(*ref_rec, "Extrude", "target")) return *invalid;
    }

    const double distance = read_scalar(params, "distance", 10.0);
    const double distance2 = read_scalar(params, "distance2", 0.0);
    const double draft_angle = read_scalar(params, "draftAngleDeg", 0.0);
    if (!std::isfinite(distance) || !std::isfinite(distance2) ||
        !std::isfinite(draft_angle)) {
        return OpOutcome::fail("OP_FAILED", "Extrude parameters must be finite");
    }
    // Distance validation runs before the profile is built and the reference body
    // may be absent (NewBody), and `authoring_resolution()` is scale-independent in
    // v1 (GeometryPrecision.h), so the floor-only context answers exactly what a
    // measured one would.
    const double min_value =
        kernel::validation::GeometryPrecisionContext{}.authoring_resolution();
    const bool distance_driven = !two_dirs && (mode_str == "Blind" || mode_str == "Symmetric");
    if (distance_driven && std::abs(distance) < min_value) {
        return OpOutcome::fail("OP_FAILED", "Extrude distance too small");
    }
    if (two_dirs && mode_str == "Blind" && std::abs(distance) < min_value) {
        return OpOutcome::fail("OP_FAILED", "Extrude first distance too small");
    }
    if (two_dirs && mode2_str == "Blind" && std::abs(distance2) < min_value) {
        return OpOutcome::fail("OP_FAILED", "Extrude second distance too small");
    }

    // Resolve a signed extrude distance for one end condition + direction. ToFace
    // resolution can raise NeedsRepair (surfaced via `nr`), a named refusal
    // (`tf_fail`) or a hard error (`err`); a TILTED planar target additionally
    // yields the half-space `trim` the prism must be cut back by.
    std::optional<ToFaceTrim> trim1;
    std::optional<ToFaceTrim> trim2;
    std::optional<ToFaceFailure> tf_fail;
    auto effective_distance = [&](const std::string& m, double blind, const gp_Dir& ref_dir,
                                  const json& face_ref, const std::string& ref_id,
                                  std::optional<ToFaceTrim>& trim, std::optional<json>& nr,
                                  std::string& err) -> std::optional<double> {
        if (m == "Blind" || m == "Symmetric") return blind;
        if (m == "ThroughAll") return through_all_distance(blind, origin, ref_dir, ref_shape);
        if (m == "ToFace") {
            ToFaceResolve tf =
                resolve_to_face(ctx, face_ref, *profile, origin, ref_dir, ref_id);
            if (tf.needs_repair) { nr = tf.needs_repair; return std::nullopt; }
            if (!tf.distance) {
                tf_fail = tf.failure;
                err = tf.error;
                return std::nullopt;
            }
            trim = tf.trim;
            return tf.distance;
        }
        if (m == "ToNext") {
            if (!ref_shape) { err = "ToNext requires an existing target body"; return std::nullopt; }
            const ToNextResult next = to_next_distance(*profile, ref_dir, *ref_shape);
            if (next.status == ToNextStatus::Unprovable) {
                // UNKNOWN is refused by name, so it is never confused with the honest
                // "there is nothing ahead" negative below.
                err = "ToNext could not prove exact first contact against the target body";
                return std::nullopt;
            }
            if (next.status != ToNextStatus::Contact) {
                err = "ToNext: no face found ahead of the extrude direction";
                return std::nullopt;
            }
            return next.distance;
        }
        err = "Extrude: unknown end condition '" + m + "'";
        return std::nullopt;
    };

    if (std::abs(draft_angle) > kDraftAngleEpsilon &&
        (two_dirs || mode_str != "Blind")) {
        const std::string message =
            "Extrude draft refused: only a single-direction Blind end condition has proven draft semantics";
        return draft_refusal(
            DraftFailure{"EXTRUDE_DRAFT_END_CONDITION_UNSUPPORTED", message,
                         nlohmann::json{{"draft",
                                         {{"angleDeg", draft_angle},
                                          {"eligibleFaces", 0},
                                          {"addedFaces", 0},
                                          {"endCondition", mode_str},
                                          {"twoDirections", two_dirs}}}}});
    }

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
                                         op_id + ".targetFace", trim1, nr, err);
            if (nr) { OpOutcome o; o.needs_repair.push_back(std::move(*nr)); return o; }
            if (!d1) {
                if (tf_fail) return to_face_refusal(*tf_fail);
                return OpOutcome::fail("OP_FAILED", err.empty() ? "Extrude: bad end condition" : err);
            }
            auto d2 = effective_distance(mode2_str, distance2, dir2,
                                         params.value("targetFace2", json()), op_id + ".targetFace2",
                                         trim2, nr, err);
            if (nr) { OpOutcome o; o.needs_repair.push_back(std::move(*nr)); return o; }
            if (!d2) {
                if (tf_fail) return to_face_refusal(*tf_fail);
                return OpOutcome::fail("OP_FAILED", err.empty() ? "Extrude: bad end condition" : err);
            }
            TopoDS_Shape p1 = make_prism(*profile, direction, *d1, err);
            if (p1.IsNull()) return OpOutcome::fail("OP_FAILED", err);
            TopoDS_Shape p2 = make_prism(*profile, dir2, *d2, err);
            if (p2.IsNull()) return OpOutcome::fail("OP_FAILED", err);
            if (trim1) {
                std::optional<TopoDS_Shape> cut = trim_prism_to_target(p1, *trim1, tf_fail, err);
                if (!cut) {
                    if (tf_fail) return to_face_refusal(*tf_fail);
                    return OpOutcome::fail("OP_FAILED", err);
                }
                p1 = std::move(*cut);
            }
            if (trim2) {
                std::optional<TopoDS_Shape> cut = trim_prism_to_target(p2, *trim2, tf_fail, err);
                if (!cut) {
                    if (tf_fail) return to_face_refusal(*tf_fail);
                    return OpOutcome::fail("OP_FAILED", err);
                }
                p2 = std::move(*cut);
            }
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
                                         op_id + ".targetFace", trim1, nr, err);
            if (nr) { OpOutcome o; o.needs_repair.push_back(std::move(*nr)); return o; }
            if (!d1) {
                if (tf_fail) return to_face_refusal(*tf_fail);
                return OpOutcome::fail("OP_FAILED", err.empty() ? "Extrude: bad end condition" : err);
            }
            tool_shape = make_prism(*profile, direction, *d1, err);
            if (tool_shape.IsNull()) return OpOutcome::fail("OP_FAILED", err);
            if (trim1) {
                std::optional<TopoDS_Shape> cut =
                    trim_prism_to_target(tool_shape, *trim1, tf_fail, err);
                if (!cut) {
                    if (tf_fail) return to_face_refusal(*tf_fail);
                    return OpOutcome::fail("OP_FAILED", err);
                }
                tool_shape = std::move(*cut);
            }
        }

        const std::string shape_error =
            invalid_shape_reason(tool_shape, "Extrude");
        if (!shape_error.empty()) {
            return OpOutcome::fail("GEOMETRY_INVALID", shape_error);
        }

        // Draft (side faces only) — applied to the prism before the boolean.
        DraftFailure draft_failure;
        std::optional<TopoDS_Shape> drafted =
            apply_draft(tool_shape, draft_angle, plane, direction, distance, draft_failure);
        if (!drafted) return draft_refusal(draft_failure);
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
        kernel::validation::PublicationPolicy policy =
            kernel::validation::single_solid_policy(
                "Extrude", kernel::validation::PublicationTier::TierA);
        // A fresh body has no input tolerance to grow FROM, so the ceiling is the
        // authoring resolution outright.
        policy.maximum_tolerance =
            kernel::validation::precision_of(tool_shape).authoring_resolution();
        const kernel::validation::PublicationDecision decision =
            publication_decision(tool_shape, policy);
        if (!decision.publishable()) {
            return publication_refusal(decision, "publication");
        }
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
    kernel::validation::PublicationPolicy policy;
    policy.name = "Extrude boolean";
    policy.allowed_top_level_shapes = kernel::validation::TopLevelShapePolicy::SolidSet;
    policy.max_solid_count = -1;
    policy.tier = result_validation_tier(
        ctx, kernel::validation::PublicationTier::TierB);
    policy.require_closed_manifold =
        policy.tier == kernel::validation::PublicationTier::TierB;
    // Grows from the TARGET's tolerance: a boolean inherits whatever uncertainty
    // the body it modifies arrived with, and may double it plus a slack term.
    {
        const auto prec = kernel::validation::precision_of(old_target);
        policy.maximum_tolerance = prec.tolerance_ceiling(prec.input_tolerance, 2.0, 1.0e-6);
    }
    policy.allow_empty_lifecycle = true;
    const kernel::validation::PublicationDecision decision = publication_decision(br.shape, policy);
    if (!decision.publishable() && !decision.lifecycle_only()) {
        return publication_refusal(decision, "publication");
    }

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
