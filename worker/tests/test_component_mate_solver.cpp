// test_component_mate_solver.cpp — Component Library P3 WP-3.1
// `solve_mate_placement` numerics. No framework: exit code == failure count.
// Mirrors `test_m6a_ops.cpp`'s shape.
//
// These cases are a DELIBERATE numeric-parity port of
// `src/modules/library/placementSolver.test.ts`'s own cases — same inputs,
// same expected outputs — since `ComponentMateSolver.cpp` is a verbatim port
// of that file's algorithm (see its header). A future edit to either side
// that silently diverges the two should be caught by keeping both test
// files' cases in lockstep.
#include <array>
#include <cmath>
#include <cstdio>
#include <string>

#include "nlohmann/json.hpp"
#include "ops/ComponentMateSolver.h"

using nlohmann::json;
using onecad::ops::solve_mate_placement;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

constexpr double kPi = 3.14159265358979323846;
using Vec3 = std::array<double, 3>;

bool approx_equal(const Vec3& a, const Vec3& b, double eps = 1e-6) {
    return std::abs(a[0] - b[0]) < eps && std::abs(a[1] - b[1]) < eps && std::abs(a[2] - b[2]) < eps;
}

// Rodrigues' rotation of world +Z by (axis, angleDeg) — an INDEPENDENT check
// that the solved rotation actually produces the requested direction, ported
// from `placementSolver.test.ts`'s own `rotateZ` helper (not shared code with
// `ComponentMateSolver.cpp`, on purpose — an oracle must not reuse the thing
// it's checking).
Vec3 rotate_z(const Vec3& axis, double angle_deg) {
    const double rad = angle_deg * kPi / 180.0;
    const double len = std::sqrt(axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]);
    const double inv = len > 0.0 ? 1.0 / len : 1.0;
    const Vec3 u{axis[0] * inv, axis[1] * inv, axis[2] * inv};
    const Vec3 v{0.0, 0.0, 1.0};
    const double cos_t = std::cos(rad);
    const double sin_t = std::sin(rad);
    const double dot_uv = u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    const Vec3 cross_uv{u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]};
    return {
        v[0] * cos_t + cross_uv[0] * sin_t + u[0] * dot_uv * (1 - cos_t),
        v[1] * cos_t + cross_uv[1] * sin_t + u[1] * dot_uv * (1 - cos_t),
        v[2] * cos_t + cross_uv[2] * sin_t + u[2] * dot_uv * (1 - cos_t),
    };
}

Vec3 translate_of(const json& placement) {
    return {placement["translate"][0].get<double>(), placement["translate"][1].get<double>(),
           placement["translate"][2].get<double>()};
}

Vec3 rotate_axis_of(const json& placement) {
    const json& a = placement["rotate"]["axis"];
    return {a[0].get<double>(), a[1].get<double>(), a[2].get<double>()};
}

double rotate_angle_of(const json& placement) {
    return placement["rotate"]["angleDeg"].get<double>();
}

// ── placementSolver.test.ts "solveCandidatePlacement" cases, ported 1:1. ──────────

void test_coincident_seats_at_pick_point_aligned_to_normal() {
    const json frame = {{"origin", {5.0, 5.0, 5.0}}, {"normal", {0.0, 0.0, 1.0}}};
    const auto placement = solve_mate_placement("coincident", frame, {1.0, 2.0, 3.0}, false);
    check(placement.has_value(), "coincident: resolves");
    if (!placement) return;
    check(approx_equal(translate_of(*placement), {1.0, 2.0, 3.0}),
          "coincident: seats exactly at the pick point");
    check(approx_equal(rotate_z(rotate_axis_of(*placement), rotate_angle_of(*placement)), {0.0, 0.0, 1.0}),
          "coincident: local +Z aligns to the target normal");
}

void test_coincident_flip_aligns_to_opposite_normal() {
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"normal", {0.0, 0.0, 1.0}}};
    const auto placement = solve_mate_placement("coincident", frame, {0.0, 0.0, 0.0}, true);
    check(placement.has_value(), "coincident+flip: resolves");
    if (!placement) return;
    check(approx_equal(rotate_z(rotate_axis_of(*placement), rotate_angle_of(*placement)),
                       {0.0, 0.0, -1.0}),
          "coincident+flip: local +Z aligns to the OPPOSITE of the target normal");
}

