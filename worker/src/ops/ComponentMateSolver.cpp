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

// ── WP-F1.1 attachment-frame composition (see the header for the formula). ───────
// Ports `placementSolver.ts`'s `composeWithSelfFrame` and its helpers 1:1 —
// same matrix layout (row-major `m[row][col]`), same degenerate-case
// handling, so the two stay numerically comparable.

using Mat3 = std::array<std::array<double, 3>, 3>;

Mat3 mat_identity() {
    return Mat3{{{1.0, 0.0, 0.0}, {0.0, 1.0, 0.0}, {0.0, 0.0, 1.0}}};
}

// Rodrigues: the rotation matrix for (unit-ish `axis`, `angle_deg`).
Mat3 mat_from_axis_angle(const Vec3& axis, double angle_deg) {
    const std::optional<Vec3> u = vnormalize(axis);
    if (!u) return mat_identity();
    const double rad = angle_deg * kPi / 180.0;
    const double c = std::cos(rad);
    const double s = std::sin(rad);
    const double t = 1.0 - c;
    const double x = (*u)[0], y = (*u)[1], z = (*u)[2];
    return Mat3{{{t * x * x + c, t * x * y - s * z, t * x * z + s * y},
                 {t * x * y + s * z, t * y * y + c, t * y * z - s * x},
                 {t * x * z - s * y, t * y * z + s * x, t * z * z + c}}};
}

