#include "session/EditOperandContext.h"

#include <cmath>
#include <BRepAdaptor_Curve.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <gp_Vec.hxx>

#include "ops/OffsetFaceOp.h"
#include "session/ClassifyElement.h"
#include "session/PlanExecutor.h"
#include "session/ScratchJob.h"
#include "session/ShapeMetrics.h"

namespace onecad::session {
using nlohmann::json;
namespace {
json xyz(const gp_XYZ& point) { return json::array({point.X(), point.Y(), point.Z()}); }

bool geometry(const TopoDS_Shape& input, const TopoDS_Shape& body, json& out) {
    TopTools_IndexedMapOfShape located;
    TopExp::MapShapes(body, input.ShapeType(), located);
    const int index = located.FindIndex(input);
    if (index == 0) return false;
    const TopoDS_Shape shape = located(index);
    out["classification"] = classify_shape_in_body(shape, body);
    if (shape.ShapeType() == TopAbs_FACE) {
        const auto sample = ops::offsetface::sample_face(TopoDS::Face(shape));
        if (!sample.ok) return false;
        out["point"] = xyz(sample.point.XYZ());
        out["direction"] = xyz(sample.normal.XYZ());
        return true;
    }
    if (shape.ShapeType() != TopAbs_EDGE) return false;
    const auto descriptor = elementmap::ElementMapPartition::describe(shape, body);
    if (!descriptor.hasOutward) return false;
    BRepAdaptor_Curve curve(TopoDS::Edge(shape));
    const double parameter = (curve.FirstParameter() + curve.LastParameter()) * 0.5;
    if (!std::isfinite(parameter)) return false;
    gp_Pnt point;
    gp_Vec tangent;
    curve.D1(parameter, point, tangent);
    if (tangent.Magnitude() <= 1e-12) return false;
    tangent.Normalize();
    gp_Vec direction(descriptor.outward);
    direction -= tangent * direction.Dot(tangent);
    if (direction.Magnitude() <= 1e-12) return false;
    direction.Normalize();
    out["point"] = xyz(point.XYZ());
    out["direction"] = xyz(direction.XYZ());
    out["tangent"] = xyz(tangent.XYZ());
    return true;
}
}

json edit_operand_context(ScratchJob& job, const json& operation) {
    json eligible = json::array();
    for (const auto& [id, body] : job.bodies.all())
        if (body.visible && body.modeling_eligible()) eligible.push_back(id);
    const auto resolved = resolve_read_operand_inputs(job, operation);
    const bool refused = job.stopped_reason != "completed" || resolved.status != CandidateResult::Status::Ok || !resolved.needs_repair.empty();
    json inputs = json::array();
    for (std::size_t index = 0; index < operation["inputs"].size(); ++index) {
        const json primary = operation["inputs"][index].value("primary", json::object());
        const auto body_id = primary.value("bodyId", std::string());
        const auto element_id = primary.value("elementId", std::string());
        const auto kind = primary.value("kind", std::string());
        json result{{"inputIndex", index}, {"bodyId", body_id}, {"elementId", element_id}, {"kind", kind}};
        const auto* body = job.bodies.get(body_id);
        const auto* entry = job.partition.find(element_id);
        if (!refused && body && kind == "body" && element_id == body_id && !body->geom.IsNull()) {
            const auto metrics = compute_shape_metrics(body->geom);
            bool valid = true;
            for (std::size_t i = 0; i < 3; ++i)
                valid = valid && std::isfinite(metrics.bbox_min[i]) &&
                    std::isfinite(metrics.bbox_max[i]) && metrics.bbox_min[i] <= metrics.bbox_max[i];
            if (valid) result["bounds"] = {{"min", metrics.bbox_min}, {"max", metrics.bbox_max}};
            else result["refusal"] = "operand body has no finite ordered bounds";
        } else if (refused || !body || !entry || entry->body_id != body_id || entry->shape.IsNull() ||
            elementmap::ElementMapPartition::kind_name(entry->kind) != kind) {
            result["refusal"] = "operand does not resolve unambiguously in the authoritative prefix";
        } else {
            try {
                if (!geometry(entry->shape, body->geom, result))
                    result["refusal"] = "operand has no supported analytic handle anchor";
            } catch (const Standard_Failure&) {
                result["refusal"] = "kernel could not measure the operand handle anchor";
            }
        }
        inputs.push_back(std::move(result));
    }
    return json{{"eligibleBodies", std::move(eligible)}, {"inputs", std::move(inputs)}};
}
}
