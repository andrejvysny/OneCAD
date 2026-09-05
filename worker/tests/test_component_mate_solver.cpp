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
//
// RE-PINNED by kernel-hardening WP-I (2026-09-04, SCHEMA §7.3 "Coincident
// seat"): a `coincident` seat is now the anchor PROJECTED onto the resolved
// plane along its normal, so the three cases below whose anchor was OFF the
// frame's plane expect the projected seat instead of the raw anchor.
// `placementSolver.ts` took the same change in the same work package, so the
// lockstep above holds — its own cases move with these. The projection is the
// identity at pick time (the cursor hit point is already on the plane); only
// the regen lane, whose anchor is a frozen seat the target has since moved away
// from, can see the difference. Every `concentric` case is untouched — that
// branch already projected onto the axis line.
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
Vec3 rotate_about(const Vec3& axis, double angle_deg, const Vec3& v) {
    const double rad = angle_deg * kPi / 180.0;
    const double len = std::sqrt(axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]);
    const double inv = len > 0.0 ? 1.0 / len : 1.0;
    const Vec3 u{axis[0] * inv, axis[1] * inv, axis[2] * inv};
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

/// Rodrigues' rotation of world +Z specifically — the pre-WP-F1.1 cases all
/// check the seating direction, which is where local +Z ends up.
Vec3 rotate_z(const Vec3& axis, double angle_deg) {
    return rotate_about(axis, angle_deg, Vec3{0.0, 0.0, 1.0});
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
    // WP-I: the anchor (1,2,3) is 2 mm below the plane z = 5, so the seat is
    // its projection (1,2,5) — not the anchor verbatim.
    check(approx_equal(translate_of(*placement), {1.0, 2.0, 5.0}),
          "coincident: seats on the target plane, projecting the pick along the normal");
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

// ── WP-F1.1 attachment frames — the SAME cases as
// `placementSolver.test.ts`'s "solveCandidatePlacement with an attachment
// frame" block, with the same inputs and the same expected numbers. The FE
// ghost and this re-seat must agree or a component jumps the instant it is
// committed. ──────────────────────────────────────────────────────────────────

json self_frame(const Vec3& origin, const Vec3& z, const Vec3& x) {
    return json{{"origin", {origin[0], origin[1], origin[2]}},
               {"z", {z[0], z[1], z[2]}},
               {"x", {x[0], x[1], x[2]}}};
}

// `placement`'s rotation applied to a component-LOCAL direction — Rodrigues
// again, an independent oracle, never the solver's own matrices.
Vec3 apply_rotation(const json& placement, const Vec3& local) {
    return rotate_about(rotate_axis_of(placement), rotate_angle_of(placement), local);
}

// `placement` applied to a component-LOCAL point.
Vec3 apply_placement(const json& placement, const Vec3& local) {
    const Vec3 r = apply_rotation(placement, local);
    const Vec3 t = translate_of(placement);
    return {r[0] + t[0], r[1] + t[1], r[2] + t[2]};
}

void test_absent_self_frame_is_byte_identical() {
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"axis", {0.0, 0.0, 1.0}}};
    const auto bare = solve_mate_placement("concentric", frame, {3.0, 4.0, 7.0}, false);
    const auto null_frame = solve_mate_placement("concentric", frame, {3.0, 4.0, 7.0}, false, json());
    const auto not_an_object =
        solve_mate_placement("concentric", frame, {3.0, 4.0, 7.0}, false, json("nonsense"));
    check(bare.has_value() && null_frame.has_value() && not_an_object.has_value(),
          "absent selfFrame: resolves");
    if (!bare || !null_frame || !not_an_object) return;
    // Byte-identical, not merely close: the absent case must not route through
    // the matrix path at all, so no pre-WP-F1.1 document picks up float noise.
    check(*bare == *null_frame && *bare == *not_an_object,
          "absent selfFrame: byte-identical to the un-composed solve");
}

