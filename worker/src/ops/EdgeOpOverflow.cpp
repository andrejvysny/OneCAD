// EdgeOpOverflow.cpp — see EdgeOpOverflow.h.
#include "ops/EdgeOpOverflow.h"
#include "ops/TopologyHistory.h"

#include <memory>
#include <string>
#include <utility>

#include <NCollection_IndexedMap.hxx>
#include <TopExp.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/fillet/BlendOverflow.h"
#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;
namespace kf = onecad::kernel::fillet;

namespace {

OpOutcome with_unsupported(OpOutcome failure, const std::string& reason_code,
                           const std::string& message) {
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", "EDGE_OP_OVERFLOW_UNSUPPORTED"},
                                   {"stage", "overflow"},
                                   {"reasonCode", reason_code},
                                   {"message", message}});
    return failure;
}

// "f:<ordinal>" in the BEFORE body's face map; empty when the face is absent.
std::string face_ref(const TopTools_IndexedMapOfShape& faces, const TopoDS_Shape& face) {
    const int index = face.IsNull() ? 0 : faces.FindIndex(face);
    return index > 0 ? "f:" + std::to_string(index) : std::string();
}

// The seed's other adjacent face (the one that survives); null when ambiguous.
TopoDS_Face other_support(const TopoDS_Shape& before, const TopoDS_Edge& seed,
                          const TopoDS_Face& consumed) {
    TopTools_IndexedDataMapOfShapeListOfShape ancestors;
    TopExp::MapShapesAndAncestors(before, TopAbs_EDGE, TopAbs_FACE, ancestors);
    if (!ancestors.Contains(seed)) return TopoDS_Face();
    TopoDS_Face other;
    for (const TopoDS_Shape& face : ancestors.FindFromKey(seed)) {
        if (face.IsSame(consumed)) continue;
        if (!other.IsNull() && !other.IsSame(face)) return TopoDS_Face();
        other = TopoDS::Face(face);
    }
    return other;
}

json overflow_warning(const kf::OverflowResult& overflow, kf::EdgeOpMode mode,
                      const TopoDS_Shape& before, const TopoDS_Edge& seed) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(before, TopAbs_FACE, faces);
    const bool fillet = mode == kf::EdgeOpMode::Fillet;
    json evidence = {{"overflowKind", kf::overflow_kind_name(overflow.kind)},
                     {"setback", overflow.setback},
                     {"analyticRemovedVolume", overflow.analytic_removed_volume},
                     {"measuredRemovedVolume", overflow.measured_removed_volume}};
    evidence["consumedFace"] = face_ref(faces, overflow.consumed_face);
    evidence["survivingSupport"] =
        face_ref(faces, other_support(before, seed, overflow.consumed_face));
    return {{"severity", "warning"},
            {"code", fillet ? "FILLET_OVERFLOW_TRIMMED" : "CHAMFER_OVERFLOW_TRIMMED"},
            {"stage", "publication"},
            {"message", std::string(fillet ? "Fillet" : "Chamfer") +
                            " exceeded an adjacent face; the face was consumed and the "
                            "next face trimmed (Auto overflow)"},
            {"evidence", std::move(evidence)}};
}

using ShapeSet = NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher>;

// `shape`'s own image(s) inside `result`: itself, plus the Cut's Modified images.
void add_images(BRepAlgoAPI_Cut& cut, const TopoDS_Shape& shape, const ShapeSet& result,
                ShapeSet& images) {
    if (result.Contains(shape)) images.Add(shape);
    for (const TopoDS_Shape& modified : cut.Modified(shape)) {
        if (result.Contains(modified)) images.Add(modified);
    }
}

