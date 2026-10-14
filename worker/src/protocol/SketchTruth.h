#pragma once

#include "nlohmann/json.hpp"
#include "sketch/ExactTruth.h"
#include "sketch/WireSketch.h"

namespace onecad::protocol {

std::string exact_state_token(core::sketch::ExactState state);
nlohmann::json exact_truth_fields(const core::sketch::ExactTruth& truth, const wire::WireIndex& index);
nlohmann::json exact_truth_diagnostics(const core::sketch::ExactTruth& truth,
                                      const wire::WireIndex& index);
nlohmann::json exact_refusal(const core::sketch::ExactTruth& truth, const wire::WireIndex& index);
nlohmann::json translation_refusal(const wire::TranslateResult& result);

}  // namespace onecad::protocol
