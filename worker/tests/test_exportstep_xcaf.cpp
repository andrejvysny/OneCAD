// test_exportstep_xcaf.cpp — DI-5 / W3: STEP export carries body NAMES and per-face
// COLOURS through XCAF, proven by reading the written file back and comparing VALUES.
//
// test_wp6_exportstep.cpp already pins the STRUCTURAL claim (a file appears, it
// re-imports, nothing lands on fd 1). It cannot see the defect this file exists for:
// the old `STEPControl_Writer` path wrote geometry and *only* geometry, so a body's
// name and every face colour — imported or user-authored — were silently dropped on
// the way out. A structural test passes on a file that lost all of it.
//
// So every assertion here is on a value recovered from the written bytes, through
// `io::read_step_attributes` — the SAME XCAF read lane a real re-import uses. If the
// exporter and the importer ever disagree about colour space or about where a part
// colour lives, this fails; a "did the file appear" test would not.
//
// Expectations are keyed GEOMETRICALLY (quantized area + centroid, `io::face_key` /
// `io::solid_key`), never by face-map index: STEP round-trips through a rebuilt
// topology whose map order is its own business, and an index-keyed expectation would
// be asserting the reader's internal ordering instead of the appearance.
//
// stdout hygiene: like the other in-process OCCT tests this binary never runs
// main.cpp's startup, so it replicates redirect_occt_to_stderr() and requires ZERO
// bytes on the real fd 1 across the export AND the read-back (SCHEMA §2 — fd 1 is
// the OCW1 frame channel).
// No framework: exit code == failure count.
#include <fcntl.h>
#include <unistd.h>

#include <cstdint>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

#include <Interface_Static.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>

#include "io/ExportStep.h"
#include "io/InspectStep.h"
#include "io/XcafCodec.h"
#include "io/XcafRead.h"
#include "nlohmann/json.hpp"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "step_fixture_util.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::io::PackedColor;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::BodyRecord;
using onecad::session::BodyStore;
using onecad::session::Session;

namespace {

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
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
void check_color(PackedColor got, PackedColor want, const std::string& msg) {
    check_eq(hex8(got), hex8(want), msg);
}

constexpr const char* kEmpty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr const char* kNext = "1111111111111111111111111111111111111111111111111111111111111111";

// The one AP242 value OCCT 8.0.1's `write.step.schema` enum knows (values are
// 1 AP214CD / 2 AP214DIS / 3 AP203 / 4 AP214IS / 5 AP242DIS). "AP242IS" is NOT a
// value this OCCT accepts, which is why the exporter reports a refusal instead of
// silently writing AP214.
constexpr const char* kSchemaAp242 = "AP242DIS";

void redirect_occt_to_stderr() {
    Handle(Message_Messenger) messenger = Message::DefaultMessenger();
    messenger->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    messenger->AddPrinter(new Message_PrinterOStream("cerr", Standard_False, Message_Info));
}

std::string tmp_path(const char* name) {
    return (std::filesystem::temp_directory_path() / name).string();
}

// Run `fn`, returning how many bytes it wrote to the process's real stdout.
// Mirrors test_wp6_exportstep.cpp's guard.
template <typename Fn>
std::uintmax_t capture_stdout_bytes(Fn&& fn) {
    std::fflush(stdout);
    const std::string tmp = tmp_path("onecad_exportstep_xcaf_stdout.tmp");
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

std::string read_text(const std::string& path) {
    std::ifstream in(path, std::ios::binary);
    std::ostringstream buf;
    buf << in.rdbuf();
    return buf.str();
}

json line_ent(const std::string& id, double x0, double y0, double x1, double y1) {
    return json{{"id", id}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y1}}};
}

