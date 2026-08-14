// test_gear_math_worm.cpp — ZA worm thread cross-section math.
//
// Oracle: upstream's own trapezoid-station algebra and `helical_projection`,
// evaluated in Python over three parameter sets (see the Gear Generator plan
// §10.1 leg 2). Every flank sample is pinned, not just the endpoints, because
// the flanks are straight in the AXIAL plane but curved once wrapped — an
// error in the wrap would leave endpoints correct and bow the interior.
//
// The headline structural assertion is the exact angular-span identity
// documented in WormMath.h: phi_4 - phi_0 == 2*pi/t identically, whatever the
// module, pressure angle, head or clearance. It is checked to 1e-15 over a
// sweep, which is what proves the t-fold replication closes.
//
// No test framework (matches the prototype style): exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "kernel/gear/WormMath.h"

namespace gear = onecad::kernel::gear;

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
constexpr double kTol = 1e-9;

void CHECK_NEAR(double a, double e, const char* what, const char* caseName) {
    if (std::abs(a - e) > kTol) {
        std::fprintf(stderr, "FAIL %s: %s: actual=%.17g expected=%.17g (case %s)\n", __FILE__, what,
                     a, e, caseName);
        ++g_failures;
    }
}

void CHECK_POINT(const gear::Point2d& a, const gear::Point2d& e, const char* what,
                 const char* caseName) {
    if (std::abs(a.x - e.x) > kTol || std::abs(a.y - e.y) > kTol) {
        std::fprintf(stderr, "FAIL %s: %s: actual=(%.17g,%.17g) expected=(%.17g,%.17g) (case %s)\n",
                     __FILE__, what, a.x, a.y, e.x, e.y, caseName);
        ++g_failures;
    }
}

double deg(double d) { return d * kPi / 180.0; }

struct Oracle {
    double module, diameter;
    int numThreads;
    double pressureAngleDeg, clearance, head;
    int sampleCount;
    const char* name;
    double leadAngle, rootRadius, tipRadius;
    double z[5];
    double phi[5];
    std::vector<gear::Point2d> rising;
    std::vector<gear::Point2d> falling;
};

