// test_xcaf_import.cpp — WP-A W4.5: XCAF fidelity end to end.
//
// The claim under test is a SURVIVAL claim, not a parsing claim: a STEP file's
// product names and per-face colors must still be there after the bytes have gone
// through the whole replay path the document actually uses —
//
//   colored STEP  →  InspectStep (probe + xbf conversion)
//                 →  the xbf blob a document persists
//                 →  ImportStep op, sourceCodec "xbf", inside a real ExecutePlan
//                 →  BodyRecord::face_colors
//                 →  Tessellate → MESH1 FACE_COLORS (mesh_format.md §4 type 12)
//
// The old brep lane parsed fine and lost everything at step 2, which is exactly why
// the assertions here start at the far end (the MESH1 bytes a viewport would read)
// rather than at the reader.
//
// Expectations are stated GEOMETRICALLY (quantized face centroid → color), never by
// face-map index: the heal pipeline is free to renumber the map, and an index-keyed
// expectation would be asserting the reader's internal ordering instead of the
// appearance. See tests/step_fixture_util.h.
//
// stdout hygiene: like test_import_step, this binary calls OCCT in-process without
// main.cpp's startup, so it replicates redirect_occt_to_stderr() and asserts ZERO
// bytes reach the real fd 1 — including the BinXCAF read path, which is new here.
// No framework: exit code == failure count.
#include <fcntl.h>
#include <unistd.h>

#include <cstdint>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <map>
#include <set>
#include <string>
#include <vector>

#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>

#include "io/InspectStep.h"
#include "io/StepRead.h"
#include "io/XcafCodec.h"
#include "io/XcafRead.h"
#include "nlohmann/json.hpp"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "session/ShapeMetrics.h"
#include "step_fixture_util.h"
#include "tess/Tessellate.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::BodyRecord;
using onecad::session::BodyStore;
using onecad::session::Session;

