// test_gear_math_timing.cpp — GT/HTD timing-belt pulley tooth math, checked
// against an oracle captured by RUNNING the actual upstream
// `freecad.gears` TimingGear arc algebra (v1.3.0, GPL-3.0) over all seven
// belt sections — not re-derived by hand. See test_gear_math_involute.cpp for
// the same posture and the Gear Generator implementation plan §10.1 leg 2.
//
// Beyond the pinned values, two STRUCTURAL invariants are asserted for every
// section × tooth-count combination, including ones with no pinned oracle:
//   1. arc chain continuity (arcs[i].end == arcs[i+1].start)
//   2. tooth wrap (rotate(last.end, -2pi/z) == first.start)
// These catch a transcription error in the tangency algebra even where no
// oracle value is pinned, which is most of the parameter space.
//
// No test framework (matches the prototype style): exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "kernel/gear/TimingMath.h"

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

void CHECK_NEAR(double actual, double expected, const char* what, const char* caseName) {
    if (std::abs(actual - expected) > kTol) {
        std::fprintf(stderr, "FAIL %s: %s: actual=%.17g expected=%.17g (case %s)\n", __FILE__, what,
                     actual, expected, caseName);
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

// --- Oracle: upstream arc algebra, all seven sections. `rp` is upstream's
//     own per-branch value (GT: pitch·z/2π; HTD: that minus u) — the port
//     reproduces the asymmetry deliberately, see TimingMath.h. ---
struct SectionOracle {
    gear::TimingProfile profile;
    const char* name;
    int numTeeth;
    std::size_t arcCount;
    double rp;
    gear::Point2d first;  ///< arcs.front().start
    gear::Point2d last;   ///< arcs.back().end
};

const SectionOracle kSections[] = {
    {gear::TimingProfile::Gt2, "gt2", 20, 6, 6.366197723675814,
     {0.75174963212903223, 6.0657920755579537}, {-1.1594764494334913, 6.001214491793883}},
    {gear::TimingProfile::Gt3, "gt3", 15, 6, 7.1619724391352904,
     {1.1711341496358454, 6.6790741891274967}, {-1.6467399318499838, 6.5779810593496784}},
    {gear::TimingProfile::Gt5, "gt5", 24, 6, 19.098593171027442,
     {1.9591273196471068, 18.423219086613067}, {-2.8759082966761098, 18.302522581246585}},
    {gear::TimingProfile::Gt8, "gt8", 30, 6, 38.197186342054884,
     {3.1255080161518727, 37.151545823401271}, {-4.6670325415635823, 36.98952506701459}},
    {gear::TimingProfile::Htd3, "htd3", 15, 4, 6.7809724391352901,
     {1.1499999999999999, 6.7209724391352896}, {-1.6830884918098699, 6.6076609822503256}},
    {gear::TimingProfile::Htd5, "htd5", 20, 4, 15.343994309189533,
     {1.9199999999999999, 15.203994309189532}, {-2.8722641126328572, 15.053170490669036}},
    {gear::TimingProfile::Htd8, "htd8", 36, 4, 45.150623610465857,
     {3.1600000000000001, 44.860623610465858}, {-4.6779730394408734, 44.727818177976793}},
};

// --- Oracle: every arc of two representative sections, one per construction
//     branch (gt3 = six-arc GT branch, htd5 = four-arc HTD branch). ---
struct FullOracle {
    gear::TimingProfile profile;
    const char* name;
    int numTeeth;
    std::vector<gear::Arc3> arcs;
};

const FullOracle kFull[] = {
    {gear::TimingProfile::Gt3,
     "gt3 z=15 (GT six-arc branch)",
     15,
     {
         {{1.1711341496358454, 6.6790741891274967},
          {0.98611125175932623, 6.6386945162115873},
          {0.88248278721041706, 6.4801854248046382}},
         {{0.88248278721041706, 6.4801854248046382},
          {0.83824087191849694, 6.306583096407544},
          {0.77388059701492518, 6.139391976703398}},
         {{0.77388059701492518, 6.139391976703398},
          {0, 5.6409724391352896},
          {-0.77388059701492518, 6.139391976703398}},
         {{-0.77388059701492518, 6.139391976703398},
          {-0.83824087191849694, 6.306583096407544},
          {-0.88248278721041706, 6.4801854248046382}},
         {{-0.88248278721041706, 6.4801854248046382},
          {-0.98611125175932623, 6.6386945162115873},
          {-1.1711341496358454, 6.6790741891274967}},
         {{-1.1711341496358454, 6.6790741891274967},
          {-1.4098434452092439, 6.6327919219822462},
          {-1.6467399318499838, 6.5779810593496784}},
     }},
    {gear::TimingProfile::Htd5,
     "htd5 z=20 (HTD four-arc branch)",
     20,
     {
         {{1.9199999999999999, 15.203994309189532},
          {1.6159440840897847, 15.078050225099748},
          {1.49, 14.773994309189533}},
         {{1.49, 14.773994309189533}, {0, 13.283994309189533}, {-1.49, 14.773994309189533}},
         {{-1.49, 14.773994309189533},
          {-1.6159440840897847, 15.078050225099748},
          {-1.9199999999999999, 15.203994309189532}},
         {{-1.9199999999999999, 15.203994309189532},
          {-2.3973184576450146, 15.136073043114656},
          {-2.8722641126328572, 15.053170490669036}},
     }},
};

/// The two structural invariants, checked without reference to any pinned
/// value. `caseName` identifies the failing combination.
void check_structure(const std::vector<gear::Arc3>& arcs, int numTeeth, const std::string& caseName) {
    for (std::size_t i = 0; i + 1 < arcs.size(); ++i) {
        const gear::Point2d& e = arcs[i].end;
        const gear::Point2d& s = arcs[i + 1].start;
        if (std::abs(e.x - s.x) > 1e-9 || std::abs(e.y - s.y) > 1e-9) {
            std::fprintf(stderr, "FAIL %s: arc chain discontinuity at %zu->%zu (case %s)\n",
                         __FILE__, i, i + 1, caseName.c_str());
            ++g_failures;
        }
    }
    const double a = -2.0 * kPi / static_cast<double>(numTeeth);
    const gear::Point2d wrapped = gear::rotate(arcs.back().end, a);
    const gear::Point2d& first = arcs.front().start;
    if (std::abs(wrapped.x - first.x) > 1e-9 || std::abs(wrapped.y - first.y) > 1e-9) {
        std::fprintf(stderr,
                     "FAIL %s: tooth wrap broken: rot(last.end,-2pi/z)=(%.17g,%.17g) != "
                     "first.start=(%.17g,%.17g) (case %s)\n",
                     __FILE__, wrapped.x, wrapped.y, first.x, first.y, caseName.c_str());
        ++g_failures;
    }
}

}  // namespace

int main() {
    // --- Section table + endpoints, all seven profiles. ---
    for (const auto& s : kSections) {
        gear::TimingToothParams p;
        p.profile = s.profile;
        p.numTeeth = s.numTeeth;
        const auto fr = gear::compute_timing_factors(p);
        CHECK(fr.ok);
        if (!fr.ok) {
            std::fprintf(stderr, "  (case %s error: %s)\n", s.name, fr.error.c_str());
            continue;
        }
        CHECK_NEAR(fr.factors.rp, s.rp, "rp", s.name);
        CHECK(gear::timing_profile_name(s.profile) == s.name);

        const auto tr = gear::one_tooth_arcs(p, fr.factors);
        CHECK(tr.ok);
        if (!tr.ok) {
            std::fprintf(stderr, "  (case %s arcs error: %s)\n", s.name, tr.error.c_str());
            continue;
        }
        CHECK(tr.arcs.size() == s.arcCount);
        if (tr.arcs.size() != s.arcCount) {
            std::fprintf(stderr, "  (case %s: %zu arcs, expected %zu)\n", s.name, tr.arcs.size(),
                         s.arcCount);
            continue;
        }
        CHECK_POINT(tr.arcs.front().start, s.first, "first arc start", s.name);
        CHECK_POINT(tr.arcs.back().end, s.last, "last arc end", s.name);
        check_structure(tr.arcs, s.numTeeth, s.name);
    }

    // --- Every arc of the two representative sections. ---
    for (const auto& c : kFull) {
        gear::TimingToothParams p;
        p.profile = c.profile;
        p.numTeeth = c.numTeeth;
        const auto fr = gear::compute_timing_factors(p);
        CHECK(fr.ok);
        if (!fr.ok) continue;
        const auto tr = gear::one_tooth_arcs(p, fr.factors);
        CHECK(tr.ok);
        if (!tr.ok) continue;
        CHECK(tr.arcs.size() == c.arcs.size());
        if (tr.arcs.size() != c.arcs.size()) continue;
        for (std::size_t i = 0; i < c.arcs.size(); ++i) {
            char what[64];
            std::snprintf(what, sizeof(what), "arc[%zu].start", i);
            CHECK_POINT(tr.arcs[i].start, c.arcs[i].start, what, c.name);
            std::snprintf(what, sizeof(what), "arc[%zu].mid", i);
            CHECK_POINT(tr.arcs[i].mid, c.arcs[i].mid, what, c.name);
            std::snprintf(what, sizeof(what), "arc[%zu].end", i);
            CHECK_POINT(tr.arcs[i].end, c.arcs[i].end, what, c.name);
        }
    }

    // --- Structural invariants across a SWEEP with no pinned oracle: every
    //     section over a range of tooth counts. This is where a transcription
    //     error in the tangency algebra would surface for parameter
    //     combinations nobody captured an oracle for. ---
    const gear::TimingProfile all[] = {
        gear::TimingProfile::Gt2,  gear::TimingProfile::Gt3,  gear::TimingProfile::Gt5,
        gear::TimingProfile::Gt8,  gear::TimingProfile::Htd3, gear::TimingProfile::Htd5,
        gear::TimingProfile::Htd8,
    };
    for (const auto prof : all) {
        for (const int z : {8, 12, 15, 20, 30, 48, 90}) {
            gear::TimingToothParams p;
            p.profile = prof;
            p.numTeeth = z;
            const auto fr = gear::compute_timing_factors(p);
            if (!fr.ok) continue;  // legitimately refused (pulley too small) — covered below.
            const auto tr = gear::one_tooth_arcs(p, fr.factors);
            const std::string caseName =
                gear::timing_profile_name(prof) + " z=" + std::to_string(z) + " (sweep)";
            CHECK(tr.ok);
            if (!tr.ok) {
                std::fprintf(stderr, "  (sweep %s: %s)\n", caseName.c_str(), tr.error.c_str());
                continue;
            }
            check_structure(tr.arcs, z, caseName);
            // Every emitted point must be finite — a NaN from the closed-form
            // tangency solve would otherwise slip through the invariants above
            // (NaN comparisons are false, so they never trip a `>` check).
            for (const auto& a : tr.arcs) {
                CHECK(std::isfinite(a.start.x) && std::isfinite(a.start.y));
                CHECK(std::isfinite(a.mid.x) && std::isfinite(a.mid.y));
                CHECK(std::isfinite(a.end.x) && std::isfinite(a.end.y));
            }
        }
    }

    // --- Name mapping round-trip, both directions. ---
    for (const auto prof : all) {
        gear::TimingProfile back{};
        CHECK(gear::timing_profile_from_name(gear::timing_profile_name(prof), back));
        CHECK(back == prof);
    }
    {
        // An unknown section must be refused, never defaulted — substituting a
        // different belt standard silently would produce a non-meshing pulley.
        gear::TimingProfile back{};
        CHECK(!gear::timing_profile_from_name("gt4", back));
        CHECK(!gear::timing_profile_from_name("", back));
        CHECK(!gear::timing_profile_from_name("GT2", back));  // case-sensitive by contract
    }

    // --- Refusal matrix. ---
    {
        gear::TimingToothParams p;
        p.numTeeth = 2;
        const auto r = gear::compute_timing_factors(p);
        CHECK(!r.ok);
        CHECK(!r.error.empty());
    }
    {
        // A large section on a tiny pulley: the tooth cannot fit, so the
        // factors stage must refuse rather than emit inverted arcs.
        gear::TimingToothParams p;
        p.profile = gear::TimingProfile::Htd8;
        p.numTeeth = 3;
        const auto r = gear::compute_timing_factors(p);
        CHECK(!r.ok);
    }
    {
        // Negative control for the refusal above: the same section at a sane
        // tooth count must succeed.
        gear::TimingToothParams p;
        p.profile = gear::TimingProfile::Htd8;
        p.numTeeth = 20;
        const auto r = gear::compute_timing_factors(p);
        CHECK(r.ok);
    }

    if (g_failures == 0) std::fprintf(stderr, "OK: gear_math_timing\n");
    // Exit status is 8-bit on POSIX, so returning a raw failure count reports
    // SUCCESS whenever that count is a multiple of 256 — this file's sweeps
    // can produce thousands. Clamp to a nonzero cap so a failing run can never
    // masquerade as a passing one, while small counts stay readable.
    return (g_failures == 0) ? 0 : (g_failures > 100 ? 100 : g_failures);
}
