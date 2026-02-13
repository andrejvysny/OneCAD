/**
 * @file OperationMetadata.h
 * @brief Optional additive metadata for operation records.
 */
#ifndef ONECAD_APP_DOCUMENT_OPERATIONMETADATA_H
#define ONECAD_APP_DOCUMENT_OPERATIONMETADATA_H

#include <QString>

#include <cstdint>

namespace onecad::app {

struct DeterminismSettings {
    bool parallel = false;
    QString occtOptionsHash;
    QString tolerancePolicyHash;
    QString solverPolicyHash;
};

struct SelectionAnchor {
    bool hasWorldPoint = false;
    double x = 0.0;
    double y = 0.0;
    double z = 0.0;
    bool hasUv = false;
    double u = 0.0;
    double v = 0.0;
};

struct OperationMetadata {
    std::uint32_t recordSchemaVersion = 1;
    std::uint32_t stepIndex = 0;
    QString uiAlias;
    bool replayOnly = false;
    DeterminismSettings determinism;
    SelectionAnchor anchor;
};

} // namespace onecad::app

#endif // ONECAD_APP_DOCUMENT_OPERATIONMETADATA_H
