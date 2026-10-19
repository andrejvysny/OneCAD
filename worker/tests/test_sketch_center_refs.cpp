#include <cmath>
#include <cstdio>
#include <string>

#include "sketch/SketchPoint.h"
#include "sketch/WireSketch.h"

using nlohmann::json;
namespace sk = onecad::core::sketch;

namespace {
int failures = 0;
void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++failures;
    }
}

json curve(const std::string& type) {
    return {{"id", "curve"}, {"type", type}, {"center", {10, 20}}, {"centerRef", "point"},
            {"radius", 3}, {"startAngle", 0}, {"endAngle", 1},
            {"majorR", 6}, {"minorR", 3}};
}

json args(json entity) {
    // Curves can precede their referenced points on the wire.
    return {{"entities", json::array({entity, {{"id", "point"}, {"type", "Point"},
                                              {"at", {10, 20}}}})},
            {"constraints", json::array()}};
}

void shared_center(const std::string& type, int dof, std::size_t count) {
    auto wire = args(curve(type));
    auto tr = onecad::wire::translate(wire);
    check(tr.ok, type + " translates: " + tr.error);
    if (!tr.ok) return;
    const auto center = tr.index.resolve_point("point", "");
    check(tr.index.resolve_point("curve", "center") == center, type + " aliases centerRef");
    check(tr.index.handle_for(center) == "point", type + " preserves canonical point identity");
    check(tr.sketch->getAllEntities().size() == count, type + " has no duplicate center");
    check(tr.sketch->solve().success, type + " solves");
    const int actual = tr.sketch->getDegreesOfFreedom();
    check(actual == dof, type + " DOF expected " + std::to_string(dof) + ", got " + std::to_string(actual));
    auto* point = tr.sketch->getEntityAs<sk::SketchPoint>(center);
    point->setPosition({12, 24});
    onecad::wire::apply_solved_positions(wire, *tr.sketch, tr.index);
    check(wire["entities"][0]["center"] == json::array({12.0, 24.0}), type + " echoes shared center");
}

void invalid_refs(const std::string& type) {
    for (const json& ref : {json(nullptr), json(12), json(""), json("missing"), json("curve")}) {
        auto entity = curve(type);
        entity["centerRef"] = ref;
        const auto tr = onecad::wire::translate(args(entity));
        check(!tr.ok && !tr.error.empty(), type + " refuses invalid centerRef " + ref.dump());
    }
    auto wire = args(curve(type));
    wire["entities"].insert(wire["entities"].begin(),
        json{{"id", "wrong"}, {"type", "Circle"}, {"center", {0, 0}}, {"radius", 1}});
    wire["entities"][1]["centerRef"] = "wrong";
    check(!onecad::wire::translate(wire).ok, type + " refuses existing non-Point reference");
}

void legacy_center(const std::string& type) {
    auto entity = curve(type);
    entity.erase("centerRef");
    const auto tr = onecad::wire::translate({{"entities", json::array({entity})}});
    check(tr.ok && tr.index.handle_for(tr.index.resolve_point("curve", "center")) == "curve.center",
          type + " retains legacy inline-only center");
}

void point_identity_owns_coordinates(const std::string& type) {
    auto entity = curve(type);
    entity["center"] = {0, 0};
    auto wire = args(entity);
    auto tr = onecad::wire::translate(wire);
    check(tr.ok, type + " accepts legacy inline coordinate alongside reference");
    if (!tr.ok) return;
    onecad::wire::apply_solved_positions(wire, *tr.sketch, tr.index);
    check(wire["entities"][0]["center"] == json::array({10.0, 20.0}),
          type + " uses referenced point coordinates");
}

void circle_center_drag_and_constraint() {
    auto wire = args(curve("Circle"));
    auto tr = onecad::wire::translate(wire);
    if (!tr.ok) { check(false, tr.error); return; }
    const auto center = tr.index.resolve_point("point", "");
    tr.sketch->beginPointDrag(center);
    check(tr.sketch->solveWithDrag(center, {15, 25}).success, "shared circle center drag solves");
    tr.sketch->endPointDrag();
    onecad::wire::apply_solved_positions(wire, *tr.sketch, tr.index);
    check(wire["entities"][0]["center"] == json::array({15.0, 25.0}), "drag moves curve center");
    check(wire["entities"][1]["at"] == json::array({15.0, 25.0}), "drag moves authored Point");
    wire["constraints"] = json::array({{{"id", "fixed"}, {"type", "Fixed"}, {"entities", {"point"}}},
                                       {{"id", "radius"}, {"type", "Radius"}, {"entities", {"curve"}}, {"value", 3}}});
    auto fixed = onecad::wire::translate(wire);
    check(fixed.ok && fixed.sketch->solve().success && fixed.sketch->getDegreesOfFreedom() == 0,
          "point Fixed plus curve Radius fully constrain one shared circle");
}
}  // namespace

int main() {
    shared_center("Circle", 3, 2);
    shared_center("Arc", 5, 4);
    shared_center("Ellipse", 5, 2);
    for (const auto* type : {"Circle", "Arc", "Ellipse"}) {
        invalid_refs(type);
        legacy_center(type);
        point_identity_owns_coordinates(type);
    }
    circle_center_drag_and_constraint();
    return failures == 0 ? 0 : 1;
}