// A 10x10x10 box published as `body_op1` — the same plan test_wp6_exportstep runs.
void publish_box(Session& s) {
    s.open("doc", 0, 3, "determinism");
    json ops = json::array(
        {json{{"opType", "Sketch"},
              {"opId", "op0"},
              {"stepIndex", 0},
              {"params",
               {{"sketchId", "sk"},
                {"plane", {{"kind", "XY"}}},
                {"entities", json::array({line_ent("e1", 0, 0, 10, 0), line_ent("e2", 10, 0, 10, 10),
                                          line_ent("e3", 10, 10, 0, 10),
                                          line_ent("e4", 0, 10, 0, 0)})},
                {"constraints", json::array()}}}},
         json{{"opType", "Extrude"},
              {"opId", "op1"},
              {"stepIndex", 1},
              {"params",
               {{"sketchId", "sk"},
                {"distance", 10.0},
                {"extrudeMode", "Blind"},
                {"booleanMode", "NewBody"}}}}});

    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", 1},           {"documentRevision", 0},
                 {"workerEpoch", 3},     {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({"a", "b"})},
                 {"targetStep", 1},      {"ops", ops}};
    onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    const Envelope acc = onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    check(acc.ok.value_or(false), "fixture: box plan accepted (body published)");
}

// Import the WP-A W4.5 coloured two-box fixture through the real replay lane, so the
// session's bodies carry IMPORT-DERIVED `BodyRecord::face_colors`.
void publish_colored_import(Session& s, const std::string& step_path, CancelToken& tok) {
    s.open("doc", 0, 3, "determinism");
    const Envelope probe = onecad::io::handle_inspect_step(
        Envelope::request(1, "InspectStep", json{{"path", step_path}, {"includeGeometry", true}}),
        tok);
    check(probe.ok.value_or(false), "fixture: InspectStep ok");
    if (!probe.ok.value_or(false)) return;

    const std::string xbf = tmp_path("onecad_exportstep_xcaf_fixture.xbf");
    {
        std::ofstream out(xbf, std::ios::binary | std::ios::trunc);
        out.write(reinterpret_cast<const char*>(probe.out_bin.data()),
                  static_cast<std::streamsize>(probe.out_bin.size()));
    }

    json params = {{"sourceSha256", std::string(64, 'c')},
                   {"sourceName", "colored.step"},
                   {"sourceCodec", onecad::io::kXcafCodecName},
                   {"healPolicy", "v1"},
                   {"unitScale", 1.0},
                   {"brepFormat", onecad::io::kXcafFormatVersion},
                   {"path", xbf}};
    json ops = json::array(
        {json{{"opType", "ImportStep"}, {"opId", "imp"}, {"stepIndex", 0}, {"params", params}}});
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", 2},
                 {"documentRevision", 0},
                 {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({kNext})},
                 {"targetStep", 0},
                 {"ops", ops}};
    const Envelope resp = onecad::session::handle_execute_plan(
        s, Envelope::request(2, "ExecutePlan", args), ctx);
    check(resp.ok.value_or(false) && resp.result.value("stoppedReason", std::string{}) == "completed",
          "fixture: ImportStep plan completed");
    onecad::session::handle_accept_prepared(
        s, Envelope::request(2, "AcceptPrepared",
                             json{{"jobId", 2}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    std::error_code ec;
    std::filesystem::remove(xbf, ec);
}

Envelope export_step(Session& s, const json& args) {
    return onecad::io::handle_export_step(s, Envelope::request(9, "ExportStep", args));
}

// `TopExp::MapShapes` order of one body — index i (0-based) is TopoKey "f:(i+1)".
std::vector<TopoDS_Shape> body_faces(const BodyStore& bodies, const std::string& id) {
    std::vector<TopoDS_Shape> out;
    const BodyRecord* rec = bodies.get(id);
    if (rec == nullptr) return out;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(rec->geom, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) out.push_back(faces(i));
    return out;
}

PackedColor color_at(const onecad::io::StepAttributes& attrs, const TopoDS_Shape& face) {
    const auto it = attrs.face_colors.find(onecad::io::face_key(face));
    return it == attrs.face_colors.end() ? onecad::io::kUnsetColor : it->second;
}

// ── Scenario A: an AUTHORED name + whole-body colour + per-face overrides ─────
//
// The everyday case: the user renamed the body in the tree, gave it a colour, then
// painted two faces. All three must be in the file, and the body colour must be
// readable back as a body colour (i.e. it must reach every face the per-face table
// did not claim) — that is what makes an export look in another CAD the way it looks
// here.
void test_authored_attributes() {
    Session s;
    publish_box(s);
    const BodyStore bodies = s.bodies_copy();
    const std::vector<TopoDS_Shape> faces = body_faces(bodies, "body_op1");
    check(faces.size() == 6, "authored: the stock box has 6 faces");
    if (faces.size() != 6) return;

    constexpr PackedColor kRed = 0xFF0000FFu;
    constexpr PackedColor kGreen = 0x00FF00FFu;
    constexpr PackedColor kBodyGrey = 0x14283CFFu;  // [20,40,60,255]

    const std::string path = tmp_path("onecad_exportstep_xcaf_authored.step");
    std::error_code rm;
    std::filesystem::remove(path, rm);

    const json args = {
        {"path", path},
        {"bodyIds", json::array({"body_op1"})},
        {"schema", kSchemaAp242},
        {"bodyNames", {{"body_op1", "Bracket"}}},
        {"bodyColors", {{"body_op1", json::array({20, 40, 60, 255})}}},
        {"faceColors",
         {{"body_op1",
           {{"f:1", json::array({255, 0, 0, 255})}, {"f:3", json::array({0, 255, 0, 255})}}}}}};

    Envelope resp;
    onecad::io::StepAttributes attrs;
    const std::uintmax_t stdout_bytes = capture_stdout_bytes([&]() {
        resp = export_step(s, args);
        if (resp.ok.value_or(false)) attrs = onecad::io::read_step_attributes(path);
    });
    check(stdout_bytes == 0,
          "authored: zero bytes on the real stdout across export + read-back (stdout hygiene)");
    check(resp.ok.value_or(false), "authored: ExportStep ok");
    if (!resp.ok.value_or(false)) return;
    check(resp.result.value("written", false), "authored: written true");
    check(resp.result.value("bytes", std::uint64_t{0}) > 0, "authored: byte count > 0");
    check(resp.result.value("namedBodies", std::size_t{0}) == 1, "authored: 1 named body");
    check(resp.result.value("coloredFaces", std::size_t{0}) == 2,
          "authored: 2 per-face colours written");
    check(resp.result.value("unresolvedFaceColors", std::size_t{1}) == 0,
          "authored: every authored TopoKey addressed a face");

    // The requested schema really is what OCCT wrote — not the AP214 default it
    // falls back to when a knob value is refused.
    const std::string text = read_text(path);
    check(text.find("AP242") != std::string::npos,
          "authored: the FILE_SCHEMA header names AP242 (write.step.schema=AP242DIS took)");

    check_eq(attrs.error, "", "authored: the written file reads back through XCAF");
    const BodyRecord* rec = bodies.get("body_op1");
    check(rec != nullptr, "authored: body present");
    if (rec == nullptr) return;
    // `solid_names` is keyed by SOLID, and a body's geom may be a compound wrapping
    // one, so key on the solid rather than on the body shape.
    std::string got_name;
    for (TopExp_Explorer e(rec->geom, TopAbs_SOLID); e.More(); e.Next()) {
        const auto it = attrs.solid_names.find(onecad::io::solid_key(e.Current()));
        if (it != attrs.solid_names.end()) got_name = it->second;
    }
    check_eq(got_name, "Bracket",
             "authored: the body's display name round-trips as the STEP product name");

    check_color(color_at(attrs, faces[0]), kRed, "authored: f:1 comes back red");
    check_color(color_at(attrs, faces[2]), kGreen, "authored: f:3 comes back green");
    for (std::size_t i = 0; i < faces.size(); ++i) {
        if (i == 0 || i == 2) continue;
        check_color(color_at(attrs, faces[i]), kBodyGrey,
                    "authored: face " + std::to_string(i + 1) +
                        " inherits the whole-body colour (nothing per-face claimed it)");
    }

    std::filesystem::remove(path, rm);
}

// ── Scenario B: a TopoKey that addresses nothing is DROPPED, not nudged ──────
//
// Rust resolves every authored ElementId against the current snapshot before
// sending, so a key that misses here means the snapshot moved under the export.
// Painting the colour onto whichever face happens to hold that ordinal is the
// silent mis-bind (H5-B) the whole stack refuses to make: the export still
// succeeds, the colour is dropped, and the count says so.
void test_unresolvable_topokey_is_dropped() {
    Session s;
    publish_box(s);
    const BodyStore bodies = s.bodies_copy();
    const std::vector<TopoDS_Shape> faces = body_faces(bodies, "body_op1");

    const std::string path = tmp_path("onecad_exportstep_xcaf_stale.step");
    std::error_code rm;
    std::filesystem::remove(path, rm);

    const json args = {{"path", path},
                       {"bodyIds", json::array({"body_op1"})},
                       {"schema", kSchemaAp242},
                       {"faceColors",
                        {{"body_op1",
                          {{"f:999", json::array({255, 0, 0, 255})},
                           {"e:2", json::array({0, 0, 255, 255})}}}}}};
    const Envelope resp = export_step(s, args);
    check(resp.ok.value_or(false), "stale-key: the export still succeeds");
    if (!resp.ok.value_or(false)) return;
    check(resp.result.value("coloredFaces", std::size_t{1}) == 0,
          "stale-key: no face took a colour");
    check(resp.result.value("unresolvedFaceColors", std::size_t{0}) == 2,
          "stale-key: both unaddressable keys are counted, not guessed");

    const onecad::io::StepAttributes attrs = onecad::io::read_step_attributes(path);
    check_eq(attrs.error, "", "stale-key: the written file still reads back");
    for (std::size_t i = 0; i < faces.size(); ++i) {
        check_color(color_at(attrs, faces[i]), onecad::io::kUnsetColor,
                    "stale-key: face " + std::to_string(i + 1) + " carries no colour");
    }
    std::filesystem::remove(path, rm);
}

// ── Scenario C: import-derived colours survive, and AUTHORED wins over them ──
//
// A file that came in coloured must leave coloured even though the user never
// painted anything (layer 1), and a face the user DID paint must leave with the
// user's colour rather than the imported one (layer 2 over layer 1).
void test_import_derived_then_authored() {
    const std::string fixture = tmp_path("onecad_exportstep_xcaf_source.step");
    stepfx::ColoredFixture expect;
    const std::string err = stepfx::write_colored_step_fixture(fixture, expect);
    check_eq(err, "", "precedence: coloured source fixture authored");
    if (!err.empty()) return;

    CancelToken tok;
    Session s;
    publish_colored_import(s, fixture, tok);
    const BodyStore bodies = s.bodies_copy();
    const std::vector<std::string> ids = bodies.ids();
    check(ids.size() == 2, "precedence: the import published two bodies");
    if (ids.size() != 2) return;

    std::size_t with_colors = 0;
    for (const std::string& id : ids) {
        const BodyRecord* rec = bodies.get(id);
        if (rec != nullptr && !rec->face_colors.empty()) ++with_colors;
    }
    check(with_colors == 2,
          "precedence: both imported bodies carry import-derived face colours (the fixture's own "
          "precondition)");

    // Repaint exactly ONE face of the first body; everything else must keep what the
    // import gave it.
    const std::string target_body = ids.front();
    const std::vector<TopoDS_Shape> target_faces = body_faces(bodies, target_body);
    check(!target_faces.empty(), "precedence: the repainted body has faces");
    if (target_faces.empty()) return;
    constexpr PackedColor kAuthored = 0xFF00FFFFu;  // magenta, in neither fixture body

    const std::string path = tmp_path("onecad_exportstep_xcaf_precedence.step");
    std::error_code rm;
    std::filesystem::remove(path, rm);
    const json args = {
        {"path", path},
        {"schema", kSchemaAp242},
        {"faceColors", {{target_body, {{"f:1", json::array({255, 0, 255, 255})}}}}}};
    const Envelope resp = export_step(s, args);  // no bodyIds ⇒ "all"
    check(resp.ok.value_or(false), "precedence: ExportStep ok");
    if (!resp.ok.value_or(false)) return;
    check(resp.result.value("unresolvedFaceColors", std::size_t{1}) == 0,
          "precedence: the authored key resolved");

    const onecad::io::StepAttributes attrs = onecad::io::read_step_attributes(path);
    check_eq(attrs.error, "", "precedence: the written file reads back through XCAF");

    check_color(color_at(attrs, target_faces[0]), kAuthored,
                "precedence: the repainted face carries the AUTHORED colour, not the imported one");

    std::size_t imported_kept = 0;
    for (const std::string& id : ids) {
        const std::vector<TopoDS_Shape> faces = body_faces(bodies, id);
        for (std::size_t i = 0; i < faces.size(); ++i) {
            if (id == target_body && i == 0) continue;  // the repainted one
            const auto want = expect.face_colors.find(stepfx::spot_of(faces[i]));
            if (want == expect.face_colors.end()) continue;
            check_color(color_at(attrs, faces[i]), want->second,
                        "precedence: an untouched imported face keeps its imported colour");
            ++imported_kept;
        }
    }
    // The fixture colours 10 faces (PartA's 4 explicit + PartB's 6 inherited); one of
    // them is the face we repainted, so 9 is the full untouched set.
    check(imported_kept == 9,
          "precedence: most of the fixture's coloured faces were actually compared (got " +
              std::to_string(imported_kept) + ")");

    std::filesystem::remove(path, rm);
    std::filesystem::remove(fixture, rm);
}

// ── Scenario D: a schema OCCT does not know is refused, and the knob is restored ─
//
// `Interface_Static` is process-global. The old exporter set `write.step.schema`
// with a raw `SetCVal` whose result it ignored, so an unknown value silently left
// the previous schema in place AND leaked the last accepted one into every later
// STEP write in the worker.
void test_bad_schema_refused_and_knob_restored() {
    Session s;
    publish_box(s);
    const char* before_raw = Interface_Static::CVal("write.step.schema");
    const std::string before = before_raw != nullptr ? before_raw : "";

    const std::string path = tmp_path("onecad_exportstep_xcaf_badschema.step");
    std::error_code rm;
    std::filesystem::remove(path, rm);
    const Envelope resp = export_step(s, json{{"path", path},
                                              {"bodyIds", json::array({"body_op1"})},
                                              {"schema", "AP242IS"}});
    check(!resp.ok.value_or(false), "bad-schema: an unknown schema is a loud refusal");
    check(!std::filesystem::exists(path, rm), "bad-schema: nothing was written");

    // …and a GOOD export afterwards must not have inherited anything.
    const std::string good = tmp_path("onecad_exportstep_xcaf_afterbad.step");
    std::filesystem::remove(good, rm);
    const Envelope ok = export_step(s, json{{"path", good},
                                            {"bodyIds", json::array({"body_op1"})},
                                            {"schema", kSchemaAp242}});
    check(ok.ok.value_or(false), "bad-schema: the next export still works");
    const char* after_raw = Interface_Static::CVal("write.step.schema");
    check_eq(after_raw != nullptr ? after_raw : "", before,
             "bad-schema: write.step.schema is restored by the guard, not leaked process-wide");
    std::filesystem::remove(good, rm);
}

}  // namespace

int main() {
    redirect_occt_to_stderr();

    test_authored_attributes();
    test_unresolvable_topokey_is_dropped();
    test_import_derived_then_authored();
    test_bad_schema_refused_and_knob_restored();

    if (g_failures == 0) std::fprintf(stderr, "exportstep_xcaf: OK\n");
    return g_failures;
}