namespace {

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_eq(const std::string& got, const std::string& want, const std::string& msg) {
    if (got != want) {
        std::fprintf(stderr, "FAIL: %s\n  got  %s\n  want %s\n", msg.c_str(), got.c_str(),
                     want.c_str());
        ++g_failures;
    }
}

std::string hex8(std::uint32_t v) {
    char buf[16];
    std::snprintf(buf, sizeof(buf), "%08X", v);
    return buf;
}

// Per-channel comparison: the three saturated fixture colors round-trip exactly,
// but the mid-tone crosses STEP's decimal serialization AND the linear↔sRGB
// transform twice, so ±2/255 is the honest bar for it.
bool color_near(std::uint32_t got, std::uint32_t want, int tol) {
    for (int shift = 0; shift <= 24; shift += 8) {
        const int a = static_cast<int>((got >> shift) & 0xff);
        const int b = static_cast<int>((want >> shift) & 0xff);
        if (a - b > tol || b - a > tol) return false;
    }
    return true;
}

constexpr const char* kEmpty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr const char* kNext = "1111111111111111111111111111111111111111111111111111111111111111";

void redirect_occt_to_stderr() {
    Handle(Message_Messenger) messenger = Message::DefaultMessenger();
    messenger->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    messenger->AddPrinter(new Message_PrinterOStream("cerr", Standard_False, Message_Info));
}

std::string tmp_path(const char* name) {
    return (std::filesystem::temp_directory_path() / name).string();
}

// --- MESH1 reader (mesh_format.md §2-§4), only what this test needs ----------

std::uint16_t u16(const std::vector<std::uint8_t>& b, std::size_t at) {
    return static_cast<std::uint16_t>(b[at] | (b[at + 1] << 8));
}
std::uint32_t u32(const std::vector<std::uint8_t>& b, std::size_t at) {
    std::uint32_t v = 0;
    for (int i = 0; i < 4; ++i) v |= static_cast<std::uint32_t>(b[at + i]) << (i * 8);
    return v;
}

struct Mesh1View {
    std::uint32_t face_count = 0;
    std::uint16_t flags = 0;
    bool has_face_ranges = false;
    std::uint32_t face_ranges_len = 0;
    bool has_face_colors = false;
    std::vector<std::uint32_t> face_colors;  // packed r<<24|g<<16|b<<8|a
};

Mesh1View read_mesh1(const std::vector<std::uint8_t>& b) {
    Mesh1View v;
    if (b.size() < 64 || u32(b, 0) != 0x4D455348) return v;
    v.face_count = u32(b, 0x10);
    v.flags = u16(b, 0x06);
    const std::uint16_t count = u16(b, 0x1E);
    for (std::uint16_t i = 0; i < count; ++i) {
        const std::size_t e = 0x40 + static_cast<std::size_t>(i) * 16;
        const std::uint32_t type = u32(b, e + 0x00);
        const std::uint32_t off = u32(b, e + 0x08);
        const std::uint32_t len = u32(b, e + 0x0C);
        if (type == 4) {
            v.has_face_ranges = true;
            v.face_ranges_len = len;
        } else if (type == 12) {
            v.has_face_colors = true;
            for (std::uint32_t p = 0; p + 3 < len; p += 4) {
                v.face_colors.push_back((static_cast<std::uint32_t>(b[off + p]) << 24) |
                                        (static_cast<std::uint32_t>(b[off + p + 1]) << 16) |
                                        (static_cast<std::uint32_t>(b[off + p + 2]) << 8) |
                                        static_cast<std::uint32_t>(b[off + p + 3]));
            }
        }
    }
    return v;
}

// --- ExecutePlan driver (same shape as test_import_step) --------------------

struct PlanRun {
    bool ok = false;
    std::string stopped_reason;
    std::string step_message;
};

PlanRun run_import_plan(Session& s, std::uint64_t job_id, const std::string& op_id,
                        const json& params, CancelToken& tok) {
    PlanRun run;
    json ops = json::array({json{{"opType", "ImportStep"},
                                 {"opId", op_id},
                                 {"stepIndex", 0},
                                 {"params", params}}});
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", job_id},      {"documentRevision", 0},
                 {"workerEpoch", 3},     {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({kNext})}, {"targetStep", 0},
                 {"ops", ops}};
    const Envelope resp = onecad::session::handle_execute_plan(
        s, Envelope::request(job_id, "ExecutePlan", args), ctx);
    run.ok = resp.ok.value_or(false);
    if (!run.ok) return run;
    run.stopped_reason = resp.result.value("stoppedReason", std::string{});
    if (resp.result.contains("perStepResults") && resp.result["perStepResults"].is_array() &&
        !resp.result["perStepResults"].empty()) {
        run.step_message = resp.result["perStepResults"][0].value("message", std::string{});
    }
    if (run.stopped_reason == "completed") {
        onecad::session::handle_accept_prepared(
            s, Envelope::request(job_id, "AcceptPrepared",
                                 json{{"jobId", job_id}, {"documentRevision", 0},
                                      {"workerEpoch", 3}}));
    } else {
        onecad::session::handle_discard_prepared(
            s, Envelope::request(job_id + 1000, "DiscardPrepared", json{{"jobId", job_id}}));
    }
    return run;
}

json xbf_params(const std::string& path, double unit_scale = 1.0) {
    return json{{"sourceSha256", std::string(64, 'c')}, {"sourceName", "colored.step"},
                {"sourceCodec", onecad::io::kXcafCodecName}, {"healPolicy", "v1"},
                {"unitScale", unit_scale},
                {"brepFormat", onecad::io::kXcafFormatVersion}, {"path", path}};
}

void open_session(Session& s) { s.open("doc", 0, 3, "determinism"); }

void write_blob(const std::string& path, const std::vector<std::uint8_t>& bytes) {
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    out.write(reinterpret_cast<const char*>(bytes.data()),
              static_cast<std::streamsize>(bytes.size()));
}

// --- scenarios --------------------------------------------------------------

