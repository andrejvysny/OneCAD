// test_step_read.cpp — STEP-IMPORT WP-A W0: the pure `read_step()` core.
//
// Covers, in one binary (no framework; exit code == failure count):
//   1. FIXTURE GENERATION. Writes both STEP fixtures into the test's working
//      directory using the same STEPControl_Writer sequence ExportStep.cpp uses.
//      Nothing binary is committed — see tests/step_fixture_util.h for why
//      (corpus/step/NOTE.md records that the C++ oracle has no code-free STEP
//      producer, so the gap is closed by producing them here).
//   2. NUMERICS. Solid counts (1 and 3) and volumes against the ANALYTIC values,
//      not against a recorded blob: a 20x30x40 box is 24000 mm^3 and a cylinder
//      is pi*r^2*h, and a healing pipeline that quietly changes geometry has to
//      fail that.
//   3. THE PROCESS-GLOBAL LEAK GUARD. `Interface_Static` is global state; a
//      reader that leaves `read.precision.mode` flipped changes the behaviour of
//      the NEXT unrelated request in a long-lived worker. Snapshot the knobs,
//      run export→import→export, and require every one of them byte-identical.
//   4. STDOUT HYGIENE. `read_step()` runs OCCT code that prints through the
//      default messenger, which is bound to std::cout — i.e. fd 1, the OCW1
//      frame channel. Capture fd 1 across the whole read and require zero bytes.
//      Asserted WITHOUT installing main.cpp's global redirect first, because the
//      point is that the library function guards itself.
//   5. ROBUSTNESS. Truncated garbage bytes must produce an error RESULT, not a
//      crash and not a silent empty success.
//   6. CODEC EVIDENCE (WP-A decision input). BinTools round-trips the healed
//      multi-solid result and must reproduce the digest exactly; both lanes are
//      timed and the numbers logged to stderr. No timing assertion — a wall
//      clock is not a correctness oracle, and this is evidence for the
//      step-replay vs brep-replay decision, not a gate.
#include <fcntl.h>
#include <unistd.h>

#include <chrono>
#include <cstdint>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

#include <BRepCheck_Analyzer.hxx>
#include <BinTools.hxx>
#include <Interface_Static.hxx>
#include <STEPControl_Controller.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS_Shape.hxx>

#include "io/StepRead.h"
#include "ops/OpCommon.h"
#include "step_fixture_util.h"

using onecad::io::StepReadPolicy;
using onecad::io::StepReadResult;

namespace {

int g_failures = 0;

void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

void check_close(double actual, double expected, double rel_tol, const std::string& msg) {
    const double denom = std::abs(expected) > 1e-12 ? std::abs(expected) : 1.0;
    const double rel = std::abs(actual - expected) / denom;
    if (rel > rel_tol) {
        std::fprintf(stderr, "FAIL: %s (actual=%.9f expected=%.9f rel=%.3e)\n", msg.c_str(), actual,
                     expected, rel);
        ++g_failures;
    }
}

bool has_diag(const StepReadResult& r, const char* code) {
    for (const auto& d : r.diagnostics) {
        if (d.code == code) return true;
    }
    return false;
}

double solid_volume(const TopoDS_Shape& s) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(s, props);
    return props.Mass();
}

int count_of(const TopoDS_Shape& s, TopAbs_ShapeEnum kind) {
    TopTools_IndexedMapOfShape m;
    TopExp::MapShapes(s, kind, m);
    return m.Extent();
}

// Topology counts are the sharp end of "healing did not restructure my solid".
// The pipeline runs ShapeFix_Shape UNCONDITIONALLY, and OCCT reports
// ShapeExtend_DONE even for a tolerance-only touch-up, so the STEP_HEALED
// diagnostic alone cannot distinguish a benign pass from a destructive one.
// A box that comes back with anything other than 6/12/8 has been restructured.
void check_counts(const TopoDS_Shape& s, int faces, int edges, int verts, const std::string& what) {
    check(count_of(s, TopAbs_FACE) == faces, what + ": " + std::to_string(faces) + " faces");
    check(count_of(s, TopAbs_EDGE) == edges, what + ": " + std::to_string(edges) + " edges");
    check(count_of(s, TopAbs_VERTEX) == verts, what + ": " + std::to_string(verts) + " vertices");
}

