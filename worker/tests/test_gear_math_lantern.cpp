// test_gear_math_lantern.cpp — lantern (pin/roller) gear tooth math.
//
// Oracle: the offset-involute flank sampled from the closed-form parametric
// equations, with phi_min from an INDEPENDENT bracketed bisection written in
// Python — cross-checked against upstream's `scipy.optimize.root` answer,
// which agreed to ~1e-13 on every case. See the Gear Generator plan §10.1.
//
// The most important thing this file pins is not a coordinate: it is that the
// phi_min solve finds the MEANINGFUL root and never the spurious phi = 0 one
// that upstream's objective also satisfies identically (see LanternMath.h).
// `phi_min_objective_zero_at_origin_but_solver_avoids_it` below states that
// directly.
//
// No test framework (matches the prototype style): exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "kernel/gear/LanternMath.h"

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

/// Upstream's objective, restated here so the test is not merely re-running
/// the implementation's own copy of it.
double objective(double phi, double r0, double rr) {
    return phi * phi * r0 - 2.0 * phi * r0 * std::sin(phi) - 2.0 * phi * rr -
           2.0 * r0 * std::cos(phi) + 2.0 * r0 + 2.0 * rr * std::sin(phi);
}

struct Oracle {
    double module;
    int numTeeth;
    double rollerRadius;
    double head;
    int sampleCount;
    const char* name;
    double pitchRadius, phiMin, phiMax;
    gear::Point2d rootMid, p3, tipMid;
    std::vector<gear::Point2d> flank;
};

const Oracle kCases[] = {
    {1, 15, 1, 0, 10, "default_m1_z15_rr1",
     7.5, 0.1778090231025557, 0.66666666666666663,
     {6.5, 0},
     {8.2519695776779916, 2.038381242328561},
     {8.3142546062373484, 1.7672493719509541},
     {
         {7.4407511595100146, 0.99824324435510026},
         {7.4692989395353342, 1.0042802577167034},
         {7.5188625163629093, 1.0175309314233549},
         {7.5886797449502623, 1.0402782694037065},
         {7.6778035790511492, 1.0747403985228452},
         {7.7851070507107121, 1.1230572851960869},
         {7.9092891432011445, 1.1872778111724305},
         {8.0488815325566545, 1.269347266113094},
         {8.2022561692567422, 1.3710953129242232},
         {8.367633668106059, 1.4942244799152358},
     }},
    {2, 12, 1.5, 0.20000000000000001, 8, "m2_z12_rr1.5_head0.2",
     12, 0.1666924089674352, 0.7095819969098528,
     {10.5, 0},
     {13.090219930885603, 4.6750553110145461},
     {13.426368985418048, 3.5975847269250378},
     {
         {11.91667954179305, 1.4976841126366303},
         {11.989881747913852, 1.5134167287682896},
         {12.13093453341105, 1.5549253168112624},
         {12.335867282705014, 1.6326971547709259},
         {12.599502862117776, 1.7567273021702274},
         {12.915512461547531, 1.9363956112741334},
         {13.276482161921825, 2.1803494635084002},
         {13.673990656779583, 2.4963933020068438},
     }},
    {0.5, 30, 0.40000000000000002, 0, 6, "m0.5_z30_rr0.4",
     7.5, 0.07111310916785743, 0.38425796389308958,
     {7.0999999999999996, 0},
     {7.8173673073382552, 1.13963519696395},
     {7.8567229734093607, 0.82577485981446241},
     {
         {7.4905187849410559, 0.39988761741394901},
         {7.5134388994160304, 0.40239784252245026},
         {7.5651703184966426, 0.41117048295680342},
         {7.6449052453996567, 0.42980291757408612},
         {7.7514999588477282, 0.4618088808575031},
         {7.8834825564440099, 0.51059062099008523},
     }},
};

gear::LanternParams params_of(const Oracle& o) {
    gear::LanternParams p;
    p.module = o.module;
    p.numTeeth = o.numTeeth;
    p.rollerRadius = o.rollerRadius;
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
    // Interpolated runs must carry their endpoints, or the wire builder would
    // silently build a shorter edge than the chain advertises.
    for (const auto& s : segs) {
        if (s.kind == gear::SegmentKind::Interpolated) {
            if (s.points.size() < 2) {
                std::fprintf(stderr, "FAIL %s: interpolated run too short (case %s)\n", __FILE__,
                             caseName.c_str());
                ++g_failures;
                continue;
            }
            if (gear::distance(s.points.front(), s.start) > 1e-12 ||
                gear::distance(s.points.back(), s.end) > 1e-12) {
                std::fprintf(stderr, "FAIL %s: interpolated run endpoints disagree (case %s)\n",
                             __FILE__, caseName.c_str());
                ++g_failures;
            }
        }
    }
}

}  // namespace

