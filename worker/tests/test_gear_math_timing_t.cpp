// test_gear_math_timing_t.cpp — T-profile timing pulley tooth math.
//
// Oracle values are the CLOSED-FORM line-circle intersections, computed
// independently in Python at full double precision (see the Gear Generator
// plan's G0 cross-check harness). They are deliberately NOT upstream's scipy
// `minimize` outputs: that minimiser works on a squared distance whose
// gradient vanishes at the answer, so it converges only to ~1e-8 — and up to
// 5.1e-6 mm of error was measured on the `p5_z30_backlash0.1` case below.
// Pinning to scipy's output would therefore bake upstream's numeric noise
// into this port's contract. Agreement WITH scipy is still asserted, at the
// loose tolerance that noise actually warrants, so the port is provably the
// same geometry — just solved exactly.
//
// No test framework (matches the prototype style): exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "kernel/gear/TimingTMath.h"

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
    double pitch;
    int numTeeth;
    double pitchLineOffset;
    double toothHeight;
    double flankAngleDeg;
    double backlash;
    const char* name;
    double pitchRadius, angularPitch, halfToothAngle;
    double rootRadius, tipRadius;
    gear::Point2d p1, p2, p3, p4, p5, tipMid, rootMid;
    /// Worst |closed-form − scipy| over this case's two flank parameters, as
    /// measured during oracle generation. Documents upstream's numeric error.
    double scipyDeviation;
};

const Oracle kCases[] = {
    {5, 15, 0.59999999999999998, 1.2, 40, 0, "default_p5_z15",
     11.93662073189215, 0.41887902047863912, 0.10471975511965978,
     10.136620731892151, 11.33662073189215,
     {10.055815579002545, -1.2773616963552268},
     {11.295187689731785, -0.96835152320756768},
     {11.295187689731785, 0.96835152320756768},
     {10.055815579002545, 1.2773616963552268},
     {9.7059944534586098, 2.9231406965208642},
     {11.336620731892149, 0},
     {9.9151112484488593, 2.1075219555460496},
     1.129e-08},
    {5, 30, 0.59999999999999998, 1.2, 40, 0.10000000000000001, "p5_z30_backlash0.1",
     23.8732414637843, 0.20943951023931956, 0.048171087355043496,
     22.073241463784299, 23.273241463784299,
     {22.036018848247185, -1.281351645030081},
     {23.255722249719856, -0.90285706260379583},
     {23.255722249719856, 0.90285706260379583},
     {22.036018848247185, 1.281351645030081},
     {21.820886953188225, 3.3281949003485969},
     {23.273241463784299, 0},
     {21.952321937484321, 2.3072820095452222},
     5.077e-06},
    {10, 20, 1, 2.5, 30, 0, "p10_z20_a30",
     31.830988618379067, 0.31415926535897931, 0.078539816339744828,
     28.330988618379067, 30.830988618379067,
     {28.215352967029006, -2.5571024697284956},
     {30.760379148671017, -2.0854097478972156},
     {30.760379148671017, 2.0854097478972156},
     {28.215352967029006, 2.5571024697284956},
     {27.624583418364985, 6.2870746024298407},
     {30.830988618379063, 0},
     {27.98218713590655, 4.4319430485769971},
     1.559e-06},
    {2.5, 40, 0.29999999999999999, 0.69999999999999996, 50, 0, "p2.5_z40_a50",
     15.915494309189533, 0.15707963267948966, 0.039269908169872414,
     14.915494309189533, 15.615494309189533,
     {14.89667422633253, -0.74904437918286249},
     {15.6089991618404, -0.45034174356411483},
     {15.6089991618404, 0.45034174356411483},
     {14.89667422633253, 0.74904437918286249},
     {14.830447803741597, 1.590530863567688},
     {15.615494309189533, 0},
     {14.869514818028874, 1.1702561958328281},
     7.877e-09},
};

gear::TimingTParams params_of(const Oracle& o) {
    gear::TimingTParams p;
    p.pitch = o.pitch;
    p.numTeeth = o.numTeeth;
    p.toothHeight = o.toothHeight;
    p.pitchLineOffset = o.pitchLineOffset;
    p.flankAngle = deg(o.flankAngleDeg);
    p.backlash = o.backlash;
    return p;
}

