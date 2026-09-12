#include "session/FeaturePatternValidation.h"

#include <algorithm>
#include <cstdint>
#include <set>
#include <string>
#include <vector>

#include "nlohmann/json.hpp"

namespace onecad::session {
namespace {
using nlohmann::json;

ops::OpOutcome unsupported_source(const json& params,
                                  const std::vector<std::string>& types) {
    std::size_t index = 0;
    if (types.front() == "Sketch") {
        const std::size_t sketches = std::distance(
            types.begin(), std::find_if(types.begin(), types.end(),
                [](const std::string& type) { return type != "Sketch"; }));
        if (sketches > 2) {
            index = 2;
        } else if (sketches == types.size()) {
            index = sketches - 1;
        } else if ((sketches == 1 && types[sketches] == "Extrude") ||
                   types[sketches] == "Revolve") {
            index = sketches + 1;
            const bool extrude = types[sketches] == "Extrude";
            while (index < types.size() && (types[index] == "Fillet" ||
                   types[index] == "Chamfer" || (extrude && types[index] == "Hole")))
                ++index;
            if (index == types.size()) index = sketches;
        } else {
            index = sketches;
        }
    } else if (types.front() == "Hole") {
        index = 1;
        while (index < types.size() && types[index] == "Chamfer") ++index;
    }
    if (index >= types.size()) index = 0;
    const json& source = params["sourceOps"][index];
    return ops::OpOutcome::fail(
        "UNSUPPORTED_OP", "FeaturePattern source " +
        source.value("sourceRecordId", std::string()) + " index " +
        std::to_string(index) + " (" + source.value("opType", std::string()) +
        ") is not executable in this chain");
}

std::optional<ops::OpOutcome> validate_solid_profile(const json& params) {
    const json sketch = params["sourceOps"][0].value("params", json::object());
    const json extrude = params["sourceOps"][1].value("params", json::object());
    const std::string sketch_id = sketch.value("sketchId", std::string());
    if (sketch_id.empty() || extrude.value("sketchId", std::string()) != sketch_id)
        return ops::OpOutcome::fail(
            "OP_FAILED", "FeaturePattern source " +
            params["sourceOps"][1].value("sourceRecordId", std::string()) +
            " index 1 Extrude must explicitly reference source " +
            params["sourceOps"][0].value("sourceRecordId", std::string()) + " Sketch");
    if (extrude.value("extrudeMode", std::string("Blind")) != "Blind" ||
        extrude.value("booleanMode", std::string("NewBody")) != "NewBody" ||
        !extrude.value("targetBodyId", std::string()).empty() ||
        extrude.value("twoDirections", false))
        return ops::OpOutcome::fail(
            "UNSUPPORTED_OP", "FeaturePattern source " +
            params["sourceOps"][1].value("sourceRecordId", std::string()) +
            " index 1 supports only one-direction Blind NewBody Extrude");
    return std::nullopt;
}

std::optional<ops::OpOutcome> validate_revolve_profile(const json& params,
                                                       std::size_t index) {
    const json revolve = params["sourceOps"][index].value("params", json::object());
    if (revolve.value("booleanMode", std::string("NewBody")) != "NewBody" ||
        !revolve.value("targetBodyId", std::string()).empty())
        return ops::OpOutcome::fail(
            "UNSUPPORTED_OP", "FeaturePattern source " +
            params["sourceOps"][index].value("sourceRecordId", std::string()) +
            " index " + std::to_string(index) + " supports only NewBody Revolve");
    const json axis = revolve.value("axis", json::object());
    if (axis.value("kind", std::string()) != "sketchLine")
        return ops::OpOutcome::fail(
            "UNSUPPORTED_OP", "FeaturePattern source " +
            params["sourceOps"][index].value("sourceRecordId", std::string()) +
            " index " + std::to_string(index) +
            " supports only a selected sketch-line Revolve axis");
    std::set<std::string> selected;
    for (std::size_t i = 0; i < index; ++i)
        selected.insert(params["sourceOps"][i].value("params", json::object())
                            .value("sketchId", std::string()));
    const std::set<std::string> required = {
        revolve.value("sketchId", std::string()),
        axis.value("sketchId", std::string())};
    if (selected.contains("") || required.contains("") || selected != required ||
        selected.size() != index)
        return ops::OpOutcome::fail(
            "OP_FAILED", "FeaturePattern source " +
            params["sourceOps"][index].value("sourceRecordId", std::string()) +
            " index " + std::to_string(index) +
            " Revolve must directly consume every selected Sketch");
    return std::nullopt;
}
}

std::optional<ops::OpOutcome> validate_feature_pattern_sources(
    const json& op, json& params, int& count,
    FeaturePatternExecutionClass& execution_class,
    std::size_t& creator_index, std::string& host_body_id) {
    params = op.value("params", json::object());
    if (!params.contains("semanticsVersion") ||
        !params["semanticsVersion"].is_number_integer() ||
        params["semanticsVersion"].get<std::int64_t>() != 1)
        return ops::OpOutcome::fail("OP_FAILED", "unsupported FeaturePattern semanticsVersion");
    if (!params.contains("count") || !params["count"].is_number_integer())
        return ops::OpOutcome::fail("OP_FAILED", "FeaturePattern count must be an integer");
    const std::int64_t count64 = params["count"].get<std::int64_t>();
    if (count64 < 2 || count64 > 128 || !params.contains("sourceOps") ||
        !params["sourceOps"].is_array() || params["sourceOps"].empty())
        return ops::OpOutcome::fail("OP_FAILED", "invalid FeaturePattern count/sourceOps");
    count = static_cast<int>(count64);
    if (!params.contains("sourceRecordIds") || !params["sourceRecordIds"].is_array() ||
        params["sourceRecordIds"].size() != params["sourceOps"].size())
        return ops::OpOutcome::fail("OP_FAILED", "FeaturePattern sourceRecordIds/sourceOps mismatch");
    std::set<std::string> ids;
    std::vector<std::string> types;
    for (std::size_t i = 0; i < params["sourceOps"].size(); ++i) {
        const json& source = params["sourceOps"][i];
        if (!source.is_object())
            return ops::OpOutcome::fail("OP_FAILED", "FeaturePattern sourceOps entries must be objects");
        const std::string id = source.value("sourceRecordId", std::string());
        types.push_back(source.value("opType", std::string()));
        if (id.empty() || !params["sourceRecordIds"][i].is_string() ||
            params["sourceRecordIds"][i].get<std::string>() != id || !ids.insert(id).second)
            return ops::OpOutcome::fail("OP_FAILED", "FeaturePattern source IDs must be unique and ordered");
    }
    const bool hole_chain = types.front() == "Hole" &&
        std::all_of(types.begin() + 1, types.end(),
                    [](const std::string& type) { return type == "Chamfer"; });
    const bool solid_chain = types.size() >= 2 && types[0] == "Sketch" && types[1] == "Extrude" &&
        std::all_of(types.begin() + 2, types.end(), [](const std::string& type) {
            return type == "Hole" || type == "Fillet" || type == "Chamfer";
        });
    creator_index = solid_chain ? 1 : 0;
    const std::string shared_mode =
        solid_chain
            ? params["sourceOps"][1].value("params", json::object())
                  .value("booleanMode", std::string())
            : std::string();
    const bool shared_host = shared_mode == "Add" || shared_mode == "Cut";
    const std::size_t sketches = std::min<std::size_t>(
        2, std::distance(types.begin(),
                         std::find_if(types.begin(), types.end(),
                                      [](const std::string& type) { return type != "Sketch"; })));
    const bool revolve_chain = sketches >= 1 && sketches <= 2 &&
        types.size() > sketches && types[sketches] == "Revolve" &&
        std::all_of(types.begin() + sketches + 1, types.end(),
                    [](const std::string& type) {
                        return type == "Fillet" || type == "Chamfer";
                    });
    if (!hole_chain && !solid_chain && !revolve_chain && !shared_host)
        return unsupported_source(params, types);
    if (shared_host) {
        const json extrude = params["sourceOps"][1].value("params", json::object());
        host_body_id = extrude.value("targetBodyId", std::string());
        if (host_body_id.empty())
            return ops::OpOutcome::fail(
                "OP_FAILED", "FeaturePattern source " +
                params["sourceOps"][1].value("sourceRecordId", std::string()) +
                " index 1 shared-host Add/Cut requires targetBodyId");
        const std::string sketch_id = params["sourceOps"][0]
            .value("params", json::object()).value("sketchId", std::string());
        if (sketch_id.empty() || extrude.value("sketchId", std::string()) != sketch_id ||
            extrude.value("extrudeMode", std::string("Blind")) != "Blind" ||
            extrude.value("twoDirections", false))
            return ops::OpOutcome::fail(
                "UNSUPPORTED_OP", "FeaturePattern source " +
                params["sourceOps"][1].value("sourceRecordId", std::string()) +
                " index 1 shared-host Add/Cut requires the selected Sketch and one-direction Blind Extrude");
        execution_class = FeaturePatternExecutionClass::SharedHost;
        creator_index = 1;
        return std::nullopt;
    }
    if (solid_chain) {
        execution_class = FeaturePatternExecutionClass::IndependentBody;
        return validate_solid_profile(params);
    }
    if (revolve_chain) {
        execution_class = FeaturePatternExecutionClass::IndependentBody;
        creator_index = sketches;
        return validate_revolve_profile(params, creator_index);
    }
    execution_class = FeaturePatternExecutionClass::HostModifier;
    host_body_id = params["sourceOps"][0].value("params", json::object())
                       .value("targetBodyId", std::string());
    return std::nullopt;
}

}
