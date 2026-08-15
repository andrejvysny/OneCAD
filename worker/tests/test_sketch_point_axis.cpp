// SNAP P3 — the POINT-PAIR axis alignments (`HorizontalPoints` /
// `VerticalPoints`).
//
// A frontend H/V alignment guide asserts "these two POINTS share a coordinate".
// Neither existing kind says that:
//   * `Horizontal` / `Vertical` are LINE-only — they store one line id and
//     constrain that entity, so they cannot express an alignment between two
//     points that no single line joins;
//   * `HorizontalDistance(0)` / `VerticalDistance(0)` are user-visible DRIVING
//     DIMENSIONS with a value, which is a different thing to show, edit and
//     delete.
// These are the pins for the two new kinds: satisfaction, error, DOF, the
// solver actually MOVING a point onto the alignment, and the loud failure when
// a point handle cannot be resolved.
#include <cmath>
#include <cstdio>
#include <string>

#include "nlohmann/json.hpp"
#include "sketch/ConstraintApplicability.h"
#include "sketch/Sketch.h"
#include "sketch/SketchPoint.h"
#include "sketch/WireSketch.h"
#include "sketch/constraints/Constraints.h"
#include "sketch/solver/ConstraintSolver.h"

using nlohmann::json;
namespace sk = onecad::core::sketch;
namespace cs = onecad::core::sketch::constraints;