// The probe: names must be PER ORDINAL SOLID (Rust zips them against the minted
// bodies and discards the whole list on a length mismatch), and the conversion lane
// must advertise + emit the xbf.
std::vector<std::uint8_t> test_probe(const std::string& step, const stepfx::ColoredFixture& expect,
                                     CancelToken& tok) {
    const Envelope resp = onecad::io::handle_inspect_step(
        Envelope::request(1, "InspectStep", json{{"path", step}, {"includeGeometry", true}}), tok);
    check(resp.ok.value_or(false), "probe: InspectStep ok");
    if (!resp.ok.value_or(false)) return {};

    check(resp.result.value("solidCount", 0) == 2, "probe: solidCount 2");
    check_eq(resp.result.value("geometryCodec", std::string{}), onecad::io::kXcafCodecName,
             "probe: geometryCodec xbf");
    check(resp.result.value("geometryFormat", 0) == onecad::io::kXcafFormatVersion,
          "probe: geometryFormat is the pinned BinXCAF storage version");
    check(resp.bin.size() == 1 && resp.bin[0].name == "geometry" && !resp.out_bin.empty(),
          "probe: one non-empty 'geometry' bin section");

    // Ordinal order is `ops::ordered_solids` (volume ascending), so the SMALL box
    // is ordinal 0 — the names must follow the ordinal, not the file order.
    const json& names = resp.result["productNames"];
    check(names.is_array() && names.size() == 2, "probe: one productName per ordinal solid");
    if (names.is_array() && names.size() == 2) {
        check_eq(names[0].get<std::string>(), stepfx::kPartBName,
                 "probe: ordinal 0 (the 1000 mm3 box) is PartB");
        check_eq(names[1].get<std::string>(), stepfx::kPartAName,
                 "probe: ordinal 1 (the 24000 mm3 box) is PartA");
    }
    (void)expect;
    return resp.out_bin;
}

// Colors on the published bodies, keyed by face centroid: every authored face must
// carry its color, and no face may invent one.
void check_body_colors(const BodyStore& bodies, const stepfx::ColoredFixture& expect,
                       const char* what, std::size_t& matched_out, std::size_t& total_out) {
    std::set<std::uint32_t> seen;
    for (const std::string& id : bodies.ids()) {
        const BodyRecord* rec = bodies.get(id);
        if (rec == nullptr) continue;
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(rec->geom, TopAbs_FACE, faces);
        total_out += static_cast<std::size_t>(faces.Extent());
        check(rec->face_colors.size() == static_cast<std::size_t>(faces.Extent()),
              std::string(what) + ": " + id + " face_colors is one entry per face (got " +
                  std::to_string(rec->face_colors.size()) + " for " +
                  std::to_string(faces.Extent()) + " faces)");
        if (rec->face_colors.size() != static_cast<std::size_t>(faces.Extent())) continue;
        for (int i = 1; i <= faces.Extent(); ++i) {
            const std::uint32_t got = rec->face_colors[static_cast<std::size_t>(i - 1)];
            const auto it = expect.face_colors.find(stepfx::spot_of(faces(i)));
            if (it == expect.face_colors.end()) {
                check(got == onecad::io::kUnsetColor,
                      std::string(what) + ": " + id + " face " + std::to_string(i) +
                          " invented a color " + hex8(got));
                continue;
            }
            // Saturated fixture colors are exact; the mid-tone gets ±2/255.
            const int tol = (it->second == stepfx::kColorRed || it->second == stepfx::kColorGreen ||
                             it->second == stepfx::kColorYellow || it->second == stepfx::kColorBlue)
                                ? 0
                                : 2;
            check(color_near(got, it->second, tol),
                  std::string(what) + ": " + id + " face " + std::to_string(i) + " color " +
                      hex8(got) + " != authored " + hex8(it->second));
            if (color_near(got, it->second, tol)) ++matched_out;
            seen.insert(it->second);
        }
    }
    check(seen.size() == 5,
          std::string(what) + ": all five authored colors survived (saw " +
              std::to_string(seen.size()) + ")");
}

// MESH1: the section is present, index-aligned with FACE_RANGES, and byte-equal to
// the body's own vector. The alignment is the load-bearing part — a FACE_COLORS
// whose index space is not FACE_RANGES' colors the wrong faces.
void check_mesh1_colors(const BodyStore& bodies, const char* what) {
    for (const std::string& id : bodies.ids()) {
        const BodyRecord* rec = bodies.get(id);
        if (rec == nullptr) continue;
        const onecad::tess::BodyMesh bm = onecad::tess::tessellate_body(
            rec->geom, id, "coarse", /*include_edges=*/true, nullptr, &rec->face_colors);
        check(bm.ok, std::string(what) + ": " + id + " tessellated");
        const Mesh1View v = read_mesh1(bm.blob);
        check((v.flags & 0x0010) != 0,
              std::string(what) + ": " + id + " HAS_FACE_COLORS flag bit 4 set");
        check(v.has_face_colors, std::string(what) + ": " + id + " FACE_COLORS section present");
        check(v.face_colors.size() == v.face_count,
              std::string(what) + ": " + id + " FACE_COLORS is 4·F bytes");
        check(v.has_face_ranges && v.face_ranges_len == 8 * v.face_count,
              std::string(what) + ": " + id + " FACE_RANGES is 8·F (same F, same index space)");
        check(v.face_colors == rec->face_colors,
              std::string(what) + ": " + id + " FACE_COLORS bytes == BodyRecord::face_colors, "
              "index for index");
    }
}

