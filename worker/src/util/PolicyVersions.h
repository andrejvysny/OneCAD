#pragma once

#include "Hashing.h"

namespace onecad::policy {

inline constexpr unsigned kSolverPolicyVersion = 2;
inline constexpr unsigned kArtifactSchemaVersion = 1;
inline constexpr unsigned kDescriptorVersion = 1;
inline constexpr unsigned kResolverVersion = 1;
inline constexpr unsigned kQuantizationVersion = 1;
inline constexpr unsigned kSignatureVersion = 1;

inline std::string occt_fingerprint() {
    return hashing::hex16(hashing::fnv1a(std::string(ONECAD_OCCT_FINGERPRINT_SEED)));
}

}  // namespace onecad::policy
