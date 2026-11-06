#pragma once

#include <cctype>
#include <filesystem>
#include <fstream>
#include <string>
#include <vector>

#include "io/SourceCapture.h"
#include "nlohmann/json.hpp"

namespace onecad::harness {

inline bool artifact_name(const std::string& name) {
    if (name.empty() || name == "." || name == "..") return false;
    for (unsigned char c : name) {
        if (!std::isalnum(c) && c != '-' && c != '_' && c != '.') return false;
    }
    return true;
}

inline bool expand_artifacts(nlohmann::json& value, const std::filesystem::path& root) {
    if (value.is_structured()) {
        for (auto& child : value) if (!expand_artifacts(child, root)) return false;
        return true;
    }
    if (!value.is_string()) return true;
    const std::string text = value.get<std::string>();
    constexpr auto path_prefix = "$artifact:";
    constexpr auto sha_prefix = "$sha256:";
    const bool path = text.starts_with(path_prefix);
    const bool sha = text.starts_with(sha_prefix);
    if (!path && !sha) return !text.starts_with("$artifact") && !text.starts_with("$sha256");
    const std::string name = text.substr(path ? 10 : 8);
    if (!artifact_name(name)) return false;
    const auto file = root / name;
    if (path) { value = file.string(); return true; }
    std::error_code ec;
    if (std::filesystem::symlink_status(file, ec).type() != std::filesystem::file_type::regular || ec) return false;
    std::ifstream in(file, std::ios::binary | std::ios::ate);
    if (!in) return false;
    const auto end = in.tellg();
    if (end < 0 || static_cast<std::uintmax_t>(end) > io::kMaxImportSourceBytes) return false;
    std::vector<std::uint8_t> bytes(static_cast<std::size_t>(end));
    in.seekg(0);
    if ((!bytes.empty() && !in.read(reinterpret_cast<char*>(bytes.data()), bytes.size())) ||
        in.peek() != std::char_traits<char>::eof() || in.bad()) return false;
    static constexpr std::uint8_t empty = 0;
    value = hashing::sha256_hex(bytes.empty() ? &empty : bytes.data(), bytes.size());
    return true;
}

}  // namespace onecad::harness