namespace {
int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

std::string uuid(unsigned int value) {
    char out[37];
    std::snprintf(out, sizeof(out), "00000000-0000-0000-0000-%012u", value);
    return out;
}

json point_entity(unsigned int id, double x, double y) {
    return {{"id", uuid(id)}, {"type", "Point"}, {"at", json::array({x, y})}};
}

json sketch_of(json entities, json constraints) {
    return {
        {"sketchId", "point-axis"},
        {"plane", {{"kind", "XY"}}},
        {"entities", std::move(entities)},
        {"constraints", std::move(constraints)},
    };
}

// ── error / satisfaction / DOF, straight on the constraint objects ───────────

void test_error_and_dof() {
    sk::Sketch sketch;
    const sk::EntityID a = sketch.addPoint(0.0, 0.0);
    const sk::EntityID b = sketch.addPoint(10.0, 3.0);

    cs::HorizontalPointsConstraint h(a, b);
    cs::VerticalPointsConstraint v(a, b);

    check(h.degreesRemoved() == 1, "HorizontalPoints removes one DOF");
    check(v.degreesRemoved() == 1, "VerticalPoints removes one DOF");
    check(h.type() == sk::ConstraintType::HorizontalPoints, "HorizontalPoints reports its type");
    check(v.type() == sk::ConstraintType::VerticalPoints, "VerticalPoints reports its type");
    check(h.typeName() == "HorizontalPoints", "HorizontalPoints type name");
    check(v.typeName() == "VerticalPoints", "VerticalPoints type name");

    // Horizontal alignment = equal Y; vertical = equal X.
    check(std::abs(h.getError(sketch) - 3.0) < 1e-12, "HorizontalPoints error is |dy|");
    check(std::abs(v.getError(sketch) - 10.0) < 1e-12, "VerticalPoints error is |dx|");
    check(!h.isSatisfied(sketch, 1e-6), "HorizontalPoints unsatisfied at dy = 3");
    check(!v.isSatisfied(sketch, 1e-6), "VerticalPoints unsatisfied at dx = 10");
    check(h.isSatisfied(sketch, 5.0), "…and satisfied within a 5mm tolerance");

    const auto refs = h.referencedEntities();
    check(refs.size() == 2 && refs[0] == a && refs[1] == b,
          "HorizontalPoints references both points, in order");

    // The icon sits between the two points, like every other point-pair kind.
    const gp_Pnt2d icon = h.getIconPosition(sketch);
    check(std::abs(icon.X() - 5.0) < 1e-9 && std::abs(icon.Y() - 1.5) < 1e-9,
          "HorizontalPoints icon is the midpoint");

    check(sk::getConstraintDOFReduction(sk::ConstraintType::HorizontalPoints) == 1,
          "solver DOF table knows HorizontalPoints");
    check(sk::getConstraintDOFReduction(sk::ConstraintType::VerticalPoints) == 1,
          "solver DOF table knows VerticalPoints");
}

void test_missing_point_is_unsatisfied_not_crashing() {
    sk::Sketch sketch;
    const sk::EntityID a = sketch.addPoint(0.0, 0.0);
    cs::HorizontalPointsConstraint h(a, "no-such-point");
    check(!h.isSatisfied(sketch, 1e-6), "an unresolvable point is never satisfied");
    check(std::isinf(h.getError(sketch)), "…and reports infinite error");
}

// ── through the solver: the constraint must actually MOVE a point ────────────

void test_solver_aligns_the_points() {
    json entities = json::array();
    entities.push_back(point_entity(1, 0.0, 0.0));
    entities.push_back(point_entity(2, 10.0, 3.0));
    json constraints = json::array();
    // Pin the first point so the solve has one obvious answer.
    constraints.push_back({{"id", uuid(50)},
                           {"type", "Fixed"},
                           {"entities", json::array({uuid(1)})}});
    constraints.push_back({{"id", uuid(51)},
                           {"type", "HorizontalPoints"},
                           {"entities", json::array({uuid(1), uuid(2)})}});

    auto translated = onecad::wire::translate(sketch_of(entities, constraints));
    check(translated.ok, "HorizontalPoints translates from the wire");
    if (!translated.ok) {
        std::fprintf(stderr, "  translate error: %s\n", translated.error.c_str());
        return;
    }
    const sk::SolveResult solve = translated.sketch->solve();
    check(solve.success, "a sketch carrying HorizontalPoints solves");

    const sk::EntityID moved = translated.index.wire_to_internal.at(uuid(2));
    const auto* p = dynamic_cast<const sk::SketchPoint*>(translated.sketch->getEntity(moved));
    check(p != nullptr, "the second point survives the solve");
    if (!p) return;
    check(std::abs(p->y()) < 1e-6, "the solver pulled the second point onto y = 0");
    // The alignment says NOTHING about x — it must not have been touched.
    check(std::abs(p->x() - 10.0) < 1e-6, "…and left its x alone");
}

void test_solver_vertical_alignment() {
    json entities = json::array();
    entities.push_back(point_entity(1, 0.0, 0.0));
    entities.push_back(point_entity(2, 10.0, 3.0));
    json constraints = json::array();
    constraints.push_back({{"id", uuid(50)},
                           {"type", "Fixed"},
                           {"entities", json::array({uuid(1)})}});
    constraints.push_back({{"id", uuid(51)},
                           {"type", "VerticalPoints"},
                           {"entities", json::array({uuid(1), uuid(2)})}});

    auto translated = onecad::wire::translate(sketch_of(entities, constraints));
    check(translated.ok, "VerticalPoints translates from the wire");
    if (!translated.ok) return;
    check(translated.sketch->solve().success, "a sketch carrying VerticalPoints solves");

    const sk::EntityID moved = translated.index.wire_to_internal.at(uuid(2));
    const auto* p = dynamic_cast<const sk::SketchPoint*>(translated.sketch->getEntity(moved));
    check(p != nullptr, "the second point survives the solve");
    if (!p) return;
    check(std::abs(p->x()) < 1e-6, "the solver pulled the second point onto x = 0");
    check(std::abs(p->y() - 3.0) < 1e-6, "…and left its y alone");
}

// ── the wire refuses an unresolved handle LOUDLY ─────────────────────────────

void test_unresolved_handle_fails_loudly() {
    json entities = json::array();
    entities.push_back(point_entity(1, 0.0, 0.0));
    json constraints = json::array();
    constraints.push_back({{"id", uuid(51)},
                           {"type", "HorizontalPoints"},
                           {"entities", json::array({uuid(1), uuid(999)})}});

    auto translated = onecad::wire::translate(sketch_of(entities, constraints));
    check(!translated.ok, "an unresolved point handle is a translation FAILURE");
    check(translated.error.find("HorizontalPoints") != std::string::npos,
          "…and the error names the kind that failed");
}

// ── applicability advertises the new kinds for two points ────────────────────

void test_applicability() {
    sk::Sketch sketch;
    const sk::EntityID a = sketch.addPoint(0.0, 0.0);
    const sk::EntityID b = sketch.addPoint(10.0, 3.0);

    using onecad::app::selection::SelectionItem;
    using onecad::app::selection::SelectionKind;
    std::vector<SelectionItem> selection;
    SelectionItem itemA;
    itemA.kind = SelectionKind::SketchPoint;
    itemA.id.elementId = a;
    SelectionItem itemB;
    itemB.kind = SelectionKind::SketchPoint;
    itemB.id.elementId = b;
    selection.push_back(itemA);
    selection.push_back(itemB);

    const auto result = onecad::ui::evaluateConstraintApplicability(&sketch, selection);
    check(result.isApplicable(sk::ConstraintType::HorizontalPoints),
          "two points offer HorizontalPoints");
    check(result.isApplicable(sk::ConstraintType::VerticalPoints),
          "two points offer VerticalPoints");
    // The LINE-only forms are NOT offered: no line joins these two points, so
    // there is no entity for them to constrain.
    check(!result.isApplicable(sk::ConstraintType::Horizontal),
          "two unconnected points do NOT offer the line-only Horizontal");
    check(!result.isApplicable(sk::ConstraintType::Vertical),
          "two unconnected points do NOT offer the line-only Vertical");
}

} // namespace

int main() {
    test_error_and_dof();
    test_missing_point_is_unsatisfied_not_crashing();
    test_solver_aligns_the_points();
    test_solver_vertical_alignment();
    test_unresolved_handle_fails_loudly();
    test_applicability();
    if (g_failures > 0) {
        std::fprintf(stderr, "%d check(s) failed\n", g_failures);
        return 1;
    }
    std::printf("ok\n");
    return 0;
}
