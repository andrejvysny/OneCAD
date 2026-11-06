#pragma once

#include <vector>
#include "nlohmann/json.hpp"

namespace onecad::io {

// Until the source ledger is installed, no certified reference may reach a
// geometric fallback. Iterative traversal avoids recursion on hostile JSON.
inline const nlohmann::json* source_identity_ref(const nlohmann::json& root) {
    std::vector<const nlohmann::json*> pending{&root};
    while (!pending.empty()) {
        const auto* value = pending.back();
        pending.pop_back();
        if (value->is_object() && value->contains("importSourceIdentity")) return value;
        if (!value->is_structured()) continue;
        for (const auto& child : *value) pending.push_back(&child);
    }
    return nullptr;
}

inline bool contains_source_identity(const nlohmann::json& root) {
    return source_identity_ref(root) != nullptr;
}

}  // namespace onecad::io