const Oracle kCases[] = {
    {1, 5, 3, 20, 0.25, 0, 10, "default_m1_d5_t3",
     0.54041950027058416, 1.25, 3.5,
     {0.090992558566550585, 0.75186329969594135, 1.5707963267948966, 2.4136521850573884,
      3.2325852121563434},
     {0.060661705711033721, 0.50124219979729423, 1.0471975511965976, 1.6091014567049255,
      2.1550568081042289},
     {
         {1.0962329283191574, 0.60064412664139566},
         {1.2693633715615988, 0.79919749182399857},
         {1.4216738713656611, 1.0204623478973507},
         {1.5510787169434797, 1.2625984372891363},
         {1.6556414847949854, 1.5235981339663867},
         {1.7335875345775078, 1.8012979375876379},
         {1.7833156508536749, 2.0933908592091286},
         {1.8034087690803817, 2.3974396366966122},
         {1.792643728592288, 2.7108907138316622},
         {1.7500000000000004, 3.0310889132455352},
     },
     {
         {-0.13403517115237507, 3.4974325687415551},
         {-0.32111742223189316, 3.2340970302604006},
         {-0.47685444449877779, 2.961859186180491},
         {-0.60091037939284564, 2.6835436862361579},
         {-0.69317600644665622, 2.401979813421975},
         {-0.75376786802596474, 2.1199844341720038},
         {-0.78302616325253949, 1.8403450838530275},
         {-0.78151142502752613, 1.5658032738985588},
         {-0.74999999999999978, 1.299038105676658},
         {-0.68947835715087979, 1.0426502745506396},
     }},
    {2, 12, 1, 20, 0.25, 0.10000000000000001, 10, "m2_d12_t1_head0.1",
     0.16514867741462683, 3.5, 8.1999999999999993,
     {0.18198511713310117, 1.5037265993918827, 3.2143867004430335, 4.7545103232615364,
      6.4651704243126877},
     {0.18198511713310117, 1.5037265993918827, 3.2143867004430335, 4.7545103232615364,
      6.4651704243126877},
     {
         {0.23456809207565724, 3.4921308409307898},
         {-0.49350124563587733, 3.9918326775422472},
         {-1.3996319999818916, 4.3235408605989454},
         {-2.4430848983115401, 4.4387438865914755},
         {-3.5714123275068084, 4.298917654396055},
         {-4.7228828194524146, 3.8781512201133785},
         {-5.82946421629235, 3.1651947905426163},
         {-6.8202307023053956, 2.1648160374435568},
         {-7.6250365289500337, 0.89838162109199671},
         {-8.1782838016492931, -0.59638415445144022},
     },
     {
         {0.34529288691192839, -8.1927268245833762},
         {1.7667631281280953, -7.4717347151799558},
         {2.9325636049777071, -6.5270242845727271},
         {3.8127404519898627, -5.4280863439034608},
         {4.3941110293936223, -4.2470539522952837},
         {4.6800716516906586, -3.0549318073219154},
         {4.6895986605448448, -1.9180134290789288},
         {4.4555103968472913, -0.89465211799205324},
         {4.0220906572111312, -0.032532293389825787},
         {3.4422022594299153, 0.63343792527412501},
     }},
    {1.5, 20, 5, 25, 0.20000000000000001, 0, 10, "m1.5_d20_t5_a25",
     0.35877067027057225, 8.1999999999999993, 11.5,
     {0.13989229744649961, 0.81737921828084925, 2.3561944901923448, 3.3134660059196936,
      4.8522812778311888},
     {0.037304612652399899, 0.21796779154155979, 0.62831853071795862, 0.8835909349119182,
      1.293941674088317},
     {
         {8.0059791599617416, 1.7732167634720426},
         {8.2708429237397763, 2.2318008666996816},
         {8.5098079588220372, 2.7180163627935645},
         {8.7209157913439128, 3.2304222263178466},
         {8.9022869860957901, 3.7674568161604691},
         {9.0521280473172556, 4.3274421535995211},
         {9.1687380907080929, 4.9085885775850606},
         {9.2505152666686659, 5.5089997651335061},
         {9.2959629154164247, 6.1266781036964622},
         {9.3036954353118961, 6.759530401363441},
     },
     {
         {7.2953627438482584, 8.8897515395915772},
         {6.6631523074972137, 8.9192775737839707},
         {6.0438698865589799, 8.9102608214045347},
         {5.4396865913341701, 8.8639612921119664},
         {4.8526846545183471, 8.7817554863239007},
         {4.2848513855293371, 8.6651308731242906},
         {3.7380734452044537, 8.5156800619948889},
         {3.2141314560822138, 8.3350946861728747},
         {2.7146949625228891, 8.1251590161811986},
         {2.2413177539261659, 7.8877433227720681},
     }},
};

gear::WormParams params_of(const Oracle& o) {
    gear::WormParams p;
    p.module = o.module;
    p.diameter = o.diameter;
    p.numThreads = o.numThreads;
    p.pressureAngle = deg(o.pressureAngleDeg);
    p.clearance = o.clearance;
    p.head = o.head;
    p.sampleCount = o.sampleCount;
    return p;
}

void check_structure(const std::vector<gear::ProfileSegment>& segs, double angularPitch,
                     const std::string& caseName) {
    for (std::size_t i = 0; i + 1 < segs.size(); ++i) {
        if (gear::distance(segs[i].end, segs[i + 1].start) > 1e-9) {
            std::fprintf(stderr, "FAIL %s: chain discontinuity %zu->%zu (case %s)\n", __FILE__, i,
                         i + 1, caseName.c_str());
            ++g_failures;
        }
    }
    const gear::Point2d wrapped = gear::rotate(segs.back().end, -angularPitch);
    if (gear::distance(wrapped, segs.front().start) > 1e-9) {
        std::fprintf(stderr, "FAIL %s: tooth wrap broken (case %s)\n", __FILE__, caseName.c_str());
        ++g_failures;
    }
}

}  // namespace

