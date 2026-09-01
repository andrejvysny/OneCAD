// ComponentOp.cpp — see ComponentOp.h.
#include "ops/ComponentOp.h"

#include <array>
#include <cmath>
#include <filesystem>
#include <map>
#include <optional>
#include <utility>
#include <string>
#include <vector>

#include <BRepBuilderAPI_Transform.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <GProp_GProps.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>
#include <gp_XYZ.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "io/BrepCodec.h"
#include "io/StepRead.h"
#include "io/XcafCodec.h"
#include "kernel/validation/ShapeAudit.h"
#include "ops/ComponentGenerators.h"
#include "ops/ComponentMateSolver.h"
#include "ops/OpCommon.h"
#include "session/ClassifyElement.h"

namespace onecad::ops {

using nlohmann::json;
namespace em = onecad::elementmap;

namespace {

constexpr double kPi = 3.14159265358979323846;
constexpr double kMinAxisLen2 = 1e-12;

// Default free params — chosen to reproduce P0/P1's old hardcoded M6×20
// EXACTLY, so every existing caller (nothing sends `source.params.thread`/
// `.length` yet — see WP-2.1's plan doc) stays byte-identical.
constexpr const char* kDefaultThread = "M6";
constexpr double kDefaultLengthMm = 20.0;
// WP-2.5: third free param, cosmetic stays default so every pre-existing
// caller (nothing sends `source.params.thread_detail` yet) stays
// byte-identical, same rule as thread/length above.
constexpr const char* kDefaultThreadDetail = "cosmetic";

// §7.3 `source.kind:"profile"`. The worker enforces exactly THREE canonicality
// properties — the face's plane passes through `z = 0`, its orientation-corrected
// normal is `+Z`, and its AREA centroid is on the origin. Each is one cheap
// measurement and each has a consequence a user would see: the first two decide
// which way the prism grows, the third decides WHERE the component lands.
//
// The remaining half of the ingest convention — the tie-broken principal in-plane
// axis on `+X` — stays an AUTHORING rule owned by the component-library spec and
// is deliberately NOT re-checked: that one needs a per-regen eigen-solve whose
// degenerate branch is platform-sensitive, and a placed component must never
// START refusing because a last bit moved. A rotated-but-centred profile still
// places in the right place; an off-centre one does not.
constexpr double kProfileCanonicalTol = 1.0e-6;
// `params.length` upper bound (mm). A prism longer than this is an authoring
// mistake (a unit slip), not a part.
constexpr double kMaxProfileLengthMm = 1.0e5;

// Reads `placement.{translate,rotate}` into a `gp_Trsf`, SAME normative order
// as TransformBody: `X' = T ∘ R(center, axis, angleDeg) · X` — rotate about
// the frozen pivot first, then translate. Absent `rotate` is the identity
// rotation (mirrors `FrozenPlacement`'s `#[serde(default)]`).
bool read_placement(const json& params, const std::string& op_label, gp_Trsf& trsf_out,
                    std::string& err) {
    if (!params.contains("placement") || !params["placement"].is_object()) {
        err = op_label + " requires a placement object";
        return false;
    }
    const json& placement = params["placement"];
    double tx = 0.0, ty = 0.0, tz = 0.0;
    if (!placement.contains("translate") || !placement["translate"].is_array() ||
        placement["translate"].size() != 3) {
        err = op_label + " placement.translate must be a 3-array";
        return false;
    }
    const json& t = placement["translate"];
    // Each entry is a bare number OR a `{value}` object (SCHEMA §4 Scalar wire form).
    auto scalar_at = [&](const json& v, const char* what, double& out) -> bool {
        if (v.is_number()) {
            out = v.get<double>();
        } else if (v.is_object() && v.contains("value") && v["value"].is_number()) {
            out = v["value"].get<double>();
        } else {
            err = op_label + " placement.translate." + what + " must be a scalar";
            return false;
        }
        if (!std::isfinite(out)) {
            err = op_label + " placement.translate." + what + " must be finite";
            return false;
        }
        return true;
    };
    if (!scalar_at(t[0], "x", tx) || !scalar_at(t[1], "y", ty) || !scalar_at(t[2], "z", tz)) {
        return false;
    }

    double cx = 0.0, cy = 0.0, cz = 0.0;
    double ax = 0.0, ay = 0.0, az = 1.0;
    double angle_deg = 0.0;
    if (placement.contains("rotate") && placement["rotate"].is_object()) {
        const json& rot = placement["rotate"];
        if (rot.contains("center")) {
            if (!rot["center"].is_array() || rot["center"].size() != 3 ||
                !scalar_at(rot["center"][0], "center.x", cx) ||
                !scalar_at(rot["center"][1], "center.y", cy) ||
                !scalar_at(rot["center"][2], "center.z", cz)) {
                if (err.empty()) err = op_label + " placement.rotate.center must be a 3-array";
                return false;
            }
        }
        if (rot.contains("axis")) {
            if (!rot["axis"].is_array() || rot["axis"].size() != 3 ||
                !scalar_at(rot["axis"][0], "axis.x", ax) ||
                !scalar_at(rot["axis"][1], "axis.y", ay) ||
                !scalar_at(rot["axis"][2], "axis.z", az)) {
                if (err.empty()) err = op_label + " placement.rotate.axis must be a 3-array";
                return false;
            }
        }
        if (rot.contains("angleDeg") && !scalar_at(rot["angleDeg"], "angleDeg", angle_deg)) {
            return false;
        }
    }
    const double axis_len2 = ax * ax + ay * ay + az * az;
    if (angle_deg != 0.0 && axis_len2 < kMinAxisLen2) {
        err = op_label + " placement.rotate.axis must be non-zero when angleDeg != 0";
        return false;
    }

    gp_Trsf rotation;
    if (angle_deg != 0.0) {
        rotation.SetRotation(gp_Ax1(gp_Pnt(cx, cy, cz), gp_Dir(ax, ay, az)), angle_deg * kPi / 180.0);
    }
    gp_Trsf translation;
    translation.SetTranslation(gp_Vec(tx, ty, tz));
    trsf_out = translation * rotation;
    return true;
}

// Component Library P3 WP-3.1 (spec §5.5): re-seat epsilons. Translation
// reuses HoleOp.cpp's own `kPointPlaneFence` value/unit verbatim (spec: "mirrors
// Hole's 1e-3 mm re-projection gate") — deliberate reuse, not an independent
// choice. Rotation has no existing precedent in this worker; chosen to
// correspond to roughly the same ~1e-3 mm of positional drift at the SHCS's
// own characteristic ~10 mm head radius, comfortably above OCCT's
// floating-point round-trip noise (~1e-6 rad ≈ 6e-5°).
constexpr double kMateReseatTranslationEpsilonMm = 1e-3;
constexpr double kMateReseatRotationEpsilonDeg = 1e-2;

// A minimal §9 NeedsRepair (STATE) for a mate whose target ref is malformed,
// whose owning body no longer exists, or whose resolved geometry can't seat
// the requested mate kind. Mirrors `PlanExecutor.cpp`'s own
// `missing_body_repair` shape (not reachable from here — op executors stay
// self-contained rather than reaching into PlanExecutor internals).
json mate_unresolved_repair(const std::string& ref_id, const std::string& element_id,
                            const std::string& ui_label) {
    return json{{"refId", ref_id},
               {"elementId", element_id},
               {"ladderFailed", "descriptor"},
               {"reason", "no-candidates"},
               {"scoringVersion", em::kResolverVersion},
               {"candidates", json::array()},
               {"anchor", json::object()},
               {"uiLabel", ui_label}};
}

// The candidate placement a resolved mate recomputed, paired with the
// already-solved `gp_Trsf` (avoids re-parsing the JSON a second time).
struct MateReseat {
    json placement;
    gp_Trsf trsf;
};

// Component Library P3 WP-3.1 (spec §5.5): re-resolves `mate.target` through
// the resolution ladder (cross-body-safe — target_body_id is read from
// `mate.target.primary.bodyId`, NEVER assumed to be the op's own body,
// exactly the discipline `worker/tests/test_cross_body_element_ref.cpp`
// (VF-M7) exists to enforce) and recomputes the component's seat from the
// mate kind + resolved frame + flip via `solve_mate_placement`.
//
// Returns the new placement + transform iff AutoBind resolved AND the
// candidate moved beyond the reseat epsilon vs `frozen` — the caller applies
// it and echoes it back on `OpOutcome.mate_placement` for Rust to persist
// (a derived writeback, never dropped/moved silently otherwise). Returns
// `std::nullopt` in every other case (unresolved target, malformed frame,
// or resolved-but-unmoved — the common no-op tick) — the frozen placement
// stands unchanged, and an unresolved target additionally appends a
// NeedsRepair item to `needs_repair_out` (never a hard `OP_FAILED`: a
// component with a broken mate still publishes at its last-good spot,
// spec's "never drop it, never silently move it").
std::optional<MateReseat> resolve_mate_reseat(OpContext& ctx, const json& mate, const json& frozen,
                                              const std::string& op_id,
                                              std::vector<json>& needs_repair_out) {
    if (!mate.contains("target") || !mate["target"].is_object()) return std::nullopt;
    const json& target_ref_json = mate["target"];
    // `<opId>.input0` — NOT a real wire input anymore (P3 WP-3.1 removed
    // `mate` from `inputs[]` on purpose, see `wire.rs::wire_op_inputs`), but
    // this matches `parse_input_ref_id`'s addressing convention
    // (`document_runtime.rs`) so a future manual-repair-rebind UI can still
    // find this ref: `element_refs_mut`'s `PlaceComponent` arm returns
    // `mate.target` as its ONLY entry, i.e. index 0.
    const std::string ref_id = op_id + ".input0";
    const em::LadderRef ref = em::ladder_ref_from_input(target_ref_json, ref_id);
    const std::string target_body_id =
        (target_ref_json.contains("primary") && target_ref_json["primary"].is_object())
            ? read_str(target_ref_json["primary"], "bodyId")
            : "";
    if (target_body_id.empty() || ref.element_id.empty()) {
        needs_repair_out.push_back(
            mate_unresolved_repair(ref_id, ref.element_id, "mate target has no primary body reference"));
        return std::nullopt;
    }
    const session::BodyRecord* target_rec = ctx.bodies.get(target_body_id);
    if (target_rec == nullptr) {
        needs_repair_out.push_back(mate_unresolved_repair(
            ref_id, ref.element_id, "mate target body not found: " + target_body_id));
        return std::nullopt;
    }

    // Tracked rung first — VF-M7-safe (body-scoped), same two-rung order
    // `HoleOp.cpp::resolve_host_face` uses.
    TopoDS_Shape bound;
    const std::string topo_key = em::ElementMapPartition::topokey_for_element_in_body(
        ctx.partition, ref.element_id, target_body_id);
    if (!topo_key.empty()) {
        bound = em::ElementMapPartition::shape_for_topokey(target_rec->geom, topo_key);
    }
    if (bound.IsNull()) {
        std::vector<em::LadderRef> refs{ref};
        const std::vector<em::LadderResolution> res = em::resolve_descriptor_stage(
            target_rec->geom, target_body_id, refs,
            em::LadderEditContext{ctx.post_upstream_edit, ctx.from_zero_replay});
        if (res.empty() || res[0].outcome != em::LadderOutcome::AutoBind || res[0].bound_shape.IsNull()) {
            needs_repair_out.push_back(res.empty()
                                           ? mate_unresolved_repair(ref_id, ref.element_id,
                                                                    "mate target unresolved")
                                           : res[0].to_needs_repair_json());
            return std::nullopt;
        }
        bound = res[0].bound_shape;
    }

    // `classify_shape` nests the seatable geometry under its own `frame` key
    // (sibling of `kind`/`surfaceType`) — `solve_mate_placement` wants that
    // nested object directly. Absent (e.g. a non-planar/non-cylindrical
    // face) ⇒ an empty object, which every `read_vec3` call below fails on,
    // same as any other unresolvable frame.
    const json classify_result = session::classify_shape(bound);
    const json frame = classify_result.contains("frame") ? classify_result["frame"] : json::object();
    const std::string snap_kind = read_str(mate, "kind");
    const bool flipped =
        mate.contains("flipped") && mate["flipped"].is_boolean() && mate["flipped"].get<bool>();

    // Seat anchor: the component's OWN current frozen translate stands in for
    // the live cursor a fresh interactive placement would use (see
    // ComponentMateSolver.h) — preserves how far along the axis / where on
    // the plane the user originally seated it as the target moves, rather
    // than snapping to the frame's raw origin every tick.
    std::array<double, 3> seat_anchor{0.0, 0.0, 0.0};
    if (frozen.contains("translate") && frozen["translate"].is_array() &&
        frozen["translate"].size() == 3) {
        for (std::size_t i = 0; i < 3; ++i) {
            const json& v = frozen["translate"][i];
            if (v.is_number()) {
                seat_anchor[i] = v.get<double>();
            } else if (v.is_object() && v.contains("value") && v["value"].is_number()) {
                seat_anchor[i] = v["value"].get<double>();
            }
        }
    }

    // `mate.selfFrame` (WP-F1.1, SCHEMA §7.3): the component-local basis the
    // attachment seats from, frozen into the record at authoring. Absent on
    // every pre-WP-F1.1 record ⇒ the identity frame ⇒ the seat solve is
    // arithmetically untouched.
    const json self_frame = mate.contains("selfFrame") ? mate["selfFrame"] : json();

    gp_Trsf frozen_trsf, candidate_trsf;
    std::string terr;
    const bool frozen_ok = read_placement(json{{"placement", frozen}}, "mate", frozen_trsf, terr);

    // WP-F1.1 anchor correction, and it is LOAD-BEARING: `solve_mate_placement`
    // anchors on the world SEAT point, which with an attachment frame is where
    // the ATTACHMENT sits — not where the body origin sits. Feeding it the raw
    // frozen translate would re-subtract the frame offset on every regen and
    // walk the component down the axis a frame-length per tick. Transforming
    // the frame's local origin by the frozen placement recovers the seat, and
    // makes a re-seat that changed nothing an exact fixed point.
    if (frozen_ok && self_frame.is_object()) {
        std::array<double, 3> local{0.0, 0.0, 0.0};
        if (self_frame.contains("origin") && self_frame["origin"].is_array() &&
            self_frame["origin"].size() == 3) {
            for (std::size_t i = 0; i < 3; ++i) {
                const json& v = self_frame["origin"][i];
                if (v.is_number()) local[i] = v.get<double>();
            }
        }
        const gp_Pnt seat_point = gp_Pnt(local[0], local[1], local[2]).Transformed(frozen_trsf);
        seat_anchor = {seat_point.X(), seat_point.Y(), seat_point.Z()};
    }

    const std::optional<json> candidate =
        onecad::ops::solve_mate_placement(snap_kind, frame, seat_anchor, flipped, self_frame);
    if (!candidate) {
        needs_repair_out.push_back(mate_unresolved_repair(
            ref_id, ref.element_id,
            "mate target resolved but its geometry can't seat a " + snap_kind + " mate"));
        return std::nullopt;
    }

    if (!frozen_ok || !read_placement(json{{"placement", *candidate}}, "mate", candidate_trsf, terr)) {
        // Both placements are worker-authored (frozen was validated at
        // authoring; candidate was just built by `solve_mate_placement`) —
        // a parse failure here is an internal defect, not a repair signal.
        // Treat as "did not move": the frozen placement stands.
        return std::nullopt;
    }

    const gp_XYZ dt = frozen_trsf.TranslationPart() - candidate_trsf.TranslationPart();
    const double translation_delta_mm = std::sqrt(dt.X() * dt.X() + dt.Y() * dt.Y() + dt.Z() * dt.Z());
    // Rotation delta: how far the component's local +Z axis direction
    // differs between the two transforms — the axis both solved orientations
    // are built around (ComponentMateSolver's own local-Z convention), a
    // robust proxy that avoids depending on a specific quaternion API.
    const gp_Dir local_z(0.0, 0.0, 1.0);
    const gp_Dir zf = local_z.Transformed(frozen_trsf);
    const gp_Dir zc = local_z.Transformed(candidate_trsf);
    const double rotation_delta_deg = zf.Angle(zc) * 180.0 / kPi;

    if (translation_delta_mm <= kMateReseatTranslationEpsilonMm &&
        rotation_delta_deg <= kMateReseatRotationEpsilonDeg) {
        return std::nullopt;  // within tolerance — a no-op tick, frozen stands
    }
    return MateReseat{*candidate, candidate_trsf};
}

// A CONVERTED codec must pin the binary format version it was written in, and a
// pin this worker does not write fails loudly rather than being misparsed (the
// same rule `ImportStep` enforces). Shared by every blob-backed source kind.
std::string format_pin_error(const json& source, const std::string& codec, int expected,
                             const std::string& op_label) {
    if (!source.contains("brepFormat") || !source["brepFormat"].is_number_integer()) {
        return op_label + ": source codec '" + codec + "' requires an integer brepFormat";
    }
    const int got = source["brepFormat"].get<int>();
    if (got != expected) {
        return op_label + ": source brepFormat " + std::to_string(got) +
               " is not the version this worker writes (" + std::to_string(expected) + ")";
    }
    return {};
}

// A `profile` refusal: a RECOVERABLE `OP_FAILED` whose fine-grained reason rides
// `detail.diagnostics[].reasonCode` (SCHEMA §8). The top-level `error.code`
// taxonomy is closed and never grows a value for a new refusal reason.
OpOutcome profile_refusal(const char* reason_code, const std::string& message) {
    OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", "OP_FAILED"},
                                   {"message", message},
                                   {"stage", "build"},
                                   {"reasonCode", reason_code}});
    return failure;
}