void test_offset_origin_seats_the_attachment_point() {
    // A component whose mate point sits 10mm up its own local +Z, seated on the
    // +Z plane z = 5: the pick (1,2,3) projects to the seat (1,2,5) (WP-I), so
    // the body must sit 10mm BELOW that seat, at (1,2,-5).
    const json frame = {{"origin", {5.0, 5.0, 5.0}}, {"normal", {0.0, 0.0, 1.0}}};
    const json sf = self_frame({0.0, 0.0, 10.0}, {0.0, 0.0, 1.0}, {1.0, 0.0, 0.0});
    const auto p = solve_mate_placement("coincident", frame, {1.0, 2.0, 3.0}, false, sf);
    check(p.has_value(), "offset origin: resolves");
    if (!p) return;
    check(approx_equal(translate_of(*p), {1.0, 2.0, -5.0}),
          "offset origin: body seats 10mm below the seat, not AT it");
    check(rotate_angle_of(*p) == 0.0, "offset origin: identity rotation is preserved exactly");
    check(approx_equal(apply_placement(*p, {0.0, 0.0, 10.0}), {1.0, 2.0, 5.0}),
          "offset origin: the ATTACHMENT point lands on the seat");
}

void test_rotated_frame_z_aligns_to_the_target_axis() {
    // The attachment's local +X is its seating axis. Target axis is world +Z
    // through the origin, pick projects to (0,0,4); the attachment origin is
    // 2mm out along local +X, so the body lands at (0,0,2).
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"axis", {0.0, 0.0, 1.0}}};
    const json sf = self_frame({2.0, 0.0, 0.0}, {1.0, 0.0, 0.0}, {0.0, 0.0, 1.0});
    const auto p = solve_mate_placement("concentric", frame, {0.0, 0.0, 4.0}, false, sf);
    check(p.has_value(), "rotated frame z: resolves");
    if (!p) return;
    check(approx_equal(translate_of(*p), {0.0, 0.0, 2.0}), "rotated frame z: body lands at (0,0,2)");
    check(approx_equal(apply_placement(*p, {2.0, 0.0, 0.0}), {0.0, 0.0, 4.0}),
          "rotated frame z: the attachment point lands on the seat");
    check(approx_equal(apply_rotation(*p, {1.0, 0.0, 0.0}), {0.0, 0.0, 1.0}),
          "rotated frame z: the attachment axis aligns to the target axis");
}

void test_non_degenerate_composed_rotation() {
    // Neither the seat rotation nor the frame rotation is the identity or a
    // 180° flip — the general arm of `axis_angle_from_mat`.
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"normal", {0.0, 1.0, 0.0}}};
    const json sf = self_frame({1.0, 2.0, 3.0}, {0.0, 0.0, 1.0}, {0.0, 1.0, 0.0});
    const auto p = solve_mate_placement("coincident", frame, {4.0, 5.0, 6.0}, false, sf);
    check(p.has_value(), "non-degenerate compose: resolves");
    if (!p) return;
    // WP-I: the pick (4,5,6) projects onto the y = 0 plane at (4,0,6).
    check(approx_equal(apply_placement(*p, {1.0, 2.0, 3.0}), {4.0, 0.0, 6.0}),
          "non-degenerate compose: the attachment point is ON the seat");
    check(approx_equal(apply_rotation(*p, {0.0, 0.0, 1.0}), {0.0, 1.0, 0.0}),
          "non-degenerate compose: the attachment +Z is ALONG the target normal");
    check(approx_equal(apply_rotation(*p, {0.0, 1.0, 0.0}), {1.0, 0.0, 0.0}),
          "non-degenerate compose: the roll is the frame's own, not an arbitrary one");
}

void test_flip_with_a_self_frame() {
    const json frame = {{"origin", {0.0, 0.0, 0.0}}, {"normal", {0.0, 0.0, 1.0}}};
    const json sf = self_frame({0.0, 0.0, 10.0}, {0.0, 0.0, 1.0}, {1.0, 0.0, 0.0});
    const auto p = solve_mate_placement("coincident", frame, {0.0, 0.0, 0.0}, true, sf);
    check(p.has_value(), "flip + frame: resolves");
    if (!p) return;
    check(approx_equal(apply_rotation(*p, {0.0, 0.0, 1.0}), {0.0, 0.0, -1.0}),
          "flip + frame: the attachment axis inverts");
    check(approx_equal(translate_of(*p), {0.0, 0.0, 10.0}),
          "flip + frame: the 10mm offset points the other way, so the body sits ABOVE");
    check(approx_equal(apply_placement(*p, {0.0, 0.0, 10.0}), {0.0, 0.0, 0.0}),
          "flip + frame: the attachment point still lands on the seat");
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
    test_absent_self_frame_is_byte_identical();
    test_offset_origin_seats_the_attachment_point();
    test_rotated_frame_z_aligns_to_the_target_axis();
    test_non_degenerate_composed_rotation();
    test_flip_with_a_self_frame();
    if (g_failures == 0) std::fprintf(stderr, "component_mate_solver: OK\n");
    return g_failures;
}
