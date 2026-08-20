// test_gear_tool.cpp — the gear SOLID builder (SCHEMA §7.3 `Gear`, G1).
//
// Where test_gear_math_*.cpp pin the 2D mathematics against the reference,
// this file pins the OCCT construction on top of it: that the profile closes
// into a valid solid, that the solid's measurements agree with the closed-form
// gear geometry it came from, and that every refusal path refuses.
//
// The volume assertions are BOUNDS, not equalities, and deliberately so — a
// splined involute flank has no elementary closed-form area. The bounds are
// the two cylinders that must sandwich any real gear (root and addendum), which
// is loose but genuinely independent of the implementation: a gear that leaks
// outside them is wrong no matter how it was built.
//
// No test framework (matches the prototype style): exit code == failure count,
// clamped (see the note at the bottom of main).
#include <cmath>
#include <cstdio>
#include <string>

#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <Bnd_Box.hxx>
#include <BRepBndLib.hxx>
#include <GProp_GProps.hxx>
#include <TopAbs.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopExp.hxx>

#include "ops/gear/GearTool.h"

namespace gear = onecad::ops::gear;
namespace kg = onecad::kernel::gear;

namespace {
int g_failures = 0;
#define CHECK(cond)                                                              \
    do {                                                                         \
        if (!(cond)) {                                                           \
            std::fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond); \
            ++g_failures;                                                        \
        }                                                                        \
    } while (0)

constexpr double kPi = 3.14159265358979323846;

double deg(double d) { return d * kPi / 180.0; }

gear::GearBuildSpec spur(int teeth, double module, double height) {
    gear::GearBuildSpec s;
    s.involute.module = module;
    s.involute.numTeeth = teeth;
    s.involute.pressureAngle = deg(20.0);
    s.involute.clearance = 0.25;
    s.involute.shift = 0.0;
    s.sampleCount = 12;
    s.height = height;
    return s;
}

double volume_of(const TopoDS_Shape& s) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(s, props);
    return props.Mass();
}

std::size_t count(const TopoDS_Shape& s, TopAbs_ShapeEnum kind) {
    NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> map;
    TopExp::MapShapes(s, kind, map);
    return static_cast<std::size_t>(map.Extent());
}

const gp_Ax2 kOrigin(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1));

/// Every solid a gear build publishes must satisfy these, whatever the
/// parameters — asserted for every success case below.
void check_solid_invariants(const gear::GearBuildResult& r, const gear::GearBuildSpec& spec,
                            const char* name) {
    if (!r.ok) {
        std::fprintf(stderr, "FAIL %s: build refused unexpectedly: %s\n", name, r.error.c_str());
        ++g_failures;
        return;
    }
    if (r.shape.IsNull()) {
        std::fprintf(stderr, "FAIL %s: shape is null\n", name);
        ++g_failures;
        return;
    }
    if (!BRepCheck_Analyzer(r.shape).IsValid()) {
        std::fprintf(stderr, "FAIL %s: shape is not BRep-valid\n", name);
        ++g_failures;
    }
    if (count(r.shape, TopAbs_SOLID) != 1) {
        std::fprintf(stderr, "FAIL %s: expected exactly 1 solid, got %zu\n", name,
                     count(r.shape, TopAbs_SOLID));
        ++g_failures;
    }

    const double vol = volume_of(r.shape);
    if (!(vol > 0.0)) {
        std::fprintf(stderr, "FAIL %s: volume %.6g is not positive\n", name, vol);
        ++g_failures;
        return;
    }
    // A gear must contain its root cylinder and be contained by its addendum
    // cylinder. Bores only remove material, so they weaken the lower bound —
    // which is why the lower bound is skipped when a bore is present.
    const double rRoot = r.computed.rootDiameter * 0.5;
    const double rTip = r.computed.addendumDiameter * 0.5;
    const double vRoot = kPi * rRoot * rRoot * spec.height;
    const double vTip = kPi * rTip * rTip * spec.height;
    if (vol > vTip) {
        std::fprintf(stderr, "FAIL %s: volume %.6g exceeds the addendum cylinder %.6g\n", name,
                     vol, vTip);
        ++g_failures;
    }
    if (!spec.axleHole && !spec.offsetHole && vol < vRoot) {
        std::fprintf(stderr, "FAIL %s: volume %.6g is below the root cylinder %.6g\n", name, vol,
                     vRoot);
        ++g_failures;
    }

    // The body must live inside the addendum cylinder and span exactly the
    // requested height — this catches a mis-scaled or mis-placed build that
    // still happens to be a valid solid.
    Bnd_Box box;
    BRepBndLib::Add(r.shape, box);
    double xmin, ymin, zmin, xmax, ymax, zmax;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    const double tol = 1e-3 + 0.02 * rTip;  // bbox is a loose outer hull
    if (xmin < -rTip - tol || xmax > rTip + tol || ymin < -rTip - tol || ymax > rTip + tol) {
        std::fprintf(stderr, "FAIL %s: body escapes the addendum cylinder\n", name);
        ++g_failures;
    }
    if (std::abs(zmin) > 1e-6 || std::abs(zmax - spec.height) > 1e-6) {
        std::fprintf(stderr, "FAIL %s: z extent [%.9g,%.9g] != [0,%.9g]\n", name, zmin, zmax,
                     spec.height);
        ++g_failures;
    }
}

}  // namespace

