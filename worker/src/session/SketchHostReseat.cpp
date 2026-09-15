// SketchHostReseat.cpp — see SketchHostReseat.h.
#include "session/SketchHostReseat.h"

#include <array>
#include <cmath>
#include <cstdint>
#include <utility>

#include <BRepAdaptor_Surface.hxx>
#include <BRepClass_FaceClassifier.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopAbs_State.hxx>
#include <TopoDS.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>

#include "elementmap/Scoring.h"
#include "kernel/topology/CoplanarFacePatch.h"
#include "util/Log.h"

namespace onecad::session {
namespace {

namespace em = onecad::elementmap;
using nlohmann::json;

constexpr double kPi = 3.14159265358979323846;

// The frozen frame must be an EXACT orthonormal basis to transport from; the
// tolerances below are pure IEEE round-trip slack on values Rust authored as
// unit vectors, not a modelling tolerance.
constexpr double kUnitTolerance = 1.0e-9;
constexpr double kOrthoTolerance = 1.0e-9;

std::string read_str(const json& o, const char* key) {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return {};
}

bool read_vec3(const json& v, gp_Vec& out) {
    if (!v.is_array() || v.size() != 3) return false;
    for (std::size_t i = 0; i < 3; ++i) {
        if (!v[i].is_number()) return false;
    }
    out = gp_Vec(v[0].get<double>(), v[1].get<double>(), v[2].get<double>());
    return true;
}

bool read_pnt3(const json& v, gp_Pnt& out) {
    gp_Vec tmp;
    if (!read_vec3(v, tmp)) return false;
    out = gp_Pnt(tmp.X(), tmp.Y(), tmp.Z());
    return true;
}

json vec3_json(const gp_Vec& v) { return json::array({v.X(), v.Y(), v.Z()}); }
json pnt3_json(const gp_Pnt& p) { return json::array({p.X(), p.Y(), p.Z()}); }

bool finite_vec(const gp_Vec& v) {
    return std::isfinite(v.X()) && std::isfinite(v.Y()) && std::isfinite(v.Z());
}

// A §9 item in exactly the shape `Ladder.cpp::to_needs_repair_json` emits, so a
// repair UI cannot tell an op-built sketch item apart from a ladder one. Mirrors
// `ComponentOp.cpp`'s `mate_unresolved_repair` / `mate_op_built_item` (op
// executors stay self-contained rather than reaching into each other).
json sketch_repair_item(const std::string& ref_id, const std::string& element_id,
                        const char* reason, json candidates, const std::string& ui_label,
                        const json& anchor) {
    return json{{"refId", ref_id},
                {"elementId", element_id},
                {"ladderFailed", "descriptor"},
                {"reason", reason},
                {"scoringVersion", em::kResolverVersion},
                {"candidates", std::move(candidates)},
                {"anchor", anchor.is_object() ? anchor : json::object()},
                {"uiLabel", ui_label}};
}

// One §9 candidate naming the face we DID resolve — the same shape
// `ComponentOp.cpp::mate_face_candidate` builds, down to the `summary` wording.
json resolved_face_candidate(const TopoDS_Shape& body_shape, const TopoDS_Shape& face,
                             double score, double margin) {
    const em::km::ElementDescriptor d = em::ElementMapPartition::describe(face, body_shape);
    return json{{"topoKey", em::ElementMapPartition::topokey_for_shape(body_shape, face,
                                                                      em::km::ElementKind::Face)},
                {"score", score},
                {"margin", margin},
                {"worldPos", json::array({d.center.X(), d.center.Y(), d.center.Z()})},
                {"summary", em::candidate_summary(em::km::ElementKind::Face, d)},
                {"featureContributions", json::object()}};
}

// The LADDER's own §9 item, with our `uiLabel` (and, for the stricter host gate,
// our `reason`). The ranked candidates, their feature contributions, the
// `scoringVersion` and the anchor echo are carried VERBATIM: a repair UI must not
// be able to tell a sketch-host item apart from any other ladder item, and the
// candidates are the evidence a re-pick is chosen from.
json ladder_repair_item(const em::LadderResolution& res, const std::string& ui_label,
                        const char* reason_override) {
    json item = res.to_needs_repair_json();
    if (reason_override != nullptr) item["reason"] = reason_override;
    item["uiLabel"] = ui_label;
    return item;
}

json info_diagnostic(const char* code, const std::string& message, json evidence) {
    return json{{"severity", "info"},
                {"code", code},
                {"stage", "sketch"},
                {"message", message},
                {"evidence", std::move(evidence)}};
}

}  // namespace

// ─────────────────────────────────────────────────────────────────────────────
// Frame algebra (A1 §3a/§3b)
// ─────────────────────────────────────────────────────────────────────────────

bool sketch_frame_valid(const SketchFrame& f) {
    if (!std::isfinite(f.origin.X()) || !std::isfinite(f.origin.Y()) ||
        !std::isfinite(f.origin.Z())) {
        return false;
    }
    if (!finite_vec(f.x_axis) || !finite_vec(f.y_axis) || !finite_vec(f.normal)) return false;
    if (std::abs(f.x_axis.Magnitude() - 1.0) > kUnitTolerance) return false;
    if (std::abs(f.y_axis.Magnitude() - 1.0) > kUnitTolerance) return false;
    if (std::abs(f.normal.Magnitude() - 1.0) > kUnitTolerance) return false;
    if (std::abs(f.x_axis.Dot(f.y_axis)) > kOrthoTolerance) return false;
    if (std::abs(f.x_axis.Dot(f.normal)) > kOrthoTolerance) return false;
    if (std::abs(f.y_axis.Dot(f.normal)) > kOrthoTolerance) return false;
    // RIGHT-handed: (x × y) · n must be +1, not −1. A left-handed stored basis is
    // a repair, never a silently "corrected" one — correcting it would mirror
    // every stored (u,v).
    return f.x_axis.Crossed(f.y_axis).Dot(f.normal) > 0.0;
}

FrameChange frame_change(const SketchFrame& frozen, const SketchFrame& candidate) {
    FrameChange out;
    const gp_Vec dt(frozen.origin, candidate.origin);
    out.translation_mm = dt.Magnitude();

    // A1 §3b: measure the WHOLE frame, not just its normal. With
    // `Bi = [xi yi ni]` (columns) and `Q = Bc·B0ᵀ`, the rotation angle is
    // `atan2(‖(Q32−Q23, Q13−Q31, Q21−Q12)‖/2, (trace(Q)−1)/2)`.
    const std::array<const gp_Vec*, 3> bc{&candidate.x_axis, &candidate.y_axis, &candidate.normal};
    const std::array<const gp_Vec*, 3> b0{&frozen.x_axis, &frozen.y_axis, &frozen.normal};
    const auto comp = [](const gp_Vec& v, int i) {
        return i == 0 ? v.X() : (i == 1 ? v.Y() : v.Z());
    };
    double q[3][3] = {{0.0, 0.0, 0.0}, {0.0, 0.0, 0.0}, {0.0, 0.0, 0.0}};
    for (int i = 0; i < 3; ++i) {
        for (int j = 0; j < 3; ++j) {
            double sum = 0.0;
            for (int k = 0; k < 3; ++k) sum += comp(*bc[k], i) * comp(*b0[k], j);
            q[i][j] = sum;
        }
    }
    const double ax = (q[2][1] - q[1][2]) * 0.5;
    const double ay = (q[0][2] - q[2][0]) * 0.5;
    const double az = (q[1][0] - q[0][1]) * 0.5;
    const double trace = q[0][0] + q[1][1] + q[2][2];
    out.rotation_rad = std::atan2(std::sqrt(ax * ax + ay * ay + az * az), (trace - 1.0) * 0.5);

    // EXACT (A1 §3b): a positive transport deadband would leave a sketch behind on
    // a sub-epsilon plane displacement, so `moved` compares the components
    // themselves and `significant` is only a report.
    out.moved = !(frozen.origin.X() == candidate.origin.X() &&
                  frozen.origin.Y() == candidate.origin.Y() &&
                  frozen.origin.Z() == candidate.origin.Z() &&
                  frozen.x_axis.X() == candidate.x_axis.X() &&
                  frozen.x_axis.Y() == candidate.x_axis.Y() &&
                  frozen.x_axis.Z() == candidate.x_axis.Z() &&
                  frozen.y_axis.X() == candidate.y_axis.X() &&
                  frozen.y_axis.Y() == candidate.y_axis.Y() &&
                  frozen.y_axis.Z() == candidate.y_axis.Z() &&
                  frozen.normal.X() == candidate.normal.X() &&
                  frozen.normal.Y() == candidate.normal.Y() &&
                  frozen.normal.Z() == candidate.normal.Z());
    out.significant = out.translation_mm > kSketchReseatTranslationEpsilonMm ||
                      out.rotation_rad > kSketchReseatRotationEpsilonDeg * kPi / 180.0;
    return out;
}

TransportResult transport_sketch_frame(const SketchFrame& frozen, const gp_Pnt& plane_point,
                                       const gp_Vec& plane_normal) {
    TransportResult out;
    out.effective = frozen;

    // Validate the INPUTS first — the stored frame and the resolved plane. Only a
    // failure HERE is `sketchFrameInvalid`; everything after this point is the
    // transport's own arithmetic and is reported as ill-conditioned instead.
    if (!sketch_frame_valid(frozen) || !finite_vec(plane_normal) ||
        std::abs(plane_normal.Magnitude() - 1.0) > kUnitTolerance ||
        !std::isfinite(plane_point.X()) || !std::isfinite(plane_point.Y()) ||
        !std::isfinite(plane_point.Z())) {
        out.status = TransportStatus::FrameInvalid;
        return out;
    }

    const gp_Vec n1 = plane_normal;
    out.normal_reversed = frozen.normal.Dot(n1) < 0.0;

    // Project the origin onto the resolved plane ALONG n1. Any other point of the
    // same plane gives the same `d`, so tangential drift of the kernel's plane
    // origin cannot drag the sketch (A1 §3a).
    const double d = gp_Vec(frozen.origin, plane_point).Dot(n1);
    const gp_Pnt oc = frozen.origin.Translated(n1 * d);

    SketchFrame candidate;
    candidate.origin = oc;
    if (frozen.normal.X() == n1.X() && frozen.normal.Y() == n1.Y() &&
        frozen.normal.Z() == n1.Z()) {
        // A1 §3a: an UNCHANGED normal carries the authored basis VERBATIM. Rebuilding
        // it from the world-axis seed would silently discard a legacy (or simply
        // different) in-plane basis that is still perfectly valid.
        candidate.x_axis = frozen.x_axis;
        candidate.y_axis = frozen.y_axis;
        candidate.normal = frozen.normal;
    } else {
        const gp_Vec qx = frozen.x_axis - n1 * frozen.x_axis.Dot(n1);
        const double h = qx.Magnitude();
        if (!(h > kSketchGramSchmidtGuard)) {
            // A1 §3a/§7: the Gram-Schmidt fallback cannot be continuous through the
            // singularity, so it is computed as REPAIR EVIDENCE and never published.
            out.status = TransportStatus::FrameIllConditioned;
            const gp_Vec qy = frozen.y_axis - n1 * frozen.y_axis.Dot(n1);
            const double hy = qy.Magnitude();
            if (hy > kSketchGramSchmidtGuard) {
                SketchFrame fallback;
                fallback.origin = oc;
                fallback.y_axis = qy / hy;
                fallback.x_axis = fallback.y_axis.Crossed(n1);
                fallback.normal = n1;
                out.ill_conditioned_fallback = fallback;
            }
            return out;
        }
        // RE-ORTHOGONALISE through cross products before publishing. The subtraction
        // above cancels catastrophically as `h` approaches the guard: for a normal
        // tilted b = 2e−8 off the frozen X axis, `qx`'s surviving component carries
        // ~1e−16 of absolute cancellation error and dividing by `h` amplifies it to
        // ~2e−9, so the projected axis misses orthogonality by more than the 1e−9 the
        // validator allows. A cross product has no such amplification: `n₁ × x̂` is
        // perpendicular to both to within a relative rounding error, and rebuilding X
        // from it lands the whole basis inside 1e−12. Nothing about the CONVENTION
        // changes — `xc` still maximises `xc·x₀` — only its conditioning.
        gp_Vec yc = n1.Crossed(qx / h);
        const double hy = yc.Magnitude();
        gp_Vec xc = yc.Crossed(n1);
        const double hx = xc.Magnitude();
        if (!(hy > kSketchGramSchmidtGuard) || !(hx > kSketchGramSchmidtGuard)) {
            out.status = TransportStatus::FrameIllConditioned;
            return out;
        }
        yc /= hy;
        xc /= hx;
        candidate.x_axis = xc;
        candidate.normal = n1;
        candidate.y_axis = yc;
    }

    if (!sketch_frame_valid(candidate)) {
        // The FROZEN frame was validated above, so a candidate that fails here is the
        // TRANSPORT's own arithmetic giving out, not a corrupt stored basis. Calling
        // that `sketchFrameInvalid` would tell the user to re-pick a frame that is
        // perfectly valid; `sketchFrameIllConditioned` is the honest reason and the
        // one whose repair (re-pick the host face) actually applies.
        out.status = TransportStatus::FrameIllConditioned;
        return out;
    }
    out.effective = candidate;
    out.change = frame_change(frozen, candidate);
    return out;
}

SeatState classify_sketch_seat(const TopoDS_Face& face, const SketchFrame& frozen,
                               const SketchFrame& candidate, const gp_Pnt& frozen_anchor) {
    if (face.IsNull()) return SeatState::Unmeasurable;
    if (!std::isfinite(frozen_anchor.X()) || !std::isfinite(frozen_anchor.Y()) ||
        !std::isfinite(frozen_anchor.Z())) {
        return SeatState::Unmeasurable;
    }
    // A1 §3d: the witness is the frozen ANCHOR re-planted in the candidate frame at
    // the same (u,v). The plane ORIGIN is deliberately not used — an unchanged
    // annular face legitimately places it inside its own central hole.
    const gp_Vec delta(frozen.origin, frozen_anchor);
    const double ua = delta.Dot(frozen.x_axis);
    const double va = delta.Dot(frozen.y_axis);
    const gp_Pnt witness = candidate.origin.Translated(candidate.x_axis * ua)
                               .Translated(candidate.y_axis * va);
    try {
        BRepClass_FaceClassifier classifier(face, witness, BRep_Tool::Tolerance(face));
        switch (classifier.State()) {
            case TopAbs_IN:
            case TopAbs_ON:
                return SeatState::OnFace;
            case TopAbs_OUT:
                return SeatState::OffFace;
            default:
                return SeatState::Unmeasurable;
        }
    } catch (const Standard_Failure&) {
        return SeatState::Unmeasurable;  // no claim; never "seated"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §7.3 `plane` marshalling
// ─────────────────────────────────────────────────────────────────────────────

json sketch_plane_params(const SketchFrame& frame) {
    return json{{"kind", "custom"},
                {"origin", pnt3_json(frame.origin)},
                {"xAxis", vec3_json(frame.x_axis)},
                {"yAxis", vec3_json(frame.y_axis)},
                {"normal", vec3_json(frame.normal)}};
}

SketchFrame sketch_frame_from_params(const json& params) {
    SketchFrame out;
    // The NORMATIVE non-standard named bases (SCHEMA §7.3), identical to
    // `sk::SketchPlane::XY()/XZ()/YZ()` and to `WireSketch::parse_plane`.
    out.x_axis = gp_Vec(0.0, 1.0, 0.0);
    out.y_axis = gp_Vec(-1.0, 0.0, 0.0);
    out.normal = gp_Vec(0.0, 0.0, 1.0);
    if (!params.is_object() || !params.contains("plane") || !params["plane"].is_object()) {
        return out;
    }
    const json& p = params["plane"];
    const std::string kind = p.contains("kind") && p["kind"].is_string()
                                 ? p["kind"].get<std::string>()
                                 : std::string("XY");
    if (kind == "XZ") {
        out.x_axis = gp_Vec(0.0, 1.0, 0.0);
        out.y_axis = gp_Vec(0.0, 0.0, 1.0);
        out.normal = gp_Vec(1.0, 0.0, 0.0);
        return out;
    }
    if (kind == "YZ") {
        out.x_axis = gp_Vec(-1.0, 0.0, 0.0);
        out.y_axis = gp_Vec(0.0, 0.0, 1.0);
        out.normal = gp_Vec(0.0, 1.0, 0.0);
        return out;
    }
    if (kind != "custom") return out;
    if (p.contains("origin")) read_pnt3(p["origin"], out.origin);
    if (p.contains("xAxis")) read_vec3(p["xAxis"], out.x_axis);
    if (p.contains("yAxis")) read_vec3(p["yAxis"], out.y_axis);
    if (p.contains("normal")) read_vec3(p["normal"], out.normal);
    return out;
}

// ─────────────────────────────────────────────────────────────────────────────
// The Sketch step's host resolution
// ─────────────────────────────────────────────────────────────────────────────

SketchHostReseatResult reseat_sketch_on_host(const BodyStore& bodies,
                                             const em::ElementMapPartition& partition,
                                             const json& params, const std::string& op_id,
                                             const em::LadderEditContext& edit) {
    SketchHostReseatResult out;
    if (!params.is_object() || !params.contains("hostFace") || !params["hostFace"].is_object()) {
        return out;  // world/datum sketch, or a legacy record: byte-identical behaviour
    }
    out.attempted = true;
    // SCHEMA §13 forward compatibility: refuse an unimplemented transport policy by
    // name. A producer that authored against a later rule must NOT have its sketch
    // re-seated under this one.
    if (params.contains("frameTransportVersion")) {
        const json& v = params["frameTransportVersion"];
        // Compare WITHOUT narrowing. `get<int>()` on a value outside `int` wraps, so
        // `4294967297` would compare equal to 1 and this build would re-seat a sketch
        // authored against a policy it has never heard of — the one outcome the
        // version field exists to prevent. Read at the width the value actually has.
        const bool supported =
            v.is_number_unsigned()
                ? v.get<std::uint64_t>() ==
                      static_cast<std::uint64_t>(kSketchFrameTransportVersion)
                : (v.is_number_integer() && v.get<std::int64_t>() ==
                                                static_cast<std::int64_t>(
                                                    kSketchFrameTransportVersion));
        if (!supported) {
            out.unsupported_transport =
                "Sketch frameTransportVersion " + v.dump() +
                " is not supported by this build (this worker implements version " +
                std::to_string(kSketchFrameTransportVersion) + ")";
            return out;
        }
    }
    const json& host_ref_json = params["hostFace"];

    // `<opId>.input0` — NOT a wire `inputs[]` entry (the `mate.target` rule), but
    // this is the addressing `parse_input_ref_id` understands and the slot
    // `KnownOperation::element_refs_mut`'s `Sketch` arm returns (`host_face` is its
    // only entry, i.e. index 0).
    const std::string ref_id = op_id + ".input0";
    const em::LadderRef ref = em::ladder_ref_from_input(host_ref_json, ref_id);
    const std::string host_body_id =
        (host_ref_json.contains("primary") && host_ref_json["primary"].is_object())
            ? read_str(host_ref_json["primary"], "bodyId")
            : std::string();

    const SketchFrame frozen = sketch_frame_from_params(params);
    out.effective = frozen;

    const auto refuse = [&](const char* reason, json candidates, const std::string& label) {
        out.needs_repair.push_back(
            sketch_repair_item(ref_id, ref.element_id, reason, std::move(candidates), label,
                               ref.anchor_json));
    };

    // ONE FAILURE CLASS: every host that cannot be re-seated HALTS (A-1). The step
    // reports its §9 item, a body-less step carrying repair items is `NeedsRepair`
    // (`merge_outcome`), and the plan stops at m−1 rather than letting downstream
    // features cut against a plane nobody can vouch for. The sketch itself is
    // Rust-owned document state, so it stays visible at its AUTHORED frame.
    //
    // This is only honest because the frozen anchor is a point the kernel
    // classified ON the face (SCHEMA §7.6 `exact.anchor`). While it was the face's
    // `gp_Pln` LOCATION it was routinely far outside the face — measured 2026-09-15
    // on `step_import_gate.rs` PHASE 4, an imported cap resolved to the RIGHT face
    // at margin 0.20 but scored 0.75, because the anchor feature contributed 0 of
    // the 0.85 auto-bind bar — and re-picking re-froze the same location, so no halt
    // built on it could ever be cleared. With an on-face anchor the same case scores
    // 1.00 and the confidence shortfall means what it says.
    if (host_body_id.empty() || ref.element_id.empty()) {
        refuse("no-candidates", json::array(),
               "the sketch's host face reference names no body — re-pick the face");
        return out;
    }
    const BodyRecord* host_rec = bodies.get(host_body_id);
    if (host_rec == nullptr) {
        refuse("no-candidates", json::array(),
               "the body the sketch is attached to no longer exists: " + host_body_id);
        return out;
    }

    // Two rungs, the mate's order: the TRACKED binding first (body-scoped, VF-M7
    // safe), then the descriptor stage.
    TopoDS_Shape bound;
    double score = 1.0;
    double margin = 1.0;
    const std::string topo_key =
        em::ElementMapPartition::topokey_for_element_in_body(partition, ref.element_id,
                                                            host_body_id);
    if (!topo_key.empty()) {
        bound = em::ElementMapPartition::shape_for_topokey(host_rec->geom, topo_key);
    }
    if (bound.IsNull()) {
        std::vector<em::LadderRef> refs{ref};
        const std::vector<em::LadderResolution> res =
            em::resolve_descriptor_stage(host_rec->geom, host_body_id, refs, edit);
        if (res.empty()) {
            refuse("no-candidates", json::array(),
                   "the sketch's host face could not be resolved — re-pick the face");
            return out;
        }
        const em::LadderResolution& resolution = res[0];
        if (resolution.outcome != em::LadderOutcome::AutoBind ||
            resolution.bound_shape.IsNull()) {
            // The ladder's own verdict, its own token (`no-candidates` /
            // `ambiguous` / `low-confidence`) and its own ranked candidates.
            out.needs_repair.push_back(ladder_repair_item(
                resolution,
                "the sketch's host face could not be identified with confidence — "
                "re-pick the face",
                /*reason_override=*/nullptr));
            WLOG_INFO("sketch host unresolved: op=%s element=%s reason=%s score=%.6f margin=%.6f",
                      op_id.c_str(), ref.element_id.c_str(), resolution.reason.c_str(),
                      resolution.score, resolution.margin);
            return out;
        }
        // STRICTER than the ladder's own gate (A1 §1): `Ladder.cpp` admits an
        // ANCHOR-DECIDED bind below `kAutoBindMinMargin`, and a sketch host is
        // exactly where that is unsafe — a congruent twin sitting on the frozen
        // anchor would silently capture the whole sketch. Refuse those with the
        // ladder's own margin token.
        score = resolution.score;
        margin = resolution.margin;
        if (!(score >= em::kAutoBindMinScore) || !(margin >= em::kAutoBindMinMargin)) {
            out.needs_repair.push_back(ladder_repair_item(
                resolution,
                "the sketch's host face has a look-alike the resolver cannot separate — "
                "re-pick the face",
                /*reason_override=*/"ambiguous"));
            WLOG_INFO("sketch host below the host gate: op=%s element=%s score=%.6f margin=%.6f",
                      op_id.c_str(), ref.element_id.c_str(), score, margin);
            return out;
        }
        bound = resolution.bound_shape;
    }

    // A1 §3c: planarity is a TOPOLOGY/SURFACE-TYPE predicate with no distance
    // epsilon. An approximately planar cylinder or spline is refused, never fitted.
    if (bound.ShapeType() != TopAbs_FACE) {
        refuse("sketchHostNonPlanar", json::array(),
               "the sketch's host is no longer a face");
        return out;
    }
    const TopoDS_Face host_face = TopoDS::Face(bound);
    gp_Pln host_plane;
    gp_Dir host_normal;
    if (!core::modeling::CoplanarFacePatch::planarFacePlaneAndNormal(host_face, host_plane,
                                                                    host_normal)) {
        refuse("sketchHostNonPlanar",
               json::array({resolved_face_candidate(host_rec->geom, bound, score, margin)}),
               "the sketch's host face is no longer planar — the sketch stays on its "
               "authored plane");
        return out;
    }

    const TransportResult transported = transport_sketch_frame(
        frozen, host_plane.Location(), gp_Vec(host_normal.X(), host_normal.Y(), host_normal.Z()));
    if (transported.status == TransportStatus::FrameInvalid) {
        refuse("sketchFrameInvalid",
               json::array({resolved_face_candidate(host_rec->geom, bound, score, margin)}),
               "the sketch's stored frame is not a valid right-handed basis — re-pick the "
               "host face");
        return out;
    }
    if (transported.status == TransportStatus::FrameIllConditioned) {
        json item_candidates =
            json::array({resolved_face_candidate(host_rec->geom, bound, score, margin)});
        refuse("sketchFrameIllConditioned", std::move(item_candidates),
               "the sketch's X axis is along the host face's new normal — the frame cannot "
               "be carried across continuously; re-pick the host face");
        // Repair EVIDENCE only, never published (A1 §3a).
        if (transported.ill_conditioned_fallback) {
            out.needs_repair.back()["fallbackPlane"] =
                sketch_plane_params(*transported.ill_conditioned_fallback);
        }
        return out;
    }

    // A1 §3d: the seat is classified even when the plane did NOT move — a face that
    // shrank under an unchanged plane still loses its sketch.
    //
    // The witness is the frozen ANCHOR transported into the candidate frame, with no
    // exemptions. The plane ORIGIN stays advisory (an unchanged annular face
    // legitimately places it inside its own central hole), but the anchor is a point
    // the kernel classified ON the face when the sketch was authored (SCHEMA §7.6
    // `exact.anchor`), so `OUT` means the sketch genuinely left its host. A LEGACY
    // record whose anchor is the face's plane location and lies off the face reports
    // `sketchSeatOffFace` — that is the honest answer for a seat nobody can vouch
    // for, and re-picking the face freezes an on-face anchor that clears it.
    const SeatState seat =
        ref.anchor.has_world_point
            ? classify_sketch_seat(host_face, frozen, transported.effective,
                                   ref.anchor.world_point)
            // SCHEMA §9: "or the `hostFace` carries no usable anchor". Without a
            // witness there is no evidence of attachment, and "unmeasurable" is
            // never "seated".
            : SeatState::Unmeasurable;
    if (seat == SeatState::OffFace) {
        refuse("sketchSeatOffFace",
               json::array({resolved_face_candidate(host_rec->geom, bound, score, margin)}),
               "the sketch no longer sits on its host face — move the sketch or re-pick "
               "the face");
        return out;
    }
    if (seat == SeatState::Unmeasurable) {
        // A1 §3d: "do not claim successful seating". An UNKNOWN classification, or no
        // witness at all, is not evidence the sketch is still attached.
        refuse("sketchSeatUnmeasurable",
               json::array({resolved_face_candidate(host_rec->geom, bound, score, margin)}),
               "the sketch's attachment point could not be measured against its host "
               "face — re-pick the face");
        return out;
    }

    out.resolved = true;
    out.effective = transported.effective;
    out.change = transported.change;

    if (transported.normal_reversed) {
        out.diagnostics.push_back(info_diagnostic(
            "SKETCH_HOST_NORMAL_REVERSED",
            "the sketch's host face came back with a reversed outward normal; the sketch "
            "follows it",
            json{{"normal", vec3_json(out.effective.normal)}}));
    }
    if (out.change.moved) {
        const double rotation_deg = out.change.rotation_rad * 180.0 / kPi;
        out.sketch_placement = json{{"plane", sketch_plane_params(out.effective)},
                                    {"translationMm", out.change.translation_mm},
                                    {"rotationDeg", rotation_deg}};
        out.diagnostics.push_back(info_diagnostic(
            "SKETCH_HOST_RESEATED", "the sketch followed its host face",
            json{{"translationMm", out.change.translation_mm},
                 {"rotationDeg", rotation_deg},
                 {"significant", out.change.significant}}));
        WLOG_DEBUG(
            "sketch host reseat: op=%s element=%s translation=%.9f mm rotation=%.9f deg "
            "significant=%d",
            op_id.c_str(), ref.element_id.c_str(), out.change.translation_mm, rotation_deg,
            out.change.significant ? 1 : 0);
    }
    return out;
}

}  // namespace onecad::session