int main() {
    for (const auto& o : kCases) {
        const gear::LanternParams p = params_of(o);
        const auto fr = gear::compute_lantern_factors(p);
        CHECK(fr.ok);
        if (!fr.ok) {
            std::fprintf(stderr, "  (case %s error: %s)\n", o.name, fr.error.c_str());
            continue;
        }
        const auto& f = fr.factors;
        CHECK_NEAR(f.pitchRadius, o.pitchRadius, "pitchRadius", o.name);
        CHECK_NEAR(f.phiMin, o.phiMin, "phiMin", o.name);
        CHECK_NEAR(f.phiMax, o.phiMax, "phiMax", o.name);

        // phi_min must actually be a root of the objective, checked against a
        // restatement of it rather than the implementation's own copy.
        CHECK(std::abs(objective(f.phiMin, f.pitchRadius, p.rollerRadius)) < 1e-9);
        // ...and it must be the MEANINGFUL root, strictly beyond the
        // stationary point, not the spurious one at the origin.
        CHECK(f.phiMin > p.rollerRadius / f.pitchRadius);

        const auto tr = gear::one_tooth_segments(p, f);
        CHECK(tr.ok);
        if (!tr.ok) {
            std::fprintf(stderr, "  (case %s segments error: %s)\n", o.name, tr.error.c_str());
            continue;
        }
        CHECK(tr.segments.size() == 4);
        if (tr.segments.size() != 4) continue;

        CHECK(tr.segments[0].kind == gear::SegmentKind::Interpolated);
        CHECK(tr.segments[1].kind == gear::SegmentKind::Arc);
        CHECK(tr.segments[2].kind == gear::SegmentKind::Interpolated);
        CHECK(tr.segments[3].kind == gear::SegmentKind::Arc);

        // Leading flank: every sample against the oracle.
        const auto& lead = tr.segments[2].points;
        CHECK(lead.size() == o.flank.size());
        if (lead.size() == o.flank.size()) {
            for (std::size_t i = 0; i < lead.size(); ++i) {
                char what[48];
                std::snprintf(what, sizeof(what), "flank[%zu]", i);
                CHECK_POINT(lead[i], o.flank[i], what, o.name);
            }
        }
        // Trailing flank is the exact mirror, in reverse order.
        const auto& trail = tr.segments[0].points;
        CHECK(trail.size() == o.flank.size());
        if (trail.size() == o.flank.size()) {
            for (std::size_t i = 0; i < trail.size(); ++i) {
                const gear::Point2d expect = gear::mirror_about_x_axis(o.flank[o.flank.size() - 1 - i]);
                char what[48];
                std::snprintf(what, sizeof(what), "mirrored flank[%zu]", i);
                CHECK_POINT(trail[i], expect, what, o.name);
            }
        }
        CHECK_POINT(tr.segments[1].mid, o.rootMid, "root arc mid", o.name);
        CHECK_POINT(tr.segments[3].mid, o.tipMid, "tip arc mid", o.name);
        CHECK_POINT(tr.segments[3].end, o.p3, "tip arc end (p_3)", o.name);

        check_structure(tr.segments, f.angularPitch, o.name);

        // Flank endpoints must sit on the radii the closed form promises.
        CHECK(std::abs(gear::norm(lead.back()) - f.maxRadius) < 1e-9);
        // The sampled run must agree with the parametric equations directly.
        for (int i = 0; i < p.sampleCount; ++i) {
            const double t = static_cast<double>(i) / static_cast<double>(p.sampleCount - 1);
            const double phi = f.phiMin + (f.phiMax - f.phiMin) * t;
            const gear::Point2d direct =
                gear::lantern_flank_point(phi, f.pitchRadius, p.rollerRadius);
            CHECK(gear::distance(lead[static_cast<std::size_t>(i)], direct) < 1e-12);
        }
    }

    // --- The spurious root, stated explicitly. The objective vanishes at
    //     phi = 0 for EVERY parameter set; the solver must never return it. ---
    {
        for (const double r0 : {3.0, 7.5, 50.0}) {
            for (const double rr : {0.2, 1.0, 4.0}) {
                CHECK(std::abs(objective(0.0, r0, rr)) < 1e-12);  // spurious root exists...
                gear::LanternParams p;
                p.module = 2.0 * r0 / 15.0;  // z=15 => pitchRadius == r0
                p.numTeeth = 15;
                p.rollerRadius = rr;
                const auto fr = gear::compute_lantern_factors(p);
                if (!fr.ok) continue;
                CHECK(fr.factors.phiMin > 0.0);                    // ...and is never returned.
                CHECK(fr.factors.phiMin > rr / fr.factors.pitchRadius);
                CHECK(std::abs(objective(fr.factors.phiMin, fr.factors.pitchRadius, rr)) < 1e-9);
            }
        }
    }

    // --- The closed-form derivative the bracket argument rests on, against a
    //     central finite difference. If this drifts, the "unique root beyond
    //     r_r/r_0" reasoning in LanternMath.h no longer holds. ---
    {
        const double r0 = 7.5;
        const double rr = 1.0;
        const double h = 1e-6;
        for (const double phi : {0.05, 0.1333, 0.3, 0.8, 1.5}) {
            const double analytic = 2.0 * (1.0 - std::cos(phi)) * (r0 * phi - rr);
            const double numeric = (objective(phi + h, r0, rr) - objective(phi - h, r0, rr)) / (2 * h);
            if (std::abs(analytic - numeric) > 1e-6) {
                std::fprintf(stderr, "FAIL %s: derivative mismatch at phi=%g: %.12g vs %.12g\n",
                             __FILE__, phi, analytic, numeric);
                ++g_failures;
            }
        }
    }

    // --- Structural sweep with no pinned oracle. ---
    for (const double m : {0.5, 1.0, 3.0}) {
        for (const int z : {6, 15, 40}) {
            for (const double rr : {0.3, 1.0, 2.5}) {
                gear::LanternParams p;
                p.module = m;
                p.numTeeth = z;
                p.rollerRadius = rr;
                p.sampleCount = 8;
                const auto fr = gear::compute_lantern_factors(p);
                if (!fr.ok) continue;
                const auto tr = gear::one_tooth_segments(p, fr.factors);
                const std::string caseName = "sweep m=" + std::to_string(m) + " z=" +
                                             std::to_string(z) + " rr=" + std::to_string(rr);
                CHECK(tr.ok);
                if (!tr.ok) {
                    std::fprintf(stderr, "  (%s: %s)\n", caseName.c_str(), tr.error.c_str());
                    continue;
                }
                check_structure(tr.segments, fr.factors.angularPitch, caseName);
                CHECK(fr.factors.phiMin < fr.factors.phiMax);
                for (const auto& s : tr.segments) {
                    CHECK(std::isfinite(s.start.x) && std::isfinite(s.start.y));
                    CHECK(std::isfinite(s.end.x) && std::isfinite(s.end.y));
                    for (const auto& q : s.points) {
                        CHECK(std::isfinite(q.x) && std::isfinite(q.y));
                    }
                }
            }
        }
    }

    // --- Refusal matrix. ---
    {
        gear::LanternParams p;
        p.numTeeth = 2;
        CHECK(!gear::compute_lantern_factors(p).ok);
    }
    {
        gear::LanternParams p;
        p.module = 0.0;
        CHECK(!gear::compute_lantern_factors(p).ok);
    }
    {
        // A zero roller radius degenerates the bracket (phi* collapses to 0,
        // where the spurious root lives) — must refuse, not return phi_min=0.
        gear::LanternParams p;
        p.rollerRadius = 0.0;
        CHECK(!gear::compute_lantern_factors(p).ok);
    }
    {
        gear::LanternParams p;
        p.sampleCount = 1;
        CHECK(!gear::compute_lantern_factors(p).ok);
    }
    {
        // A head negative enough to pull the tip inside the pitch circle.
        gear::LanternParams p;
        p.module = 1.0;
        p.rollerRadius = 1.0;
        p.head = -5.0;
        CHECK(!gear::compute_lantern_factors(p).ok);
    }
    {
        // Negative control: defaults succeed and produce a tooth.
        gear::LanternParams p;
        const auto fr = gear::compute_lantern_factors(p);
        CHECK(fr.ok);
        if (fr.ok) CHECK(gear::one_tooth_segments(p, fr.factors).ok);
    }

    if (g_failures == 0) std::fprintf(stderr, "OK: gear_math_lantern\n");
    // Exit status is 8-bit on POSIX, so returning a raw failure count reports
    // SUCCESS whenever that count is a multiple of 256 — this file's sweeps
    // can produce thousands. Clamp to a nonzero cap so a failing run can never
    // masquerade as a passing one, while small counts stay readable.
    return (g_failures == 0) ? 0 : (g_failures > 100 ? 100 : g_failures);
}