int main() {
    // --- Baseline spur gears across tooth counts and modules. ---
    for (const int teeth : {8, 12, 15, 20, 40}) {
        for (const double module : {0.5, 1.0, 2.5}) {
            const auto spec = spur(teeth, module, 5.0);
            const auto r = gear::build_gear_solid(spec, kOrigin);
            const std::string name =
                "spur z=" + std::to_string(teeth) + " m=" + std::to_string(module);
            check_solid_invariants(r, spec, name.c_str());

            if (r.ok) {
                // Computed read-outs must agree with the closed-form sampler.
                const auto fr = kg::compute_involute_factors(spec.involute);
                CHECK(fr.ok);
                if (fr.ok) {
                    CHECK(std::abs(r.computed.pitchDiameter - fr.factors.referenceDiameter) < 1e-9);
                    CHECK(std::abs(r.computed.addendumDiameter - fr.factors.addendumDiameter) < 1e-9);
                    CHECK(std::abs(r.computed.rootDiameter - fr.factors.rootDiameter) < 1e-9);
                    CHECK(std::abs(r.computed.baseDiameter - fr.factors.baseDiameter) < 1e-9);
                }
                // Pitch diameter is the one number a gear user checks first.
                CHECK(std::abs(r.computed.pitchDiameter - module * teeth) < 1e-9);
            }
        }
    }

    // --- Determinism: the same spec must build a byte-identical volume, and
    //     the same topology. The plan's double-run discipline, at tool level. ---
    {
        const auto spec = spur(17, 2.0, 6.0);
        const auto a = gear::build_gear_solid(spec, kOrigin);
        const auto b = gear::build_gear_solid(spec, kOrigin);
        CHECK(a.ok && b.ok);
        if (a.ok && b.ok) {
            CHECK(volume_of(a.shape) == volume_of(b.shape));
            CHECK(count(a.shape, TopAbs_FACE) == count(b.shape, TopAbs_FACE));
            CHECK(count(a.shape, TopAbs_EDGE) == count(b.shape, TopAbs_EDGE));
        }
    }

    // --- Height scales the volume exactly linearly (a prism must). ---
    {
        auto s1 = spur(15, 1.0, 4.0);
        auto s2 = spur(15, 1.0, 8.0);
        const auto a = gear::build_gear_solid(s1, kOrigin);
        const auto b = gear::build_gear_solid(s2, kOrigin);
        CHECK(a.ok && b.ok);
        if (a.ok && b.ok) {
            const double ratio = volume_of(b.shape) / volume_of(a.shape);
            CHECK(std::abs(ratio - 2.0) < 1e-9);
        }
    }

    // --- Tooth count shows up in the topology: a z-tooth gear's profile has
    //     z times one tooth's edge count. Catches a replication that silently
    //     drops or doubles a tooth while still closing. ---
    {
        std::size_t prevEdges = 0;
        for (const int teeth : {10, 20}) {
            const auto spec = spur(teeth, 1.0, 5.0);
            const auto r = gear::build_gear_solid(spec, kOrigin);
            CHECK(r.ok);
            if (!r.ok) continue;
            const std::size_t edges = count(r.shape, TopAbs_EDGE);
            if (prevEdges != 0) CHECK(edges == prevEdges * 2);
            prevEdges = edges;
        }
    }

    // --- Bores ------------------------------------------------------------
    {
        auto spec = spur(24, 2.0, 6.0);
        const auto plain = gear::build_gear_solid(spec, kOrigin);
        spec.axleHole = true;
        spec.axleHoleDiameter = 10.0;
        const auto bored = gear::build_gear_solid(spec, kOrigin);
        check_solid_invariants(bored, spec, "spur + axle hole");
        CHECK(plain.ok && bored.ok);
        if (plain.ok && bored.ok) {
            // The bore removes exactly its own cylinder.
            const double expect = kPi * 5.0 * 5.0 * spec.height;
            const double removed = volume_of(plain.shape) - volume_of(bored.shape);
            CHECK(std::abs(removed - expect) < 1e-6 * expect);
        }
    }
    {
        auto spec = spur(30, 2.0, 6.0);
        spec.offsetHole = true;
        spec.offsetHoleDiameter = 6.0;
        spec.offsetHoleOffset = 12.0;
        const auto r = gear::build_gear_solid(spec, kOrigin);
        check_solid_invariants(r, spec, "spur + offset hole");
    }
    {
        // Both bores at once.
        auto spec = spur(30, 2.0, 6.0);
        spec.axleHole = true;
        spec.axleHoleDiameter = 8.0;
        spec.offsetHole = true;
        spec.offsetHoleDiameter = 5.0;
        spec.offsetHoleOffset = 14.0;
        const auto r = gear::build_gear_solid(spec, kOrigin);
        check_solid_invariants(r, spec, "spur + both bores");
    }

    // --- Placement: a gear built on an offset/rotated frame is the same solid
    //     moved, so its volume is invariant and its centre follows the frame. ---
    {
        const auto spec = spur(18, 1.5, 5.0);
        const auto at_origin = gear::build_gear_solid(spec, kOrigin);
        const gp_Ax2 moved(gp_Pnt(10.0, -4.0, 3.0), gp_Dir(0, 1, 0));
        const auto placed = gear::build_gear_solid(spec, moved);
        CHECK(at_origin.ok && placed.ok);
        if (at_origin.ok && placed.ok) {
            CHECK(std::abs(volume_of(at_origin.shape) - volume_of(placed.shape)) < 1e-9);
            CHECK(BRepCheck_Analyzer(placed.shape).IsValid());
            // The body must run along the frame's own axis (+Y here), not +Z.
            Bnd_Box box;
            BRepBndLib::Add(placed.shape, box);
            double xmin, ymin, zmin, xmax, ymax, zmax;
            box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
            CHECK(std::abs((ymax - ymin) - spec.height) < 1e-3);
        }
    }

    // --- Undercut recipe still builds (a different sampler branch: the tooth
    //     gains trochoid root segments). ---
    {
        auto spec = spur(9, 1.0, 4.0);
        spec.involute.undercut = true;
        const auto r = gear::build_gear_solid(spec, kOrigin);
        check_solid_invariants(r, spec, "spur undercut z=9");
    }

    // --- Profile shift and backlash both still produce valid solids. ---
    {
        auto spec = spur(12, 2.0, 5.0);
        spec.involute.shift = 0.5;
        check_solid_invariants(gear::build_gear_solid(spec, kOrigin), spec, "spur shift=0.5");
    }
    {
        auto spec = spur(20, 2.0, 5.0);
        spec.involute.backlash = 0.1;
        check_solid_invariants(gear::build_gear_solid(spec, kOrigin), spec, "spur backlash=0.1");
    }
    {
        auto spec = spur(20, 2.0, 5.0);
        spec.involute.head = 0.3;
        check_solid_invariants(gear::build_gear_solid(spec, kOrigin), spec, "spur head=0.3");
    }

    // --- Refusal matrix: every one of these must refuse with a message, and
    //     must NOT return a shape. ---
    auto refuses = [](const gear::GearBuildSpec& spec, const char* what) {
        const auto r = gear::build_gear_solid(spec, kOrigin);
        if (r.ok) {
            std::fprintf(stderr, "FAIL: expected refusal for %s, got a shape\n", what);
            ++g_failures;
            return;
        }
        if (r.error.empty()) {
            std::fprintf(stderr, "FAIL: refusal for %s carried no message\n", what);
            ++g_failures;
        }
        if (!r.shape.IsNull()) {
            std::fprintf(stderr, "FAIL: refusal for %s still returned a shape\n", what);
            ++g_failures;
        }
    };
    {
        auto s = spur(15, 1.0, 5.0);
        s.height = 0.0;
        refuses(s, "zero height");
    }
    {
        // --- The authoring-resolution boundary --------------------------------
        // The height floor is `GeometryPrecisionContext::authoring_resolution()`.
        // "zero height" above is refused by ANY plausible threshold, so it cannot
        // prove the guard is still wired to the context. These two STRADDLE the
        // boundary and flip the moment the context moves. The upper case asserts
        // the GUARD, not the build: OCCT is entitled to refuse a 1.05 µm prism for
        // its own reasons, but not with this message.
        const char* needle = "height must be greater";
        auto s = spur(15, 1.0, 5.0);
        s.height = 9.9e-4;
        const auto below = gear::build_gear_solid(s, kOrigin);
        if (below.ok || below.error.find(needle) == std::string::npos) {
            std::fprintf(stderr,
                         "FAIL: authoring boundary: height 9.9e-4 mm is refused by name, got: %s\n",
                         below.error.c_str());
            ++g_failures;
        }
        s.height = 1.05e-3;
        const auto above = gear::build_gear_solid(s, kOrigin);
        if (above.error.find(needle) != std::string::npos) {
            std::fprintf(stderr,
                         "FAIL: authoring boundary: height 1.05e-3 mm clears the guard, got: %s\n",
                         above.error.c_str());
            ++g_failures;
        }
    }
    {
        auto s = spur(15, 1.0, 5.0);
        s.sampleCount = 1;
        refuses(s, "sampleCount 1");
    }
    {
        auto s = spur(2, 1.0, 5.0);
        refuses(s, "2 teeth");
    }
    {
        auto s = spur(15, 0.0, 5.0);
        refuses(s, "zero module");
    }
    {
        auto s = spur(15, 1.0, 5.0);
        s.involute.pressureAngle = 0.0;
        refuses(s, "zero pressure angle");
    }
    {
        // A bore that swallows the tooth roots must refuse, not produce a ring
        // of disconnected teeth.
        auto s = spur(20, 1.0, 5.0);
        s.axleHole = true;
        s.axleHoleDiameter = 100.0;
        refuses(s, "axle hole larger than the root circle");
    }
    {
        auto s = spur(20, 1.0, 5.0);
        s.offsetHole = true;
        s.offsetHoleDiameter = 4.0;
        s.offsetHoleOffset = 40.0;
        refuses(s, "offset hole outside the root circle");
    }

    if (g_failures == 0) std::fprintf(stderr, "OK: gear_tool\n");
    // Exit status is 8-bit on POSIX; clamp so a failing run can never report
    // success by landing on a multiple of 256 (see test_gear_math_*.cpp).
    return (g_failures == 0) ? 0 : (g_failures > 100 ? 100 : g_failures);
}