// `source.params.length` in mm. Unlike a `document` source's `params`
// (provenance only, never read here) this one IS a regen input — it is the
// entire point of the kind — so it is validated, not defaulted.
bool read_profile_length(const json& source, double& length_mm) {
    if (!source.contains("params") || !source["params"].is_object()) return false;
    const json& params = source["params"];
    if (!params.contains("length")) return false;
    const json& value = params["length"];
    if (value.is_number()) {
        length_mm = value.get<double>();
    } else if (value.is_object() && value.contains("value") && value["value"].is_number()) {
        length_mm = value["value"].get<double>();
    } else {
        return false;
    }
    return std::isfinite(length_mm) && length_mm > 0.0 && length_mm <= kMaxProfileLengthMm;
}

// The §7.3 `profile` arm (Component Library WP-C): a LENGTH-PARAMETRIC extrusion
// of an embedded canonical planar profile. It exists because vendor stock
// arrives as one fixed length — an aluminium extrusion STEP is a 500 mm stick —
// while the component has to be placeable at any length, which no baked-solid
// kind can express.
//
// `codec` is pinned to `brep` BY NAME: the `step` and `xbf` readers on this lane
// return SOLIDS, so accepting them would answer a face question with a solid
// reader and fail obscurely much later.
//
// Returns `nullopt` on success (`solid_out` filled), else the failing outcome.
std::optional<OpOutcome> build_profile_solid(const json& source, const std::string& op_label,
                                             TopoDS_Shape& solid_out) {
    const std::string codec = read_str(source, "codec");
    if (codec != "brep") {
        return profile_refusal("PROFILE_CODEC_UNSUPPORTED",
                               op_label + ": a profile source must be codec 'brep' (got '" +
                                   codec + "') — the step and xbf readers on this lane return "
                                           "solids, not a face");
    }
    const std::string bad = format_pin_error(source, codec, io::kBrepFormatVersion, op_label);
    if (!bad.empty()) return profile_refusal("PROFILE_FORMAT_UNSUPPORTED", bad);

    // Same "an unmaterialized blob lowers an EMPTY path so only THAT step fails"
    // rule the other blob kinds follow — and the same GENERIC `OP_FAILED` those
    // two legs carry there. They are blob-plumbing failures, identical for every
    // blob-backed kind, so they are deliberately NOT given a PROFILE_* reason
    // code: the fine-grained vocabulary describes the FACE, not the file system.
    const std::string path = read_str(source, "path");
    if (path.empty()) {
        return OpOutcome::fail("OP_FAILED",
                               op_label + ": no materialized geometry for source blob '" +
                                   read_str(source, "sha256") + "'");
    }
    std::error_code ec;
    if (!std::filesystem::is_regular_file(path, ec)) {
        return OpOutcome::fail("OP_FAILED", op_label + ": source geometry path is not a readable "
                                                       "file (" +
                                                path + ")");
    }

    const io::BrepShapeResult read = io::read_brep_shape(path);
    if (!read.ok()) return OpOutcome::fail("OP_FAILED", op_label + ": " + read.error);
    if (read.shape.ShapeType() != TopAbs_FACE) {
        return profile_refusal("PROFILE_BLOB_NOT_ONE_FACE",
                               op_label + ": a profile source must carry exactly one face, bare "
                                          "or in a single-child compound");
    }
    const TopoDS_Face face = TopoDS::Face(read.shape);
    gp_Pln plane;
    gp_Dir normal;
    if (!planar_face_plane_normal(face, plane, normal)) {
        return profile_refusal("PROFILE_FACE_NOT_PLANAR",
                               op_label + ": the profile face is not planar");
    }
    // The AREA centroid is checked too, and it costs one GProp call — no
    // eigen-solve, so the "platform-sensitive degenerate branch" argument that
    // keeps the principal-axis half an authoring rule does not reach it. Without
    // this an off-centre face places the component that far from its
    // `placement.translate` and NOTHING refuses: the part is silently in the
    // wrong place, which is the failure class this whole design exists to stop.
    GProp_GProps area_props;
    BRepGProp::SurfaceProperties(face, area_props);
    const double centroid_offset = area_props.CentreOfMass().XYZ().Modulus();
    if (std::abs(plane.Distance(gp_Pnt(0.0, 0.0, 0.0))) > kProfileCanonicalTol ||
        (normal.XYZ() - gp_XYZ(0.0, 0.0, 1.0)).Modulus() > kProfileCanonicalTol ||
        centroid_offset > kProfileCanonicalTol) {
        return profile_refusal("PROFILE_FACE_NOT_CANONICAL",
                               op_label + ": the profile face must lie on the plane z = 0, with a "
                                          "+Z orientation-corrected normal and its area centroid "
                                          "on the origin");
    }

    double length_mm = 0.0;
    if (!read_profile_length(source, length_mm)) {
        return profile_refusal("PROFILE_LENGTH_INVALID",
                               op_label + ": source.params.length must be a finite length in "
                                          "(0, 100000] mm");
    }

    try {
        BRepPrimAPI_MakePrism prism(face, gp_Vec(0.0, 0.0, length_mm));
        prism.Build();
        if (!prism.IsDone() || prism.Shape().IsNull()) {
            return OpOutcome::fail("OP_FAILED", op_label + ": the profile prism failed to build");
        }
        solid_out = prism.Shape();
    } catch (const Standard_Failure& f) {
        return OpOutcome::fail("OP_FAILED",
                               op_label + ": the profile prism raised: " +
                                   (f.GetMessageString() != nullptr ? f.GetMessageString()
                                                                    : "OCCT failure"));
    }
    return std::nullopt;
}