void test_concentric_projects_pick_onto_axis() {
    // Axis is the world Z line through (0,0,0); a pick off-axis at (3,4,7)
    // must project to (0,0,7) — the nearest point on the line.
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"axis", {0.0, 0.0, 1.0}}};
    const auto placement = solve_mate_placement("concentric", frame, {3.0, 4.0, 7.0}, false);
    check(placement.has_value(), "concentric: resolves");
    if (!placement) return;
    check(approx_equal(translate_of(*placement), {0.0, 0.0, 7.0}),
          "concentric: seats on the axis, projecting the pick onto the line");
    check(approx_equal(rotate_z(rotate_axis_of(*placement), rotate_angle_of(*placement)), {0.0, 0.0, 1.0}),
          "concentric: local +Z aligns to the target axis");
}

void test_concentric_and_coincident_ignores_the_pick() {
    const json frame = {{"origin", {2.0, 2.0, 2.0}}, {"axis", {0.0, 0.0, 1.0}}};
    const auto placement =
        solve_mate_placement("concentricAndCoincident", frame, {99.0, 99.0, 99.0}, false);
    check(placement.has_value(), "concentricAndCoincident: resolves");
    if (!placement) return;
    check(approx_equal(translate_of(*placement), {2.0, 2.0, 2.0}),
          "concentricAndCoincident: seats exactly at the frame origin (the rim center), ignoring "
          "the pick");
}

void test_coincident_with_no_normal_is_unresolvable() {
    const json frame = {{"origin", {0.0, 0.0, 0.0}}};
    const auto placement = solve_mate_placement("coincident", frame, {0.0, 0.0, 0.0}, false);
    check(!placement.has_value(), "coincident with no normal (a malformed frame): nullopt, not a guess");
}

void test_concentric_with_zero_length_axis_is_unresolvable() {
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"axis", {0.0, 0.0, 0.0}}};
    const auto placement = solve_mate_placement("concentric", frame, {1.0, 1.0, 1.0}, false);
    check(!placement.has_value(),
          "concentric with a degenerate zero-length axis: nullopt, not a divide-by-zero");
}

void test_unknown_snap_kind_is_unresolvable() {
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"normal", {0.0, 0.0, 1.0}}};
    const auto placement = solve_mate_placement("bogus", frame, {0.0, 0.0, 0.0}, false);
    check(!placement.has_value(), "unknown snap kind: nullopt");
}

// ── `rotationFromLocalZTo` cases, exercised indirectly through `coincident` (the
// only snap kind whose rotation axis is exactly `rotationFromLocalZTo`'s output
// with no further transform) — the identity and 180°-flip degenerate cases. ──────

void test_rotation_identity_when_target_is_local_z() {
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"normal", {0.0, 0.0, 1.0}}};
    const auto placement = solve_mate_placement("coincident", frame, {0.0, 0.0, 0.0}, false);
    check(placement.has_value(), "rotation identity: resolves");
    if (!placement) return;
    check(rotate_angle_of(*placement) == 0.0, "rotation identity: angleDeg is exactly 0 when target is +Z");
}

void test_rotation_180_flip_when_target_is_negative_z() {
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"normal", {0.0, 0.0, -1.0}}};
    const auto placement = solve_mate_placement("coincident", frame, {0.0, 0.0, 0.0}, false);
    check(placement.has_value(), "rotation 180 flip: resolves");
    if (!placement) return;
    check(std::abs(rotate_angle_of(*placement) - 180.0) < 1e-6, "rotation 180 flip: angleDeg is ~180");
    check(approx_equal(rotate_z(rotate_axis_of(*placement), rotate_angle_of(*placement)),
                       {0.0, 0.0, -1.0}),
          "rotation 180 flip: round-trips to the target direction despite the arbitrary axis choice");
}

}  // namespace

int main() {
    test_coincident_seats_at_pick_point_aligned_to_normal();
    test_coincident_flip_aligns_to_opposite_normal();
    test_concentric_projects_pick_onto_axis();
    test_concentric_and_coincident_ignores_the_pick();
    test_coincident_with_no_normal_is_unresolvable();
    test_concentric_with_zero_length_axis_is_unresolvable();
    test_unknown_snap_kind_is_unresolvable();
    test_rotation_identity_when_target_is_local_z();
    test_rotation_180_flip_when_target_is_negative_z();
    if (g_failures == 0) std::fprintf(stderr, "component_mate_solver: OK\n");
    return g_failures;
}
