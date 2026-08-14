#pragma once

#include <cstdint>

namespace onecad::protocol {

// SCHEMA §6 limits advertised by this worker. Producers must obey the same
// values so a control response never exceeds the transport contract it offered.
inline constexpr std::uint64_t kChunkSize = 1'048'576;
inline constexpr std::uint64_t kInitialBulkCredit = 8'388'608;

}  // namespace onecad::protocol