// Reads the baked solid behind an `embedded` / `document` source (spec §2.1,
// WP-3.2) out of the wire-only `source.path` Rust materialized for it.
//
// The bytes are one of the §7.3 REPLAY codecs and are read with the SAME readers
// `ImportStep` uses — a component's cached geometry is an import source in every
// respect that matters, so it must not grow a second, subtly different reader.
// Three rules are carried over deliberately:
//   * a CONVERTED codec must pin the binary format version it was written in,
//     and a pin this worker does not write fails loudly rather than being
//     misparsed (`ImportOp.cpp`'s `check_format`);
//   * an absent path is `OP_FAILED` for THIS step only (Rust lowers an empty
//     path when a blob is not materialized — one broken component must never
//     take down the plan);
//   * `step` is accepted because spec §2.1 allows a plain STEP payload in an
//     `embedded` package, and refusing it would make the spec's own example
//     unplaceable.
//
// EXACTLY ONE SOLID (spec §9 `single_solid_policy`): a component resolves to one
// solid in v1, and a multi-solid blob is an authoring mistake that must be named,
// not silently reduced to its first solid.
//
// Returns `nullopt` on success (`solid_out` filled), else the failing outcome.
std::optional<OpOutcome> read_source_blob(const json& source, const std::string& op_label,
                                          const onecad::CancelToken* cancel,
                                          TopoDS_Shape& solid_out) {
    const std::string codec = read_str(source, "codec", "step");
    const std::string path = read_str(source, "path");
    if (path.empty()) {
        return OpOutcome::fail("OP_FAILED",
                               op_label + ": no materialized geometry for source blob '" +
                                   read_str(source, "sha256") + "'");
    }
    std::error_code ec;
    if (!std::filesystem::is_regular_file(path, ec)) {
        return OpOutcome::fail("OP_FAILED", op_label + ": source geometry path is not a readable "
                                                       "file (" +
                                                path + ")");
    }

    const auto check_format = [&](int expected) -> std::string {
        return format_pin_error(source, codec, expected, op_label);
    };

    std::vector<TopoDS_Shape> solids;
    if (codec == "brep") {
        const std::string bad = check_format(io::kBrepFormatVersion);
        if (!bad.empty()) return OpOutcome::fail("OP_FAILED", bad);
        const io::BrepReadResult read = io::read_brep_solids(path);
        if (!read.ok()) return OpOutcome::fail("OP_FAILED", op_label + ": " + read.error);
        solids = read.solids;
    } else if (codec == io::kXcafCodecName) {
        const std::string bad = check_format(io::kXcafFormatVersion);
        if (!bad.empty()) return OpOutcome::fail("OP_FAILED", bad);
        const io::XcafReadResult read = io::read_xcaf_solids(path);
        if (!read.ok()) return OpOutcome::fail("OP_FAILED", op_label + ": " + read.error);
        solids = read.solids;
    } else if (codec == "step") {
        const io::StepReadResult read = io::read_step(path, io::StepReadPolicy{}, cancel);
        if (read.cancelled) return OpOutcome::cancelled();
        if (!read.ok()) {
            return OpOutcome::fail("OP_FAILED",
                                   op_label + ": " + (read.error ? *read.error : "STEP read failed"));
        }
        solids = read.solids;
    } else {
        return OpOutcome::fail("OP_FAILED", op_label + ": unknown source codec '" + codec +
                                                "' (known: step, brep, " + io::kXcafCodecName + ")");
    }

    if (solids.size() != 1) {
        return OpOutcome::fail("OP_FAILED",
                               op_label + ": a component must resolve to exactly one solid (spec "
                                          "§9); this source carries " +
                                   std::to_string(solids.size()));
    }
    if (solids[0].IsNull()) {
        return OpOutcome::fail("OP_FAILED", op_label + ": source geometry is a null shape");
    }
    solid_out = solids[0];
    return std::nullopt;
}

