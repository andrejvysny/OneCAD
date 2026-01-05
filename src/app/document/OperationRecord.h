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
    Extrude
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

using ExtrudeInput = std::variant<SketchRegionRef, FaceRef>;

struct ExtrudeParams {
    double distance = 0.0;
    double draftAngleDeg = 0.0;
    BooleanMode booleanMode = BooleanMode::NewBody;
};

struct OperationRecord {
    std::string opId;
    OperationType type = OperationType::Extrude;
    ExtrudeInput input;
    ExtrudeParams params;
    std::vector<std::string> resultBodyIds;
};

} // namespace onecad::app

#endif // ONECAD_APP_DOCUMENT_OPERATIONRECORD_H