// A body with no authored color must produce the SAME bytes it produced before the
// section existed: no FACE_COLORS entry, flag bit clear.
void test_uncolored_is_byte_identical(const std::string& plain_step, CancelToken& tok) {
    const onecad::io::StepReadResult read = onecad::io::read_step(plain_step);
    check(read.ok() && !read.solids.empty(), "uncolored: W0 read succeeded");
    if (read.solids.empty()) return;

    // Warm-up: `tessellate_body` attaches a triangulation to the shape, so the
    // FIRST call on a virgin shape is not comparable to a later one. Compare two
    // calls that both see an already-meshed shape.
    const std::vector<std::uint32_t> none;
    (void)onecad::tess::tessellate_body(read.solids[0], "body_x", "coarse", true, nullptr,
                                        nullptr);
    const onecad::tess::BodyMesh without = onecad::tess::tessellate_body(
        read.solids[0], "body_x", "coarse", true, nullptr, nullptr);
    const onecad::tess::BodyMesh with_empty = onecad::tess::tessellate_body(
        read.solids[0], "body_x", "coarse", true, nullptr, &none);
    check(without.blob == with_empty.blob,
          "uncolored: an empty color vector is byte-identical to passing none");
    const Mesh1View v = read_mesh1(without.blob);
    check(!v.has_face_colors, "uncolored: no FACE_COLORS section");
    check((v.flags & 0x0010) == 0, "uncolored: HAS_FACE_COLORS flag bit clear");

    // …and the same through the whole op path, on a fixture with no XCAF colors.
    std::vector<std::uint8_t> blob;
    const std::string err =
        onecad::io::write_xcaf_document(read.solids, /*attrs=*/{}, blob);
    check(err.empty(), "uncolored: xbf written without attributes (" + err + ")");
    const std::string xbf = tmp_path("onecad_xcaf_plain.xbf");
    write_blob(xbf, blob);

    Session s;
    open_session(s);
    const PlanRun run = run_import_plan(s, 50, "op0", xbf_params(xbf), tok);
    check(run.ok && run.stopped_reason == "completed",
          "uncolored: xbf import completed (" + run.step_message + ")");
    const BodyStore bodies = s.bodies_copy();
    check(bodies.size() == read.solids.size(), "uncolored: every solid published");
    for (const std::string& id : bodies.ids()) {
        check(bodies.get(id)->face_colors.empty(),
              "uncolored: " + id + " carries no face colors");
        const onecad::tess::BodyMesh bm = onecad::tess::tessellate_body(
            bodies.get(id)->geom, id, "coarse", true, nullptr, &bodies.get(id)->face_colors);
        check((read_mesh1(bm.blob).flags & 0x0010) == 0,
              "uncolored: " + id + " mesh keeps HAS_FACE_COLORS clear");
    }
    std::error_code ec;
    std::filesystem::remove(xbf, ec);
}

// A malformed xbf is a RECOVERABLE step failure — never a worker teardown.
void test_garbage_xbf(const std::string& good_xbf, CancelToken& tok) {
    const std::string garbage = tmp_path("onecad_xcaf_garbage.xbf");
    write_blob(garbage, std::vector<std::uint8_t>{'n', 'o', 't', ' ', 'o', 'c', 'a', 'f'});
    const std::string truncated = tmp_path("onecad_xcaf_truncated.xbf");
    {
        std::ifstream in(good_xbf, std::ios::binary);
        std::vector<char> head(256, '\0');
        in.read(head.data(), static_cast<std::streamsize>(head.size()));
        std::ofstream out(truncated, std::ios::binary | std::ios::trunc);
        out.write(head.data(), in.gcount());
    }

    for (const std::string& bad : {garbage, truncated}) {
        Session s;
        open_session(s);
        const PlanRun run = run_import_plan(s, 60, "op0", xbf_params(bad), tok);
        check(run.ok && run.stopped_reason == "opFailed",
              "garbage-xbf: " + bad + " ⇒ OP_FAILED, not a frame error");
        check(!run.step_message.empty(), "garbage-xbf: " + bad + " reports a reason");
        check(s.bodies_copy().size() == 0, "garbage-xbf: " + bad + " publishes nothing");
        // Session still healthy.
        const PlanRun good = run_import_plan(s, 61, "op1", xbf_params(good_xbf), tok);
        check(good.ok && good.stopped_reason == "completed",
              "garbage-xbf: the session survives and takes the next import");
    }
    std::error_code ec;
    std::filesystem::remove(garbage, ec);
    std::filesystem::remove(truncated, ec);
}

