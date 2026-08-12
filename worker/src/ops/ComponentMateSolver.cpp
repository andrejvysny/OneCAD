// ComponentMateSolver.cpp — see ComponentMateSolver.h.
#include "ops/ComponentMateSolver.h"

#include <cmath>

namespace onecad::ops {

namespace {

using Vec3 = std::array<double, 3>;
constexpr double kEps = 1e-9;
constexpr double kPi = 3.14159265358979323846;

double vlength(const Vec3& v) { return std::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]); }

// Ports `normalize` (placementSolver.ts:38-42) — `nullopt` on a near-zero
// vector rather than throwing, so callers can fold it into the same
// NeedsRepair-not-a-guess fallback as every other unresolvable input here.
std::optional<Vec3> vnormalize(const Vec3& v) {
    const double len = vlength(v);
    if (len < kEps) return std::nullopt;
    return Vec3{v[0] / len, v[1] / len, v[2] / len};
}

double vdot(const Vec3& a, const Vec3& b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }

Vec3 vcross(const Vec3& a, const Vec3& b) {
    return {a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]};
}

Vec3 vneg(const Vec3& v) { return {-v[0], -v[1], -v[2]}; }

// Ports `arbitraryPerpendicular` (placementSolver.ts:53-56). `v` must already
// be unit length; the 180°-flip case always calls this with world +Z, which
// is never near-zero, so the nullopt path is unreachable there.
Vec3 arbitrary_perpendicular(const Vec3& v) {
    const Vec3 ref = std::abs(v[0]) < 0.9 ? Vec3{1.0, 0.0, 0.0} : Vec3{0.0, 1.0, 0.0};
    return *vnormalize(vcross(v, ref));
}

struct AxisAngle {
    Vec3 axis{0.0, 0.0, 1.0};
    double angle_deg = 0.0;
};

// Ports `rotationFromLocalZTo` (placementSolver.ts:65-77) verbatim, including
// its degenerate-case handling (`to ≈ +Z` identity; `to ≈ -Z` an arbitrary
// but geometrically correct 180° axis).
AxisAngle rotation_from_local_z_to(const Vec3& to) {
    const Vec3 target = *vnormalize(to);  // caller guarantees `to` is non-zero
    const Vec3 z{0.0, 0.0, 1.0};
    double c = vdot(z, target);
    c = std::max(-1.0, std::min(1.0, c));
    if (c > 1.0 - kEps) return AxisAngle{Vec3{0.0, 0.0, 1.0}, 0.0};
    if (c < -1.0 + kEps) return AxisAngle{arbitrary_perpendicular(z), 180.0};
    const Vec3 axis = *vnormalize(vcross(z, target));
    const double angle_deg = std::acos(c) * 180.0 / kPi;
    return AxisAngle{axis, angle_deg};
}

// Ports `projectPointOntoLine` (placementSolver.ts:80-85). `direction` must
// already be non-degenerate (checked by the caller before this is reached).
Vec3 project_point_onto_line(const Vec3& point, const Vec3& origin, const Vec3& direction) {
    const Vec3 dir = *vnormalize(direction);
    const Vec3 rel{point[0] - origin[0], point[1] - origin[1], point[2] - origin[2]};
    const double t = vdot(rel, dir);
    return {origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t};
}

bool read_vec3(const nlohmann::json& j, const char* key, Vec3& out) {
    if (!j.is_object() || !j.contains(key) || !j[key].is_array() || j[key].size() != 3) return false;
    for (std::size_t i = 0; i < 3; ++i) {
        if (!j[key][i].is_number()) return false;
        out[i] = j[key][i].get<double>();
    }
    return true;
}

nlohmann::json placement_json(const Vec3& translate, const Vec3& axis, double angle_deg) {
    return nlohmann::json{
        {"translate", {translate[0], translate[1], translate[2]}},
        {"rotate",
         {{"center", {0.0, 0.0, 0.0}}, {"axis", {axis[0], axis[1], axis[2]}}, {"angleDeg", angle_deg}}},
    };
}

}  // namespace

std::optional<nlohmann::json> solve_mate_placement(const std::string& snap_kind,
                                                    const nlohmann::json& frame,
                                                    const std::array<double, 3>& seat_anchor,
                                                    bool flipped) {
    // Ports `solveCandidatePlacement` (placementSolver.ts:127-150) verbatim,
    // `pickWorldPos` → `seat_anchor` (see header for what stands in for the
    // cursor during a regen re-seat).
    if (snap_kind == "coincident") {
        Vec3 raw_normal;
        if (!read_vec3(frame, "normal", raw_normal)) return std::nullopt;
        const std::optional<Vec3> normal = vnormalize(raw_normal);
        if (!normal) return std::nullopt;
        const Vec3 direction = flipped ? vneg(*normal) : *normal;
        const AxisAngle aa = rotation_from_local_z_to(direction);
        return placement_json(seat_anchor, aa.axis, aa.angle_deg);
    }
    if (snap_kind == "concentric" || snap_kind == "concentricAndCoincident") {
        Vec3 raw_axis, origin;
        if (!read_vec3(frame, "axis", raw_axis) || !read_vec3(frame, "origin", origin)) {
            return std::nullopt;
        }
        const std::optional<Vec3> axis_dir = vnormalize(raw_axis);
        if (!axis_dir) return std::nullopt;
        const Vec3 direction = flipped ? vneg(*axis_dir) : *axis_dir;
        const AxisAngle aa = rotation_from_local_z_to(direction);
        const Vec3 seat = (snap_kind == "concentricAndCoincident")
                              ? origin
                              : project_point_onto_line(seat_anchor, origin, *axis_dir);
        return placement_json(seat, aa.axis, aa.angle_deg);
    }
    return std::nullopt;
}

}  // namespace onecad::ops
