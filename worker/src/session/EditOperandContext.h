#pragma once
#include "nlohmann/json.hpp"
namespace onecad::session {
struct ScratchJob;
// Reads only private prefix scratch; never executes the queried operation.
nlohmann::json edit_operand_context(ScratchJob& job, const nlohmann::json& operation);
}