// Shared pipeline behind BOTH `PlaceComponent` and `DetachComponent`
// (identical geometry construction — the two differ only in what the
// RECORD's params carry: PlaceComponent keeps a library identity + optional
// mate, DetachComponent carries neither, per spec §3.4's "no component_*
// fields remain"). `op_label` names the caller in error messages.
//
// `source.kind` is `"generator"`, `"embedded"`, `"document"` or `"profile"`
// (spec §2.1). The two BAKED blob kinds share ONE arm (`read_source_blob`):
// they differ
// only in what the RECORD remembers about the solid's provenance, never in how
// the solid is produced — a baked solid read back from this document's own
// cached bytes. The generator arm dispatches on `source.generatorId` through
// `ComponentGenerators.h` (WP-A1) — before that every id built an ISO 4762
// SHCS, which is the silent-substitution failure spec §0 invariant 4 forbids
// once a second family exists. Sizing comes from `source.params.thread`/
// `.length`/`.thread_detail`, defaulted for byte-identical P0/P1 behavior when
// absent.
OpOutcome resolve_source_and_publish(OpContext& ctx, const json& params, const std::string& op_id,
                                     const std::string& op_label) {
    if (!params.contains("source") || !params["source"].is_object()) {
        return OpOutcome::fail("OP_FAILED", op_label + " requires a source object");
    }
    const json& source = params["source"];
    const std::string kind = read_str(source, "kind");
    if (kind != "generator" && kind != "embedded" && kind != "document" && kind != "profile") {
        return OpOutcome::fail("UNSUPPORTED",
                               op_label + ": source.kind '" + kind +
                                   "' is unknown (known: generator, embedded, document, profile)");
    }
    TopoDS_Shape solid;
    std::string err;

    if (kind == "profile") {
        if (std::optional<OpOutcome> failure = build_profile_solid(source, op_label, solid)) {
            return *failure;
        }
        if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    } else if (kind != "generator") {
        if (std::optional<OpOutcome> failure =
                read_source_blob(source, op_label, ctx.cancel, solid)) {
            return *failure;
        }
        if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    } else {
        const std::string generator_id = read_str(source, "generatorId");
        if (generator_id.empty()) {
            return OpOutcome::fail("OP_FAILED", op_label + " source.generatorId must not be empty");
        }

        if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();

        // Free params live under `source.params` — the wire lowering has no
        // PlaceComponent special-case beyond `inputs[]`, so `ComponentSourceRef::
        // Generator.params` reaches here verbatim. Defaults reproduce P0/P1's
        // old hardcoded M6×20 exactly; `thread_detail` defaults to `cosmetic` for
        // the same byte-identical reason (WP-2.5).
        //
        // WP-F2: every STRING param is carried through verbatim rather than
        // named one by one here, so a family keyed by something other than a
        // thread (a bearing's `code`) needs no change to this layer.
        // `thread`/`thread_detail` are read back OUT of that map, which is
        // what keeps them from drifting from it.
        std::map<std::string, std::string> text_params;
        double length_mm = kDefaultLengthMm;
        bool length_given = false;
        if (source.contains("params") && source["params"].is_object()) {
            const json& gp = source["params"];
            for (auto it = gp.begin(); it != gp.end(); ++it) {
                if (it.value().is_string()) text_params[it.key()] = it.value().get<std::string>();
            }
            if (gp.contains("length")) {
                const json& l = gp["length"];
                if (l.is_number()) {
                    length_mm = l.get<double>();
                    length_given = true;
                } else if (l.is_object() && l.contains("value") && l["value"].is_number()) {
                    length_mm = l["value"].get<double>();
                    length_given = true;
                }
            }
        }
        const auto text_or = [&text_params](const char* key, const char* fallback) {
            const auto it = text_params.find(key);
            return it == text_params.end() ? std::string(fallback) : it->second;
        };
        const std::string thread = text_or("thread", kDefaultThread);
        const std::string thread_detail_str = text_or("thread_detail", kDefaultThreadDetail);
        if (!std::isfinite(length_mm) || length_mm <= 0.0) {
            return OpOutcome::fail("OP_FAILED",
                                   op_label + ": source.params.length must be finite and positive");
        }
        ThreadDetail thread_detail;
        if (!parse_thread_detail(thread_detail_str, thread_detail)) {
            return OpOutcome::fail("OP_FAILED", op_label + ": unknown thread_detail '" +
                                                    thread_detail_str +
                                                    "' — known values: cosmetic, simplified, modeled");
        }

        const GeneratorRequest req{op_label,      thread,        length_mm,
                                   length_given,  thread_detail, std::move(text_params)};
        if (!build_component(generator_id, req, solid, err)) {
            return OpOutcome::fail("OP_FAILED", err);
        }
    }

    // Tier-A input preflight, the same one every other mutating op runs — and the
    // one this op needs MOST: an `embedded`/`document` source is an untrusted BRep
    // blob out of a package or a container, not something this worker built. The
    // check sits BEFORE the transform so a null/degenerate shape is refused by
    // name instead of surfacing as an OCCT exception from `BRepBuilderAPI_Transform`.
    // A generator's own output goes through it too: cheap, and a generator whose
    // table drifted is exactly as broken as a bad blob.
    if (auto invalid = validate_modeling_input(solid, op_label, "source")) return *invalid;

    gp_Trsf trsf;
    if (!read_placement(params, op_label, trsf, err)) {
        return OpOutcome::fail("OP_FAILED", err);
    }

    // Component Library P3 WP-3.1 (spec §5.5): a mated instance re-resolves
    // its target every tick and re-seats if it moved. Absent on
    // DetachComponent's params by construction (spec §3.4 strips mate at
    // detach) — this branch simply never fires there.
    std::vector<json> mate_repairs;
    std::optional<json> mate_placement_echo;
    if (params.contains("mate") && params["mate"].is_object()) {
        if (const std::optional<MateReseat> reseated =
                resolve_mate_reseat(ctx, params["mate"], params["placement"], op_id, mate_repairs)) {
            trsf = reseated->trsf;
            mate_placement_echo = reseated->placement;
        }
    }

    try {
        BRepBuilderAPI_Transform xf(solid, trsf, /*Copy=*/Standard_True);
        if (!xf.IsDone() || xf.Shape().IsNull()) {
            return OpOutcome::fail("OP_FAILED", op_label + ": placement transform failed");
        }
        solid = xf.Shape();
    } catch (const Standard_Failure& f) {
        return OpOutcome::fail("OP_FAILED", op_label + ": placement transform raised: " +
                                                (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
    }

    if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    // spec §9: a component resolves to exactly ONE solid in v1 — the same
    // publication policy every other NewBody-minting op satisfies.
    const kernel::validation::PublicationDecision decision = publication_decision(
        solid,
        kernel::validation::single_solid_policy(op_label, kernel::validation::PublicationTier::TierA));
    if (!decision.publishable()) {
        return publication_refusal(decision, "publication");
    }

    OpOutcome out;
    const std::string bid = "body_" + op_id;
    ctx.bodies.create(bid, op_id, solid);
    out.body_events.push_back({"created", bid, {}});
    out.body_ids.push_back(bid);
    for (json& r : mate_repairs) out.needs_repair.push_back(std::move(r));
    if (mate_placement_echo) out.mate_placement = std::move(*mate_placement_echo);
    return out;
}

}  // namespace

OpOutcome execute_place_component(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();
    return resolve_source_and_publish(ctx, params, op_id, "PlaceComponent");
}

OpOutcome execute_detach_component(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();
    return resolve_source_and_publish(ctx, params, op_id, "DetachComponent");
}

}  // namespace onecad::ops