template <typename Fn>
std::uintmax_t capture_stdout_bytes(Fn&& fn) {
    std::fflush(stdout);
    const std::string tmp = tmp_path("onecad_xcaf_stdout.tmp");
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
    if (!ec && bytes > 0) {
        std::ifstream leaked(tmp, std::ios::binary);
        std::string head(static_cast<std::size_t>(bytes > 400 ? 400 : bytes), '\0');
        leaked.read(head.data(), static_cast<std::streamsize>(head.size()));
        std::fprintf(stderr, "stdout leak (%llu bytes):\n%s\n",
                     static_cast<unsigned long long>(bytes), head.c_str());
    }
    std::filesystem::remove(tmp, ec);
    return ec ? 0 : bytes;
}

}  // namespace

int main() {
    redirect_occt_to_stderr();

    const std::string colored = tmp_path("onecad_xcaf_colored.step");
    const std::string plain = tmp_path("onecad_xcaf_plain.step");
    const std::string xbf = tmp_path("onecad_xcaf_colored.xbf");

    stepfx::ColoredFixture expect;
    const std::string e1 = stepfx::write_colored_step_fixture(colored, expect);
    check(e1.empty(), "fixture: colored STEP written (" + e1 + ")");
    const std::string e2 = stepfx::write_step_fixture(stepfx::make_multi_solid(), plain);
    check(e2.empty(), "fixture: plain multi-solid STEP written (" + e2 + ")");
    if (!e1.empty() || !e2.empty()) return g_failures;

    std::size_t matched = 0;
    std::size_t total = 0;
    const std::uintmax_t stdout_bytes = capture_stdout_bytes([&]() {
        CancelToken tok;
        const std::vector<std::uint8_t> blob = test_probe(colored, expect, tok);
        if (blob.empty()) return;
        write_blob(xbf, blob);

        // The survival claim: replay from the persisted xbf, not from the STEP.
        Session s;
        open_session(s);
        const PlanRun run = run_import_plan(s, 40, "op0", xbf_params(xbf), tok);
        check(run.ok && run.stopped_reason == "completed",
              "replay: xbf import completed (" + run.step_message + ")");
        const BodyStore bodies = s.bodies_copy();
        check(bodies.size() == 2, "replay: two bodies published");
        check_body_colors(bodies, expect, "replay", matched, total);
        check_mesh1_colors(bodies, "replay");

        // unitScale re-scales from the canonical bytes AND carries the colors with
        // it (they are remapped through the transform's own history).
        Session scaled;
        open_session(scaled);
        const PlanRun sr = run_import_plan(scaled, 41, "op0", xbf_params(xbf, 2.0), tok);
        check(sr.ok && sr.stopped_reason == "completed", "scaled: xbf import completed");
        const BodyStore sb = scaled.bodies_copy();
        std::size_t scaled_colored = 0;
        for (const std::string& id : sb.ids()) {
            TopTools_IndexedMapOfShape faces;
            TopExp::MapShapes(sb.get(id)->geom, TopAbs_FACE, faces);
            check(sb.get(id)->face_colors.size() == static_cast<std::size_t>(faces.Extent()),
                  "scaled: " + id + " colors stay one-per-face after the transform");
            for (std::uint32_t c : sb.get(id)->face_colors) {
                if (c != onecad::io::kUnsetColor) ++scaled_colored;
            }
        }
        check(scaled_colored == matched,
              "scaled: unitScale preserves every color (got " + std::to_string(scaled_colored) +
                  ", want " + std::to_string(matched) + ")");

        test_uncolored_is_byte_identical(plain, tok);
        test_garbage_xbf(xbf, tok);
    });
    check(stdout_bytes == 0, "stdout hygiene: zero bytes written to real stdout");

    std::fprintf(stderr, "xcaf_import: %zu/%zu faces carried an authored color\n", matched, total);
    std::error_code rm;
    for (const std::string& p : {colored, plain, xbf}) std::filesystem::remove(p, rm);
    if (g_failures == 0) std::fprintf(stderr, "xcaf_import: OK\n");
    return g_failures;
}
