#include "io/HistoryIO.h"
#include "app/document/OperationMetadata.h"
#include "app/document/OperationRecord.h"

#include <QJsonDocument>
#include <QJsonObject>

#include <cassert>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>

namespace {

std::string trim(const std::string& text) {
    const auto first = text.find_first_not_of(" \t\r\n");
    if (first == std::string::npos) {
        return {};
    }
    const auto last = text.find_last_not_of(" \t\r\n");
    return text.substr(first, last - first + 1);
}

std::filesystem::path fixtureDir() {
    const std::filesystem::path sourcePath(__FILE__);
    return sourcePath.parent_path().parent_path() / "fixtures" / "onecad_v1";
}

} // namespace

int main() {
    const std::filesystem::path fixtures = fixtureDir();
    const std::filesystem::path opsPath = fixtures / "history_ops_legacy_basic.jsonl";
    const std::filesystem::path statePath = fixtures / "history_state_legacy_basic.json";

    std::ifstream opsFile(opsPath);
    if (!opsFile.is_open()) {
        std::cerr << "Unable to open fixture: " << opsPath << "\n";
        return 1;
    }

    std::size_t opCount = 0;
    std::string line;
    while (std::getline(opsFile, line)) {
        line = trim(line);
        if (line.empty()) {
            continue;
        }

        QJsonParseError parseError;
        const QJsonDocument jsonDoc = QJsonDocument::fromJson(QByteArray::fromStdString(line), &parseError);
        if (parseError.error != QJsonParseError::NoError || !jsonDoc.isObject()) {
            std::cerr << "Invalid JSON line in fixture\n";
            return 1;
        }

        const QJsonObject legacyJson = jsonDoc.object();
        const onecad::app::OperationRecord operation = onecad::io::HistoryIO::deserializeOperation(legacyJson);
        if (operation.opId.empty()) {
            std::cerr << "Deserialized operation has empty opId\n";
            return 1;
        }

        const QJsonObject roundtripLegacy = onecad::io::HistoryIO::serializeOperation(operation);
        if (roundtripLegacy.contains("meta")) {
            std::cerr << "Legacy serialization unexpectedly contains meta\n";
            return 1;
        }

        onecad::app::OperationMetadata metadata;
        metadata.recordSchemaVersion = 1;
        metadata.stepIndex = static_cast<std::uint32_t>(opCount);
        metadata.uiAlias = (operation.type == onecad::app::OperationType::Extrude)
            ? QStringLiteral("PushPull")
            : QStringLiteral("Shell");
        metadata.replayOnly = (operation.type == onecad::app::OperationType::Shell);
        metadata.determinism.parallel = false;
        metadata.determinism.occtOptionsHash = QStringLiteral("hash-opt");
        metadata.anchor.hasWorldPoint = true;
        metadata.anchor.x = 1.0;
        metadata.anchor.y = 2.0;
        metadata.anchor.z = 3.0;

        const QJsonObject withMeta = onecad::io::HistoryIO::serializeOperation(operation, &metadata);
        if (!withMeta.contains("meta") || !withMeta["meta"].isObject()) {
            std::cerr << "Metadata serialization missing meta object\n";
            return 1;
        }

        QJsonObject unknownExtended = withMeta;
        unknownExtended["unknownField"] = QStringLiteral("legacy-safe");
        QJsonObject unknownMeta = unknownExtended["meta"].toObject();
        unknownMeta.insert("unknownMetaField", true);
        unknownExtended["meta"] = unknownMeta;

        const onecad::app::OperationRecord parsedWithUnknowns =
            onecad::io::HistoryIO::deserializeOperation(unknownExtended);
        if (parsedWithUnknowns.opId != operation.opId || parsedWithUnknowns.type != operation.type) {
            std::cerr << "Unknown-field compatibility parse mismatch\n";
            return 1;
        }

        ++opCount;
    }

    if (opCount == 0) {
        std::cerr << "No operations parsed from fixture\n";
        return 1;
    }

    std::ifstream stateFile(statePath);
    if (!stateFile.is_open()) {
        std::cerr << "Unable to open fixture: " << statePath << "\n";
        return 1;
    }
    const std::string state((std::istreambuf_iterator<char>(stateFile)), std::istreambuf_iterator<char>());
    QJsonParseError stateError;
    const QJsonDocument stateDoc = QJsonDocument::fromJson(QByteArray::fromStdString(state), &stateError);
    if (stateError.error != QJsonParseError::NoError || !stateDoc.isObject()) {
        std::cerr << "Invalid state fixture JSON\n";
        return 1;
    }
    const QJsonObject cursor = stateDoc.object().value("cursor").toObject();
    if (cursor.value("appliedOpCount").toInt(-1) < 0) {
        std::cerr << "State fixture missing appliedOpCount\n";
        return 1;
    }

    std::cout << "HistoryIO compatibility fixture tests passed\n";
    return 0;
}
