// MassProperties.cpp — see MassProperties.h. SCHEMA §7.5 `QueryMassProperties`.
#include "session/MassProperties.h"

#include <array>
#include <cmath>
#include <string>

#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <GProp_PrincipalProps.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "session/BodyStore.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;

namespace {

using Vec3 = std::array<double, 3>;

Vec3 to_vec3(const gp_Vec& v) {
    return {v.X(), v.Y(), v.Z()};
}

double norm(const Vec3& v) {
    return std::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
}

// Unit-length, or a fallback when the kernel handed back a degenerate axis (a
// zero-volume shape has no inertia frame). The fallback is the corresponding
// world axis, so the emitted basis is still an orthonormal right-handed frame
// rather than a set of NaNs.
Vec3 unit_or(const Vec3& v, const Vec3& fallback) {
    const double n = norm(v);
    if (!std::isfinite(n) || n < 1e-12) return fallback;
    return {v[0] / n, v[1] / n, v[2] / n};
}

Vec3 cross(const Vec3& a, const Vec3& b) {
    return {a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]};
}

// Flip `v` so its dominant component is positive.
//
// Determinism, not cosmetics: an inertia eigenvector is only defined up to sign,
// and OCCT is free to hand back either. Without a canonical sign the SAME shape
// could answer `[1,0,0]` in one process and `[-1,0,0]` in the next, which would
// make the response non-reproducible (Invariant 5). Ties on |component| break by
// the LOWEST index, so the rule has no ambiguity of its own.
Vec3 sign_canonical(const Vec3& v) {
    std::size_t dominant = 0;
    for (std::size_t i = 1; i < 3; ++i) {
        if (std::fabs(v[i]) > std::fabs(v[dominant])) dominant = i;
    }
    if (v[dominant] < 0.0) return {-v[0], -v[1], -v[2]};
    return v;
}

json vec3_json(const Vec3& v) {
    return json::array({v[0], v[1], v[2]});
}

std::string get_str(const json& o, const char* key) {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return {};
}

}  // namespace

Envelope handle_query_mass_properties(Session& session, const Envelope& req) {
    const std::string body_id = get_str(req.args, "bodyId");
    // A COPY of the head — the §7.5 rule. Nothing below can touch live state.
    const BodyStore bodies = session.bodies_copy();
    const BodyRecord* rec = body_id.empty() ? nullptr : bodies.get(body_id);
    if (rec == nullptr || rec->geom.IsNull()) {
        return Envelope::error_response(
            req.id, protocol::ErrorInfo{"REF_UNRESOLVED",
                                        "QueryMassProperties: unknown bodyId '" + body_id + "'",
                                        /*retriable=*/false});
    }

    GProp_GProps volume_props;
    BRepGProp::VolumeProperties(rec->geom, volume_props);
    GProp_GProps surface_props;
    BRepGProp::SurfaceProperties(rec->geom, surface_props);

    // The VOLUME centre of mass, deliberately — the surface properties have a
    // centroid of their own (the area-weighted one) and the two differ for any
    // shape that is not uniformly thick.
    const gp_Pnt centre = volume_props.CentreOfMass();

    // OCCT transfers the inertia matrix to the centre of mass inside
    // PrincipalProperties (Huygens), which is exactly the contract's frame.
    const GProp_PrincipalProps principal = volume_props.PrincipalProperties();
    Standard_Real i1 = 0.0;
    Standard_Real i2 = 0.0;
    Standard_Real i3 = 0.0;
    principal.Moments(i1, i2, i3);

    // Axes: unit, sign-canonical, right-handed. The third is REBUILT as a1 × a2
    // rather than sign-canonicalized on its own — canonicalizing all three
    // independently can land on a left-handed triple, and a moment is even in its
    // axis sign, so replacing a3 by ±a3 never invalidates the (i3, a3) pairing.
    const Vec3 a1 = sign_canonical(unit_or(to_vec3(principal.FirstAxisOfInertia()), {1, 0, 0}));
    const Vec3 a2 = sign_canonical(unit_or(to_vec3(principal.SecondAxisOfInertia()), {0, 1, 0}));
    const Vec3 a3 = unit_or(cross(a1, a2), {0, 0, 1});

    json result = {
        {"volume", volume_props.Mass()},
        {"surfaceArea", surface_props.Mass()},
        {"centroid", json::array({centre.X(), centre.Y(), centre.Z()})},
        {"principalMoments", json::array({i1, i2, i3})},
        {"principalAxes", json::array({vec3_json(a1), vec3_json(a2), vec3_json(a3)})},
    };
    return Envelope::ok_response(req.id, std::move(result));
}

}  // namespace onecad::session
