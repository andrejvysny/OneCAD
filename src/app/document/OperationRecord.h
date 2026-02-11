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

// ─────────────────────────────────────────────────────────────────────────────
// Operation Types
// ─────────────────────────────────────────────────────────────────────────────

enum class OperationType {
    Extrude,
    Revolve,
    Fillet,
    Chamfer,
    Shell,
    Boolean
};

enum class BooleanMode {
    NewBody,
    Add,
    Cut,
    Intersect
};

// ─────────────────────────────────────────────────────────────────────────────
// Reference Types (for inputs and topology references)
// ─────────────────────────────────────────────────────────────────────────────

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

struct BodyRef {
    std::string bodyId;
};

// ─────────────────────────────────────────────────────────────────────────────
// Operation Input (primary input for any operation)
// ─────────────────────────────────────────────────────────────────────────────

using OperationInput = std::variant<
    std::monostate,      // No input (e.g., for Boolean which uses params)
    SketchRegionRef,     // Extrude/Revolve from sketch region
    FaceRef,             // Push/pull face, shell input
    BodyRef              // Fillet/chamfer/shell/boolean target
>;

// ─────────────────────────────────────────────────────────────────────────────
// Parameter Structs (operation-specific configuration)
// ─────────────────────────────────────────────────────────────────────────────

struct ExtrudeParams {
    double distance = 0.0;
    double draftAngleDeg = 0.0;
    BooleanMode booleanMode = BooleanMode::NewBody;
    std::string targetBodyId;               // Optional explicit boolean target body
};

struct RevolveParams {
    double angleDeg = 360.0;
    using AxisRef = std::variant<std::monostate, SketchLineRef, EdgeRef>;
    AxisRef axis;
    BooleanMode booleanMode = BooleanMode::NewBody;
    std::string targetBodyId;               // Optional explicit boolean target body
};

struct FilletChamferParams {
    enum class Mode { Fillet, Chamfer };
    Mode mode = Mode::Fillet;
    double radius = 0.0;                    // Fillet radius or chamfer distance
    std::vector<std::string> edgeIds;       // ElementMap edge IDs
    bool chainTangentEdges = true;          // Auto-expand to tangent chain
};

struct ShellParams {
    double thickness = 0.0;
    std::vector<std::string> openFaceIds;   // ElementMap face IDs to remove
};

struct BooleanParams {
    enum class Op { Union, Cut, Intersect };
    Op operation = Op::Union;
    std::string targetBodyId;               // Body to modify (result)
    std::string toolBodyId;                 // Body to boolean with
};

// ─────────────────────────────────────────────────────────────────────────────
// Operation Params Variant
// ─────────────────────────────────────────────────────────────────────────────

using OperationParams = std::variant<
    ExtrudeParams,
    RevolveParams,
    FilletChamferParams,
    ShellParams,
    BooleanParams
>;

// ─────────────────────────────────────────────────────────────────────────────
// Operation Record (single history entry)
// ─────────────────────────────────────────────────────────────────────────────

struct OperationRecord {
    std::string opId;                           // Unique operation ID (UUID)
    OperationType type = OperationType::Extrude;
    OperationInput input;                       // Primary input (region, face, or body)
    OperationParams params;                     // Operation-specific parameters
    std::vector<std::string> resultBodyIds;     // Bodies produced/modified by this op
};

// ─────────────────────────────────────────────────────────────────────────────
// Utility Functions
// ─────────────────────────────────────────────────────────────────────────────

inline const char* operationTypeName(OperationType type) {
    switch (type) {
        case OperationType::Extrude: return "Extrude";
        case OperationType::Revolve: return "Revolve";
        case OperationType::Fillet: return "Fillet";
        case OperationType::Chamfer: return "Chamfer";
        case OperationType::Shell: return "Shell";
        case OperationType::Boolean: return "Boolean";
        default: return "Unknown";
    }
}

inline const char* booleanModeName(BooleanMode mode) {
    switch (mode) {
        case BooleanMode::NewBody: return "NewBody";
        case BooleanMode::Add: return "Add";
        case BooleanMode::Cut: return "Cut";
        case BooleanMode::Intersect: return "Intersect";
        default: return "Unknown";
    }
}

inline const char* booleanOpName(BooleanParams::Op op) {
    switch (op) {
        case BooleanParams::Op::Union: return "Union";
        case BooleanParams::Op::Cut: return "Cut";
        case BooleanParams::Op::Intersect: return "Intersect";
        default: return "Unknown";
    }
}

} // namespace onecad::app

#endif // ONECAD_APP_DOCUMENT_OPERATIONRECORD_H