Mat3 mat_multiply(const Mat3& a, const Mat3& b) {
    Mat3 out{};
    for (std::size_t r = 0; r < 3; ++r) {
        for (std::size_t c = 0; c < 3; ++c) {
            out[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    return out;
}

Mat3 mat_transpose(const Mat3& m) {
    return Mat3{{{m[0][0], m[1][0], m[2][0]},
                 {m[0][1], m[1][1], m[2][1]},
                 {m[0][2], m[1][2], m[2][2]}}};
}

Vec3 mat_apply(const Mat3& m, const Vec3& v) {
    return {m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2]};
}

// The inverse of `mat_from_axis_angle` — the axis-angle the composed rotation
// must be re-expressed as, since `FrozenPlacement.rotate` is axis-angle and
// `read_placement` reads nothing else.
AxisAngle axis_angle_from_mat(const Mat3& m) {
    const double trace = m[0][0] + m[1][1] + m[2][2];
    const double c = std::max(-1.0, std::min(1.0, (trace - 1.0) * 0.5));
    if (c > 1.0 - kEps) return AxisAngle{Vec3{0.0, 0.0, 1.0}, 0.0};
    if (c < -1.0 + kEps) {
        // θ = π: the skew part vanishes, so recover the axis from `R + I`,
        // whose columns are all 2·u·uᵢ — take the longest to stay conditioned.
        const Mat3 k{{{m[0][0] + 1.0, m[0][1], m[0][2]},
                      {m[1][0], m[1][1] + 1.0, m[1][2]},
                      {m[2][0], m[2][1], m[2][2] + 1.0}}};
        Vec3 best{0.0, 0.0, 1.0};
        double best_len = 0.0;
        for (std::size_t col = 0; col < 3; ++col) {
            const Vec3 v{k[0][col], k[1][col], k[2][col]};
            const double len = vlength(v);
            if (len > best_len) {
                best_len = len;
                best = v;
            }
        }
        const std::optional<Vec3> axis = vnormalize(best);
        return AxisAngle{axis ? *axis : Vec3{0.0, 0.0, 1.0}, 180.0};
    }
    const double s = std::sqrt(1.0 - c * c);
    const Vec3 raw{(m[2][1] - m[1][2]) / (2.0 * s), (m[0][2] - m[2][0]) / (2.0 * s),
                   (m[1][0] - m[0][1]) / (2.0 * s)};
    const std::optional<Vec3> axis = vnormalize(raw);
    return AxisAngle{axis ? *axis : Vec3{0.0, 0.0, 1.0}, std::acos(c) * 180.0 / kPi};
}

struct SelfFrame {
    Vec3 origin{0.0, 0.0, 0.0};
    Vec3 z{0.0, 0.0, 1.0};
    Vec3 x{1.0, 0.0, 0.0};
};

// Reads `mate.selfFrame`. `nullopt` when the key is absent or not an object —
// the identity case, which the caller short-circuits so pre-WP-F1.1
// placements keep their exact former arithmetic. Individual missing axes take
// the identity default (an origin-only frame is a legal, common authoring
// case); a present-but-degenerate axis is clamped back to the identity rather
// than propagating a NaN into the transform.
std::optional<SelfFrame> read_self_frame(const nlohmann::json& j) {
    if (!j.is_object()) return std::nullopt;
    SelfFrame f;
    Vec3 v;
    if (read_vec3(j, "origin", v)) f.origin = v;
    if (read_vec3(j, "z", v)) {
        if (const std::optional<Vec3> n = vnormalize(v)) f.z = *n;
    }
    if (read_vec3(j, "x", v)) {
        if (const std::optional<Vec3> n = vnormalize(v)) f.x = *n;
    }
    // Gram-Schmidt guard: Rust normalizes before freezing, so this only
    // catches a hand-written record — an `x` parallel to `z` falls back to an
    // arbitrary perpendicular rather than collapsing the basis.
    const double d = vdot(f.z, f.x);
    const std::optional<Vec3> ortho =
        vnormalize(Vec3{f.x[0] - f.z[0] * d, f.x[1] - f.z[1] * d, f.x[2] - f.z[2] * d});
    f.x = ortho ? *ortho : arbitrary_perpendicular(f.z);
    return f;
}

// `S ∘ F⁻¹` expressed back as a `FrozenPlacement`: rotation `R_s · R_fᵀ`,
// translation `T_s − (R_s · R_fᵀ) · origin_f`.
nlohmann::json compose_with_self_frame(const Vec3& seat, const AxisAngle& seat_rot,
                                       const SelfFrame& f) {
    const Vec3 y = vcross(f.z, f.x);  // right-handed by construction
    // Columns are the frame's basis vectors: F maps local identity onto it.
    const Mat3 r_f{{{f.x[0], y[0], f.z[0]}, {f.x[1], y[1], f.z[1]}, {f.x[2], y[2], f.z[2]}}};
    const Mat3 r_m = mat_multiply(mat_from_axis_angle(seat_rot.axis, seat_rot.angle_deg),
                                  mat_transpose(r_f));
    const Vec3 shifted = mat_apply(r_m, f.origin);
    const AxisAngle aa = axis_angle_from_mat(r_m);
    return placement_json(Vec3{seat[0] - shifted[0], seat[1] - shifted[1], seat[2] - shifted[2]},
                          aa.axis, aa.angle_deg);
}

// The seat placement, composed with `self_frame` when one is present. An
// absent frame returns the un-composed placement VERBATIM — not a
// multiply-by-identity — so no pre-WP-F1.1 placement picks up float noise.
nlohmann::json seated(const Vec3& seat, const AxisAngle& seat_rot, const nlohmann::json& self_frame) {
    const std::optional<SelfFrame> f = read_self_frame(self_frame);
    if (!f) return placement_json(seat, seat_rot.axis, seat_rot.angle_deg);
    return compose_with_self_frame(seat, seat_rot, *f);
}

}  // namespace

std::optional<nlohmann::json> solve_mate_placement(const std::string& snap_kind,
                                                    const nlohmann::json& frame,
                                                    const std::array<double, 3>& seat_anchor,
                                                    bool flipped,
                                                    const nlohmann::json& self_frame) {
    // Ports `solveCandidatePlacement` (placementSolver.ts) verbatim,
    // `pickWorldPos` → `seat_anchor` (see header for what stands in for the
    // cursor during a regen re-seat).
    if (snap_kind == "coincident") {
        Vec3 raw_normal;
        if (!read_vec3(frame, "normal", raw_normal)) return std::nullopt;
        const std::optional<Vec3> normal = vnormalize(raw_normal);
        if (!normal) return std::nullopt;
        const Vec3 direction = flipped ? vneg(*normal) : *normal;
        const AxisAngle aa = rotation_from_local_z_to(direction);
        return seated(seat_anchor, aa, self_frame);
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
        return seated(seat, aa, self_frame);
    }
    return std::nullopt;
}

}  // namespace onecad::ops
