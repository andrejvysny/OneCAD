/**
 * @file OperationRecord.h
 * @brief History operation records for modeling features.
 */
#ifndef ONECAD_APP_DOCUMENT_OPERATIONRECORD_H
#define ONECAD_APP_DOCUMENT_OPERATIONRECORD_H

#include <string>
#include <variant>
#include <vector>

namespace onecad::app {

enum class OperationType {
    Extrude,
    Revolve
};

enum class BooleanMode {
    NewBody,
    Add,
    Cut
};

struct SketchRegionRef {
    std::string sketchId;
    std::string regionId;
};

struct FaceRef {
    std::string bodyId;
    std::string faceId;
};

struct SketchLineRef {
    std::string sketchId;
    std::string lineId;
};

struct EdgeRef {
    std::string bodyId;
    std::string edgeId;
};

using ExtrudeInput = std::variant<SketchRegionRef, FaceRef>;
// Revolve input is the same (Region or Face) plus an Axis (Line or Edge)
// But for now OperationRecord stores a single input field.
// We can use a variant for params to differentiate? 
// Or make 'input' more flexible?
// The current `ExtrudeInput input` field is tied to Extrude. 
// Let's rename or expand.
// Since we have `ExtrudeParams params`, maybe we should make `params` a variant too.

struct ExtrudeParams {
    double distance = 0.0;
    double draftAngleDeg = 0.0;
    BooleanMode booleanMode = BooleanMode::NewBody;
};

struct RevolveParams {
    double angleDeg = 360.0;
    using AxisRef = std::variant<std::monostate, SketchLineRef, EdgeRef>;
    AxisRef axis;
    // Axis can be a sketch line or a linear model edge.
    BooleanMode booleanMode = BooleanMode::NewBody;
};

using OperationParams = std::variant<ExtrudeParams, RevolveParams>;

struct OperationRecord {
    std::string opId;
    OperationType type = OperationType::Extrude;
    ExtrudeInput input; // Reusing ExtrudeInput (Region/Face) as "Primary Input"
    OperationParams params;
    std::vector<std::string> resultBodyIds;
};

} // namespace onecad::app

#endif // ONECAD_APP_DOCUMENT_OPERATIONRECORD_H
