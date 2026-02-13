/**
 * @file HistoryIO.h
 * @brief Serialization for operation history (JSONL format)
 */

#pragma once

#include <QJsonObject>
#include <QString>
#include <limits>
#include <unordered_map>
#include <vector>

namespace onecad::app {
class Document;
struct OperationRecord;
struct OperationMetadata;
}

namespace onecad::io {

class Package;

/**
 * @brief Serialization for history/ops.jsonl and history/state.json
 * 
 * Per FILE_FORMAT.md §7:
 * Uses JSON Lines format (one JSON object per line) for Git-friendly diffs.
 */
class HistoryIO {
public:
    /**
     * @brief Save operation history for a document.
     */
    static bool saveHistory(Package* package, const app::Document* document);

    /**
     * @brief Save operation history to package
     */
    static bool saveHistory(Package* package,
                            const std::vector<app::OperationRecord>& operations,
                            const std::unordered_map<std::string, bool>& suppressionState,
                            std::size_t appliedOpCount = std::numeric_limits<std::size_t>::max());
    
    /**
     * @brief Load operation history from package
     */
    static bool loadHistory(Package* package,
                            app::Document* document,
                            QString& errorMessage);
    
    /**
     * @brief Serialize single operation to JSON
     */
    static QJsonObject serializeOperation(const app::OperationRecord& op,
                                          const app::OperationMetadata* metadata = nullptr);
    
    /**
     * @brief Deserialize JSON to operation record
     */
    static app::OperationRecord deserializeOperation(const QJsonObject& json);
    
    /**
     * @brief Compute hash of operations for cache validation
     */
    static QString computeOpsHash(const std::vector<app::OperationRecord>& operations);
    
private:
    HistoryIO() = delete;
};

} // namespace onecad::io