// Birth roots = the tool's referenceable topology MINUS anything that coincides
// with a surviving body sub-shape. On the fillet path the tool edge through the
// cliff point is the body's own cliff edge; claiming it as a birth would make that
// edge a ledger CONFLICT (birth + predecessor).
std::vector<TopoDS_Shape> tool_births(BRepAlgoAPI_Cut& cut, const TopoDS_Shape& before,
                                      const TopoDS_Shape& tool, const TopoDS_Shape& result) {
    ShapeSet in_result, body_images;
    for (const TopAbs_ShapeEnum type : {TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX}) {
        TopTools_IndexedMapOfShape mapped;
        TopExp::MapShapes(result, type, mapped);
        for (int i = 1; i <= mapped.Extent(); ++i) in_result.Add(mapped(i));
    }
    for (const TopAbs_ShapeEnum type : {TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX}) {
        TopTools_IndexedMapOfShape mapped;
        TopExp::MapShapes(before, type, mapped);
        for (int i = 1; i <= mapped.Extent(); ++i) add_images(cut, mapped(i), in_result, body_images);
    }
    std::vector<TopoDS_Shape> births;
    for (const TopoDS_Shape& root : referenceable_topology(tool)) {
        ShapeSet images;
        add_images(cut, root, in_result, images);
        bool shared = false;
        for (int i = 1; i <= images.Extent() && !shared; ++i) shared = body_images.Contains(images(i));
        if (!shared) births.push_back(root);
    }
    return births;
}

OpOutcome publish_overflow(OpContext& ctx, const std::string& target_id,
                           const std::string& op_id, const TopoDS_Shape& before,
                           const kf::OverflowResult& overflow, json warning) {
    OpOutcome out;
    out.diagnostics.push_back(std::move(warning));
    ctx.bodies.create(target_id, op_id, overflow.shape);
    ctx.partition.apply_history(target_id, overflow.shape, *overflow.cut, out.delta,
                                &out.needs_repair);
    out.body_events.push_back({"modified", target_id, {}});
    out.body_ids.push_back(target_id);
    out.topology_history.push_back(modified_body_history(
        target_id, before, overflow.shape, *overflow.cut,
        tool_births(*overflow.cut, before, overflow.tool, overflow.shape)));
    return out;
}

}  // namespace

std::optional<OpOutcome> try_edge_op_overflow(
    OpContext& ctx, const std::string& target_id, const std::string& op_id, kf::EdgeOpMode mode,
    const TopoDS_Edge& seed, double value, double value2, const TopoDS_Face& reference_face,
    const OpOutcome& native_failure) {
    const auto* target = ctx.bodies.get(target_id);
    if (!target) return std::nullopt;
    const TopoDS_Shape before = target->geom;
    kf::OverflowRequest request;
    request.body = before;
    request.seed = seed;
    request.mode = mode;
    request.value = value;
    request.value2 = value2;
    request.reference_face = reference_face;
    const kf::OverflowResult overflow = kf::build_overflow(request, ctx.cancel);
    if (overflow.cancelled) return OpOutcome::cancelled();
    if (!overflow.ok) {
        return with_unsupported(native_failure, overflow.reason_code, overflow.message);
    }
    return publish_overflow(ctx, target_id, op_id, before, overflow,
                            overflow_warning(overflow, mode, before, seed));
}

OpOutcome edge_op_overflow_fallback(
    OpContext& ctx, const std::string& target_id, const std::string& op_id, kf::EdgeOpMode mode,
    bool distance_angle, bool two_distance, double radius, double distance2,
    const std::vector<TopoDS_Edge>& seeds, const std::vector<TopoDS_Face>& seed_reference_faces,
    OpOutcome native_failure) {
    const bool eligible_failure =
        native_failure.status == OpOutcome::Status::Failed && native_failure.needs_repair.empty() &&
        (native_failure.error_code == "OP_FAILED" || native_failure.error_code == "GEOMETRY_INVALID");
    if (!eligible_failure) return native_failure;
    if (distance_angle) {
        return with_unsupported(std::move(native_failure), "OVERFLOW_UNSUPPORTED_MODE",
                                "Auto overflow does not support distance-angle chamfers");
    }
    if (seeds.size() != 1 || (two_distance && seed_reference_faces.size() != 1)) {
        return with_unsupported(std::move(native_failure), "OVERFLOW_NOT_ISOLATED",
                                "Auto overflow needs exactly one isolated edge");
    }
    const TopoDS_Face reference = two_distance ? seed_reference_faces[0] : TopoDS_Face();
    const double second = mode == kf::EdgeOpMode::Fillet ? 0.0
                          : two_distance                 ? distance2
                                                         : radius;
    std::optional<OpOutcome> outcome = try_edge_op_overflow(
        ctx, target_id, op_id, mode, seeds[0], radius, second, reference, native_failure);
    return outcome ? std::move(*outcome) : std::move(native_failure);
}

}  // namespace onecad::ops