// Run `fn` with the process's real stdout (fd 1) redirected to a temp file;
// return the byte count that landed there. Mirrors test_wp6_exportstep.cpp's
// capture_stdout_bytes — same hazard, same probe.
template <typename Fn>
std::uintmax_t capture_stdout_bytes(Fn&& fn) {
    std::fflush(stdout);
    const std::string tmp =
        (std::filesystem::temp_directory_path() / "onecad_step_read_stdout.tmp").string();
    const int cap_fd = ::open(tmp.c_str(), O_CREAT | O_WRONLY | O_TRUNC, 0600);
    const int saved_fd = ::dup(STDOUT_FILENO);
    ::dup2(cap_fd, STDOUT_FILENO);
    ::close(cap_fd);

    fn();

    std::fflush(stdout);
    ::dup2(saved_fd, STDOUT_FILENO);
    ::close(saved_fd);

    std::error_code ec;
    const std::uintmax_t bytes = std::filesystem::file_size(tmp, ec);
    std::filesystem::remove(tmp, ec);
    return ec ? 0 : bytes;
}

// --- Interface_Static snapshot (the leak guard's oracle) --------------------

// Every knob `read_step()` pins, PLUS `write.step.schema` which the export path
// writes: the guard's promise is about the read side, and pinning the write knob
// here documents whether the export path leaks it too.
struct KnobSnapshot {
    std::vector<std::pair<std::string, std::string>> values;  // name → rendered value

    bool operator==(const KnobSnapshot& o) const { return values == o.values; }
};

std::string render_int(const char* name) {
    if (!Interface_Static::IsPresent(name)) return "<absent>";
    return "i:" + std::to_string(static_cast<long long>(Interface_Static::IVal(name)));
}
std::string render_real(const char* name) {
    if (!Interface_Static::IsPresent(name)) return "<absent>";
    // Rendered through the widest fixed form so the comparison is on BITS, not on
    // a rounded printout: a knob that drifts by 1 ULP is still a leak.
    char buf[64];
    std::snprintf(buf, sizeof(buf), "r:%.17g", static_cast<double>(Interface_Static::RVal(name)));
    return buf;
}
std::string render_cstr(const char* name) {
    if (!Interface_Static::IsPresent(name)) return "<absent>";
    const Standard_CString raw = Interface_Static::CVal(name);
    return std::string("s:") + (raw != nullptr ? raw : "");
}

KnobSnapshot snapshot_read_knobs() {
    KnobSnapshot s;
    s.values.emplace_back("xstep.cascade.unit", render_cstr("xstep.cascade.unit"));
    s.values.emplace_back("read.precision.mode", render_int("read.precision.mode"));
    s.values.emplace_back("read.precision.val", render_real("read.precision.val"));
    s.values.emplace_back("read.step.product.mode", render_int("read.step.product.mode"));
    s.values.emplace_back("read.maxprecision.mode", render_int("read.maxprecision.mode"));
    s.values.emplace_back("read.maxprecision.val", render_real("read.maxprecision.val"));
    return s;
}

void report_knob_diff(const KnobSnapshot& before, const KnobSnapshot& after) {
    for (std::size_t i = 0; i < before.values.size() && i < after.values.size(); ++i) {
        if (before.values[i].second != after.values[i].second) {
            std::fprintf(stderr, "  knob leaked: %s  before=%s  after=%s\n",
                         before.values[i].first.c_str(), before.values[i].second.c_str(),
                         after.values[i].second.c_str());
        }
    }
}

// --- BinTools lane ----------------------------------------------------------