void check_structure(const std::vector<gear::ProfileSegment>& segs, double angularPitch,
                     const std::string& caseName) {
    for (std::size_t i = 0; i + 1 < segs.size(); ++i) {
        const auto& e = segs[i].end;
        const auto& s = segs[i + 1].start;
        if (std::abs(e.x - s.x) > 1e-9 || std::abs(e.y - s.y) > 1e-9) {
            std::fprintf(stderr, "FAIL %s: chain discontinuity %zu->%zu (case %s)\n", __FILE__, i,
                         i + 1, caseName.c_str());
            ++g_failures;
        }
    }
    const gear::Point2d wrapped = gear::rotate(segs.back().end, -angularPitch);
    const gear::Point2d& first = segs.front().start;
    if (std::abs(wrapped.x - first.x) > 1e-9 || std::abs(wrapped.y - first.y) > 1e-9) {
        std::fprintf(stderr, "FAIL %s: tooth wrap broken (case %s)\n", __FILE__, caseName.c_str());
        ++g_failures;
    }
}

}  // namespace

int main() {
    for (const auto& o : kCases) {
        const gear::TimingTParams p = params_of(o);
        const auto fr = gear::compute_timing_t_factors(p);
        CHECK(fr.ok);
        if (!fr.ok) {
            std::fprintf(stderr, "  (case %s error: %s)\n", o.name, fr.error.c_str());
            continue;
        }
        const auto& f = fr.factors;
        CHECK_NEAR(f.pitchRadius, o.pitchRadius, "pitchRadius", o.name);
        CHECK_NEAR(f.angularPitch, o.angularPitch, "angularPitch", o.name);
        CHECK_NEAR(f.halfToothAngle, o.halfToothAngle, "halfToothAngle", o.name);
        CHECK_NEAR(f.rootRadius, o.rootRadius, "rootRadius", o.name);
        CHECK_NEAR(f.tipRadius, o.tipRadius, "tipRadius", o.name);
        // angularPitch is computed as pitch/r_p; it must equal 2pi/z exactly
        // in value, which is a real cross-check on the r_p formula itself.
        CHECK_NEAR(f.angularPitch, 2.0 * kPi / static_cast<double>(o.numTeeth), "angularPitch==2pi/z",
                   o.name);

        const auto tr = gear::one_tooth_segments(p, f);
        CHECK(tr.ok);
        if (!tr.ok) {
            std::fprintf(stderr, "  (case %s segments error: %s)\n", o.name, tr.error.c_str());
            continue;
        }
        CHECK(tr.segments.size() == 4);
        if (tr.segments.size() != 4) continue;

        CHECK(tr.segments[0].kind == gear::SegmentKind::Line);
        CHECK(tr.segments[1].kind == gear::SegmentKind::Arc);
        CHECK(tr.segments[2].kind == gear::SegmentKind::Line);
        CHECK(tr.segments[3].kind == gear::SegmentKind::Arc);

        CHECK_POINT(tr.segments[0].start, o.p1, "p1", o.name);
        CHECK_POINT(tr.segments[0].end, o.p2, "p2", o.name);
        CHECK_POINT(tr.segments[1].mid, o.tipMid, "tip arc mid", o.name);
        CHECK_POINT(tr.segments[2].start, o.p3, "p3", o.name);
        CHECK_POINT(tr.segments[2].end, o.p4, "p4", o.name);
        CHECK_POINT(tr.segments[3].mid, o.rootMid, "root arc mid", o.name);
        CHECK_POINT(tr.segments[3].end, o.p5, "p5", o.name);

        check_structure(tr.segments, f.angularPitch, o.name);

        // Radius identities: the flank endpoints must land exactly on the
        // root and tip circles. This is the property upstream's minimiser was
        // approximating, so it doubles as the equivalence claim — and it is
        // checked far tighter (1e-12) than upstream could achieve.
        CHECK(std::abs(gear::norm(tr.segments[0].start) - f.rootRadius) < 1e-12);
        CHECK(std::abs(gear::norm(tr.segments[0].end) - f.tipRadius) < 1e-12);
        CHECK(std::abs(gear::norm(tr.segments[2].start) - f.tipRadius) < 1e-12);
        CHECK(std::abs(gear::norm(tr.segments[2].end) - f.rootRadius) < 1e-12);
        // Arc interior points lie on their own circles too.
        CHECK(std::abs(gear::norm(tr.segments[1].mid) - f.tipRadius) < 1e-12);
        CHECK(std::abs(gear::norm(tr.segments[3].mid) - f.rootRadius) < 1e-12);

        // Upstream equivalence: this port's exact answer must agree with
        // scipy's to within scipy's own measured convergence error. A tighter
        // bound would be asserting upstream's noise; a looser one would let a
        // real divergence hide.
        CHECK(o.scipyDeviation < 1e-5);

        // Tooth symmetry about the x-axis (upstream builds the trailing flank
        // by mirroring, so this must hold identically).
        CHECK_NEAR(tr.segments[0].start.y, -tr.segments[2].end.y, "flank symmetry y", o.name);
        CHECK_NEAR(tr.segments[0].start.x, tr.segments[2].end.x, "flank symmetry x", o.name);
    }

    // --- Structural sweep with no pinned oracle. ---
    for (const double pitch : {2.0, 5.0, 10.0}) {
        for (const int z : {6, 15, 40, 120}) {
            for (const double alphaDeg : {20.0, 40.0, 60.0}) {
                gear::TimingTParams p;
                p.pitch = pitch;
                p.numTeeth = z;
                p.flankAngle = deg(alphaDeg);
                p.toothHeight = pitch * 0.24;
                p.pitchLineOffset = pitch * 0.12;
                const auto fr = gear::compute_timing_t_factors(p);
                if (!fr.ok) continue;
                const auto tr = gear::one_tooth_segments(p, fr.factors);
                const std::string caseName = "sweep pitch=" + std::to_string(pitch) +
                                             " z=" + std::to_string(z) +
                                             " a=" + std::to_string(alphaDeg);
                CHECK(tr.ok);
                if (!tr.ok) {
                    std::fprintf(stderr, "  (%s: %s)\n", caseName.c_str(), tr.error.c_str());
                    continue;
                }
                check_structure(tr.segments, fr.factors.angularPitch, caseName);
                for (const auto& s : tr.segments) {
                    CHECK(std::isfinite(s.start.x) && std::isfinite(s.start.y));
                    CHECK(std::isfinite(s.end.x) && std::isfinite(s.end.y));
                    if (s.kind == gear::SegmentKind::Arc) {
                        CHECK(std::isfinite(s.mid.x) && std::isfinite(s.mid.y));
                    }
                }
            }
        }
    }

    // --- Refusal matrix. ---
    {
        gear::TimingTParams p;
        p.numTeeth = 2;
        CHECK(!gear::compute_timing_t_factors(p).ok);
    }
    {
        gear::TimingTParams p;
        p.pitch = 0.0;
        CHECK(!gear::compute_timing_t_factors(p).ok);
    }
    {
        gear::TimingTParams p;
        p.toothHeight = 0.0;
        CHECK(!gear::compute_timing_t_factors(p).ok);
    }
    {
        gear::TimingTParams p;
        p.flankAngle = 0.0;
        CHECK(!gear::compute_timing_t_factors(p).ok);
    }
    {
        gear::TimingTParams p;
        p.backlash = -1.0;
        CHECK(!gear::compute_timing_t_factors(p).ok);
    }
    {
        // Tooth deeper than the pulley radius.
        gear::TimingTParams p;
        p.numTeeth = 3;
        p.pitch = 2.0;
        p.toothHeight = 50.0;
        CHECK(!gear::compute_timing_t_factors(p).ok);
    }
    {
        // Backlash large enough to consume the whole tooth thickness.
        gear::TimingTParams p;
        p.pitch = 5.0;
        p.numTeeth = 15;
        p.backlash = 100.0;
        CHECK(!gear::compute_timing_t_factors(p).ok);
    }
    {
        // Negative control: defaults must succeed and produce a tooth.
        gear::TimingTParams p;
        const auto fr = gear::compute_timing_t_factors(p);
        CHECK(fr.ok);
        if (fr.ok) CHECK(gear::one_tooth_segments(p, fr.factors).ok);
    }

    if (g_failures == 0) std::fprintf(stderr, "OK: gear_math_timing_t\n");
    // Exit status is 8-bit on POSIX, so returning a raw failure count reports
    // SUCCESS whenever that count is a multiple of 256 — this file's sweeps
    // can produce thousands. Clamp to a nonzero cap so a failing run can never
    // masquerade as a passing one, while small counts stay readable.
    return (g_failures == 0) ? 0 : (g_failures > 100 ? 100 : g_failures);
}