int main() {
    for (const auto& o : kCases) {
        const gear::WormParams p = params_of(o);
        const auto fr = gear::compute_worm_factors(p);
        CHECK(fr.ok);
        if (!fr.ok) {
            std::fprintf(stderr, "  (case %s error: %s)\n", o.name, fr.error.c_str());
            continue;
        }
        const auto& f = fr.factors;
        CHECK_NEAR(f.leadAngle, o.leadAngle, "leadAngle", o.name);
        CHECK_NEAR(f.rootRadius, o.rootRadius, "rootRadius", o.name);
        CHECK_NEAR(f.tipRadius, o.tipRadius, "tipRadius", o.name);
        CHECK_NEAR(f.z0, o.z[0], "z0", o.name);
        CHECK_NEAR(f.z1, o.z[1], "z1", o.name);
        CHECK_NEAR(f.z2, o.z[2], "z2", o.name);
        CHECK_NEAR(f.z3, o.z[3], "z3", o.name);
        CHECK_NEAR(f.z4, o.z[4], "z4", o.name);
        CHECK_NEAR(f.phi0, o.phi[0], "phi0", o.name);
        CHECK_NEAR(f.phi1, o.phi[1], "phi1", o.name);
        CHECK_NEAR(f.phi2, o.phi[2], "phi2", o.name);
        CHECK_NEAR(f.phi3, o.phi[3], "phi3", o.name);
        CHECK_NEAR(f.phi4, o.phi[4], "phi4", o.name);

        // The exact identity: the tooth spans one angular pitch.
        CHECK(std::abs((f.phi4 - f.phi0) - f.angularPitch) < 1e-15);
        CHECK_NEAR(f.angularPitch, 2.0 * kPi / static_cast<double>(o.numThreads), "angularPitch",
                   o.name);

        const auto tr = gear::one_tooth_segments(p, f);
        CHECK(tr.ok);
        if (!tr.ok) continue;
        CHECK(tr.segments.size() == 4);
        if (tr.segments.size() != 4) continue;
        CHECK(tr.segments[0].kind == gear::SegmentKind::Arc);
        CHECK(tr.segments[1].kind == gear::SegmentKind::Interpolated);
        CHECK(tr.segments[2].kind == gear::SegmentKind::Arc);
        CHECK(tr.segments[3].kind == gear::SegmentKind::Interpolated);

        const auto& rise = tr.segments[1].points;
        CHECK(rise.size() == o.rising.size());
        if (rise.size() == o.rising.size()) {
            for (std::size_t i = 0; i < rise.size(); ++i) {
                char what[48];
                std::snprintf(what, sizeof(what), "rising[%zu]", i);
                CHECK_POINT(rise[i], o.rising[i], what, o.name);
            }
        }
        const auto& fall = tr.segments[3].points;
        CHECK(fall.size() == o.falling.size());
        if (fall.size() == o.falling.size()) {
            for (std::size_t i = 0; i < fall.size(); ++i) {
                char what[48];
                std::snprintf(what, sizeof(what), "falling[%zu]", i);
                CHECK_POINT(fall[i], o.falling[i], what, o.name);
            }
        }

        check_structure(tr.segments, f.angularPitch, o.name);

        // Arc radii: the root and tip arcs must sit exactly on their circles.
        CHECK(std::abs(gear::norm(tr.segments[0].start) - f.rootRadius) < 1e-12);
        CHECK(std::abs(gear::norm(tr.segments[0].mid) - f.rootRadius) < 1e-12);
        CHECK(std::abs(gear::norm(tr.segments[0].end) - f.rootRadius) < 1e-12);
        CHECK(std::abs(gear::norm(tr.segments[2].start) - f.tipRadius) < 1e-12);
        CHECK(std::abs(gear::norm(tr.segments[2].mid) - f.tipRadius) < 1e-12);
        CHECK(std::abs(gear::norm(tr.segments[2].end) - f.tipRadius) < 1e-12);
        // ...and the interior point must be at the ANGULAR MIDPOINT, not just
        // somewhere on the circle. Radius alone leaves `mid` free to sit on
        // top of an endpoint, which would hand GC_MakeArcOfCircle a
        // degenerate three-point arc.
        CHECK_POINT(tr.segments[0].mid,
                    gear::Point2d{f.rootRadius * std::cos(0.5 * (f.phi0 + f.phi1)),
                                  f.rootRadius * std::sin(0.5 * (f.phi0 + f.phi1))},
                    "root arc mid at angular midpoint", o.name);
        CHECK_POINT(tr.segments[2].mid,
                    gear::Point2d{f.tipRadius * std::cos(0.5 * (f.phi2 + f.phi3)),
                                  f.tipRadius * std::sin(0.5 * (f.phi2 + f.phi3))},
                    "tip arc mid at angular midpoint", o.name);
        // Flank endpoints bridge root -> tip and back.
        CHECK(std::abs(gear::norm(rise.front()) - f.rootRadius) < 1e-12);
        CHECK(std::abs(gear::norm(rise.back()) - f.tipRadius) < 1e-12);
        CHECK(std::abs(gear::norm(fall.front()) - f.tipRadius) < 1e-12);
        CHECK(std::abs(gear::norm(fall.back()) - f.rootRadius) < 1e-12);
    }

    // --- The exact angular-span identity, over a sweep with no pinned
    //     oracle. This is the property the whole replication rests on, and it
    //     must hold independently of head, clearance and pressure angle. ---
    for (const double m : {0.5, 1.0, 4.0}) {
        for (const int t : {1, 2, 3, 5, 10}) {
            for (const double aDeg : {10.0, 20.0, 30.0}) {
                for (const double head : {0.0, 0.15}) {
                    for (const double clr : {0.0, 0.25, 0.4}) {
                        gear::WormParams p;
                        p.module = m;
                        p.numThreads = t;
                        p.diameter = m * t / std::tan(deg(15.0));  // keep a sane lead angle
                        p.pressureAngle = deg(aDeg);
                        p.head = head;
                        p.clearance = clr;
                        p.sampleCount = 6;
                        const auto fr = gear::compute_worm_factors(p);
                        if (!fr.ok) continue;
                        const auto& f = fr.factors;
                        const std::string caseName =
                            "sweep m=" + std::to_string(m) + " t=" + std::to_string(t) +
                            " a=" + std::to_string(aDeg) + " head=" + std::to_string(head) +
                            " clr=" + std::to_string(clr);
                        if (std::abs((f.phi4 - f.phi0) - 2.0 * kPi / t) > 1e-15) {
                            std::fprintf(stderr,
                                         "FAIL %s: angular span identity broken: %.17g vs %.17g "
                                         "(case %s)\n",
                                         __FILE__, f.phi4 - f.phi0, 2.0 * kPi / t, caseName.c_str());
                            ++g_failures;
                        }
                        const auto tr = gear::one_tooth_segments(p, f);
                        CHECK(tr.ok);
                        if (!tr.ok) continue;
                        check_structure(tr.segments, f.angularPitch, caseName);
                        for (const auto& s : tr.segments) {
                            CHECK(std::isfinite(s.start.x) && std::isfinite(s.start.y));
                            CHECK(std::isfinite(s.end.x) && std::isfinite(s.end.y));
                        }
                    }
                }
            }
        }
    }

    // --- Handedness: reversePitch must flip the sweep twist and nothing else. ---
    {
        gear::WormParams a;
        gear::WormParams b = a;
        b.reversePitch = true;
        const auto fa = gear::compute_worm_factors(a);
        const auto fb = gear::compute_worm_factors(b);
        CHECK(fa.ok && fb.ok);
        if (fa.ok && fb.ok) {
            CHECK_NEAR(fa.factors.helixTwistPerUnit, -fb.factors.helixTwistPerUnit,
                       "twist sign flip", "reversePitch");
            // The cross-section itself is unchanged by handedness.
            CHECK_NEAR(fa.factors.rootRadius, fb.factors.rootRadius, "rootRadius", "reversePitch");
            CHECK_NEAR(fa.factors.phi4, fb.factors.phi4, "phi4", "reversePitch");
            CHECK(fa.factors.helixTwistPerUnit != 0.0);
        }
    }

    // --- worm_projection agrees with the sampled flanks (the wrap itself). ---
    {
        gear::WormParams p;
        const auto fr = gear::compute_worm_factors(p);
        CHECK(fr.ok);
        if (fr.ok) {
            const auto tr = gear::one_tooth_segments(p, fr.factors);
            CHECK(tr.ok);
            if (tr.ok) {
                const auto& rise = tr.segments[1].points;
                for (int i = 0; i < p.sampleCount; ++i) {
                    const double u =
                        static_cast<double>(i) / static_cast<double>(p.sampleCount - 1);
                    const double r =
                        fr.factors.rootRadius + (fr.factors.tipRadius - fr.factors.rootRadius) * u;
                    const double z = fr.factors.z1 + (fr.factors.z2 - fr.factors.z1) * u;
                    const gear::Point2d direct =
                        gear::worm_projection(r, z, p.module, p.numThreads);
                    CHECK(gear::distance(rise[static_cast<std::size_t>(i)], direct) < 1e-12);
                }
            }
        }
    }

    // --- Refusal matrix. ---
    {
        gear::WormParams p;
        p.numThreads = 0;
        CHECK(!gear::compute_worm_factors(p).ok);
    }
    {
        gear::WormParams p;
        p.module = 0.0;
        CHECK(!gear::compute_worm_factors(p).ok);
    }
    {
        gear::WormParams p;
        p.diameter = 0.0;
        CHECK(!gear::compute_worm_factors(p).ok);
    }
    {
        gear::WormParams p;
        p.pressureAngle = 0.0;
        CHECK(!gear::compute_worm_factors(p).ok);
    }
    {
        gear::WormParams p;
        p.sampleCount = 1;
        CHECK(!gear::compute_worm_factors(p).ok);
    }
    {
        // Diameter too small for the module/clearance: root radius <= 0.
        gear::WormParams p;
        p.module = 4.0;
        p.diameter = 5.0;
        CHECK(!gear::compute_worm_factors(p).ok);
    }
    {
        // Pressure angle steep enough to invert the trapezoid — upstream
        // would emit a self-crossing thread here instead of refusing.
        gear::WormParams p;
        p.pressureAngle = deg(60.0);
        CHECK(!gear::compute_worm_factors(p).ok);
    }
    {
        // Negative control: defaults succeed and produce a thread.
        gear::WormParams p;
        const auto fr = gear::compute_worm_factors(p);
        CHECK(fr.ok);
        if (fr.ok) CHECK(gear::one_tooth_segments(p, fr.factors).ok);
    }

    if (g_failures == 0) std::fprintf(stderr, "OK: gear_math_worm\n");
    // Exit status is 8-bit on POSIX, so returning a raw failure count reports
    // SUCCESS whenever that count is a multiple of 256 — this file's sweeps
    // can produce thousands. Clamp to a nonzero cap so a failing run can never
    // masquerade as a passing one, while small counts stay readable.
    return (g_failures == 0) ? 0 : (g_failures > 100 ? 100 : g_failures);
}