std::string bintools_roundtrip_digest(const std::vector<TopoDS_Shape>& solids,
                                      double& write_ms, double& read_ms, std::size_t& bytes_out) {
    using clock = std::chrono::steady_clock;
    BRep_Builder builder;
    TopoDS_Compound all;
    builder.MakeCompound(all);
    for (const TopoDS_Shape& s : solids) builder.Add(all, s);

    std::ostringstream oss(std::ios::binary);
    const auto t0 = clock::now();
    BinTools::Write(all, oss);
    const auto t1 = clock::now();
    const std::string blob = oss.str();
    bytes_out = blob.size();

    TopoDS_Shape back;
    std::istringstream iss(blob, std::ios::binary);
    const auto t2 = clock::now();
    BinTools::Read(back, iss);
    const auto t3 = clock::now();

    write_ms = std::chrono::duration<double, std::milli>(t1 - t0).count();
    read_ms = std::chrono::duration<double, std::milli>(t3 - t2).count();

    if (back.IsNull()) return "<null>";
    BRepCheck_Analyzer analyzer(back);
    if (analyzer.IsValid() != Standard_True) return "<invalid>";
    return stepfx::digest_solids(onecad::ops::ordered_solids(back));
}

}  // namespace

int main() {
    // NOTE: main.cpp's redirect_occt_to_stderr() is deliberately NOT replicated
    // here (unlike test_wp6_exportstep.cpp). read_step() must be self-guarding —
    // it is a library function, and W1 will call it from a lane that cannot
    // assume process startup ran. Check 4 below is that assertion.
    const std::filesystem::path dir = std::filesystem::current_path();
    const std::string box_path = (dir / "w0_step_box.step").string();
    const std::string multi_path = (dir / "w0_step_multi.step").string();
    const std::string bad_path = (dir / "w0_step_garbage.step").string();

    // --- 1. Generate the fixtures ------------------------------------------
    check(stepfx::write_step_fixture(stepfx::make_single_box(), box_path).empty(),
          "fixture: single box written");
    check(stepfx::write_step_fixture(stepfx::make_multi_solid(), multi_path).empty(),
          "fixture: multi-solid compound written");
    {
        // Truncated garbage: a plausible ISO-10303 header cut mid-token followed
        // by binary noise. Exercises the reader's error path, not its empty path.
        std::ofstream bad(bad_path, std::ios::binary | std::ios::trunc);
        bad << "ISO-10303-21;\nHEADER;\nFILE_DESCR";
        const char noise[] = {'\x00', '\xff', '\x7f', '\x01', '\xfe', '\x00'};
        bad.write(noise, sizeof(noise));
    }
    std::error_code ec;
    check(std::filesystem::file_size(box_path, ec) > 0, "fixture: box file non-empty");
    check(std::filesystem::file_size(multi_path, ec) > 0, "fixture: multi file non-empty");

    // --- 4. stdout hygiene, measured around the FIRST real read -------------
    StepReadResult box;
    const std::uintmax_t stdout_bytes =
        capture_stdout_bytes([&]() { box = onecad::io::read_step(box_path, StepReadPolicy{}); });
    check(stdout_bytes == 0,
          "hygiene: zero bytes on real stdout during read_step (fd 1 is the frame channel)");
    if (stdout_bytes != 0) {
        std::fprintf(stderr, "  %llu byte(s) leaked to fd 1\n",
                     static_cast<unsigned long long>(stdout_bytes));
    }

    // --- 2. Numerics: single box -------------------------------------------
    check(box.ok(), "box: read succeeded");
    if (!box.ok()) std::fprintf(stderr, "  error: %s\n", box.error->c_str());
    check(box.solids.size() == 1, "box: exactly 1 solid");
    if (box.solids.size() == 1) {
        check_close(solid_volume(box.solids[0]), stepfx::kBoxVolume, 1e-6,
                    "box: volume == 24000 (analytic)");
        check_counts(box.solids[0], 6, 12, 8, "box: healing left topology intact");
        BRepCheck_Analyzer analyzer(box.solids[0]);
        check(analyzer.IsValid() == Standard_True, "box: solid passes BRepCheck");
    }
    check(!has_diag(box, onecad::io::step_diag::kNoSolids), "box: no STEP_NO_SOLIDS diagnostic");
    check(!has_diag(box, onecad::io::step_diag::kInvalidShape),
          "box: no STEP_INVALID_SHAPE diagnostic");
    check(!has_diag(box, onecad::io::step_diag::kUnitConverted),
          "box: no unit conversion (fixture is written in mm)");

    // --- 2. Numerics: multi-solid ------------------------------------------
    const StepReadResult multi = onecad::io::read_step(multi_path, StepReadPolicy{});
    check(multi.ok(), "multi: read succeeded");
    if (!multi.ok()) std::fprintf(stderr, "  error: %s\n", multi.error->c_str());
    check(multi.solids.size() == 3, "multi: exactly 3 solids");
    if (multi.solids.size() == 3) {
        // ops::ordered_solids sorts by quantized volume first, so the expected
        // order is cylinder (942.48) < small box (1000) < big box (24000).
        check_close(solid_volume(multi.solids[0]), stepfx::cylinder_volume(), 1e-6,
                    "multi: solid 0 volume == pi*r^2*h (cylinder)");
        check_close(solid_volume(multi.solids[1]), stepfx::kBoxBVolume, 1e-6,
                    "multi: solid 1 volume == 1000 (small box)");
        check_close(solid_volume(multi.solids[2]), stepfx::kBoxAVolume, 1e-6,
                    "multi: solid 2 volume == 24000 (large box)");
        // A closed cylinder is 3 faces (lateral + 2 caps), 3 edges (seam + 2
        // circles), 2 vertices (seam endpoints).
        check_counts(multi.solids[0], 3, 3, 2, "multi: cylinder topology intact");
        check_counts(multi.solids[1], 6, 12, 8, "multi: small box topology intact");
        check_counts(multi.solids[2], 6, 12, 8, "multi: large box topology intact");
        for (std::size_t k = 0; k < multi.solids.size(); ++k) {
            BRepCheck_Analyzer analyzer(multi.solids[k]);
            check(analyzer.IsValid() == Standard_True,
                  "multi: solid " + std::to_string(k) + " passes BRepCheck");
        }
    }
    check(!has_diag(multi, onecad::io::step_diag::kNoSolids), "multi: no STEP_NO_SOLIDS diagnostic");
    check(!has_diag(multi, onecad::io::step_diag::kInvalidShape),
          "multi: no STEP_INVALID_SHAPE diagnostic");
    std::fprintf(stderr, "step_read: box diagnostics =");
    for (const auto& d : box.diagnostics) std::fprintf(stderr, " %s", d.code.c_str());
    std::fprintf(stderr, "\nstep_read: multi diagnostics =");
    for (const auto& d : multi.diagnostics) std::fprintf(stderr, " %s", d.code.c_str());
    std::fprintf(stderr, "\nstep_read: multi product names (%zu root(s)) =",
                 multi.product_names.size());
    for (const auto& n : multi.product_names) {
        std::fprintf(stderr, " '%s'", n.c_str());
    }
    std::fprintf(stderr, "\n");

    // --- 5. Robustness: garbage in, error result out ------------------------
    const StepReadResult bad = onecad::io::read_step(bad_path, StepReadPolicy{});
    check(!bad.ok(), "garbage: read reports an error result (no crash, no silent success)");
    check(bad.solids.empty(), "garbage: no solids returned");
    if (!bad.ok()) std::fprintf(stderr, "step_read: garbage error = %s\n", bad.error->c_str());

    const StepReadResult missing =
        onecad::io::read_step((dir / "w0_does_not_exist.step").string(), StepReadPolicy{});
    check(!missing.ok(), "missing file: read reports an error result");

    const StepReadResult empty_path = onecad::io::read_step("", StepReadPolicy{});
    check(!empty_path.ok(), "empty path: read reports an error result");

    // --- 3. Interface_Static leak guard -------------------------------------
    // Init first: the knobs are registered by the STEP controller, so a snapshot
    // taken before Init would read "<absent>" for everything and pass vacuously.
    STEPControl_Controller::Init();
    const KnobSnapshot before = snapshot_read_knobs();
    // `write.step.schema` is OWNED by the export path (ExportStep.cpp sets it and
    // does not restore it), so it is observed and logged, never asserted — the
    // guard under test is read_step's.
    const std::string write_schema_before = render_cstr("write.step.schema");
    {
        const std::string leak_path = (dir / "w0_step_leak.step").string();
        check(stepfx::write_step_fixture(stepfx::make_single_box(), leak_path).empty(),
              "leak guard: export #1 ok");
        const StepReadResult r = onecad::io::read_step(leak_path, StepReadPolicy{});
        check(r.ok() && r.solids.size() == 1, "leak guard: import ok");
        check(stepfx::write_step_fixture(stepfx::make_single_box(), leak_path).empty(),
              "leak guard: export #2 ok");
        std::filesystem::remove(leak_path, ec);
    }
    const KnobSnapshot after = snapshot_read_knobs();
    check(before == after,
          "leak guard: every read-side Interface_Static knob byte-identical after "
          "export->import->export");
    if (!(before == after)) report_knob_diff(before, after);
    std::fprintf(stderr, "step_read: write.step.schema before=%s after=%s (export-owned, observed)\n",
                 write_schema_before.c_str(), render_cstr("write.step.schema").c_str());

    // A guard that never restores anything would also pass an all-absent
    // comparison; prove the knobs are actually registered and readable.
    bool all_present = true;
    for (const auto& kv : after.values) all_present = all_present && kv.second != "<absent>";
    check(all_present, "leak guard: all pinned knobs are registered (snapshot is not vacuous)");

    // Prove the guard RESTORES rather than merely never-setting: perturb a knob,
    // read, and require the perturbed value back.
    Interface_Static::SetIVal("read.step.product.mode", 0);
    (void)onecad::io::read_step(box_path, StepReadPolicy{});
    check(Interface_Static::IVal("read.step.product.mode") == 0,
          "leak guard: a caller-set knob survives read_step (restore, not no-op)");
    Interface_Static::SetIVal("read.step.product.mode", 1);

    // --- 6. BinTools codec evidence ----------------------------------------
    if (multi.ok() && !multi.solids.empty()) {
        using clock = std::chrono::steady_clock;
        const auto s0 = clock::now();
        const StepReadResult timed = onecad::io::read_step(multi_path, StepReadPolicy{});
        const auto s1 = clock::now();
        const double step_ms = std::chrono::duration<double, std::milli>(s1 - s0).count();

        double bin_write_ms = 0.0;
        double bin_read_ms = 0.0;
        std::size_t bin_bytes = 0;
        const std::string round =
            bintools_roundtrip_digest(multi.solids, bin_write_ms, bin_read_ms, bin_bytes);
        check(round == stepfx::digest_solids(multi.solids),
              "bintools: roundtrip reproduces the ordered-solids digest exactly");
        if (round != stepfx::digest_solids(multi.solids)) {
            std::fprintf(stderr, "  bintools digest: %s\n  step digest:     %s\n", round.c_str(),
                         stepfx::digest_solids(multi.solids).c_str());
        }
        check(timed.ok(), "bintools: timing read succeeded");

        const std::uintmax_t step_bytes = std::filesystem::file_size(multi_path, ec);
        std::fprintf(stderr,
                     "step_read TIMING (codec evidence, multi-solid fixture):\n"
                     "  step full pipeline read : %8.3f ms  (%llu bytes on disk)\n"
                     "  bintools write          : %8.3f ms  (%zu bytes)\n"
                     "  bintools read           : %8.3f ms\n"
                     "  step/bintools-read ratio: %8.2fx\n",
                     step_ms, static_cast<unsigned long long>(ec ? 0 : step_bytes), bin_write_ms,
                     bin_bytes, bin_read_ms, bin_read_ms > 0.0 ? step_ms / bin_read_ms : 0.0);
    }

    std::filesystem::remove(bad_path, ec);
    std::filesystem::remove(box_path, ec);
    std::filesystem::remove(multi_path, ec);
    if (g_failures == 0) std::fprintf(stderr, "step_read: OK\n");
    return g_failures;
}
