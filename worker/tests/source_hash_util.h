#pragma once
#include <fstream>
#include <iterator>
#include <string>
#include "util/Hashing.h"

inline std::string source_file_hash(const std::string& path) {
    std::ifstream input(path, std::ios::binary);
    if (!input) return std::string(64, '0');
    const std::string bytes((std::istreambuf_iterator<char>(input)), {});
    return onecad::hashing::sha256_hex(bytes);
}
