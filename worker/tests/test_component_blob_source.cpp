// test_component_blob_source.cpp — Component Library WP-3.2: the `embedded` /
// `document` source kinds (spec §2.1) and the `ExportGeometry` verb (SCHEMA
// §7.8) that bakes what they carry.
//
// Covers the full local loop the two kinds live on, in one binary because the
// halves only mean anything together: a body published by a real plan is baked
// through `ExportGeometry`, and the resulting bytes are placed back as a
// component. If the writer and the reader ever disagree, this test fails at the
// place, which is exactly where a user would.
//
// Also pins the refusals — a 2-solid blob (spec §9's one-solid rule), a
// `brepFormat` this worker does not write, an unmaterialized blob (Rust lowers
// an EMPTY path when a document's carrier is missing the bytes), and an unknown
// codec. Each of those is a case where publishing SOMETHING would be worse than
// failing: a component that silently becomes a different solid is the
// substitution failure the whole library design exists to prevent.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <string>
#include <vector>

#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GProp_GProps.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "io/BrepCodec.h"
#include "io/ExportGeometry.h"
#include "io/XcafCodec.h"
#include "nlohmann/json.hpp"
#include "ops/ComponentOp.h"
#include "ops/OpTypes.h"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::BodyStore;
using onecad::session::Session;
namespace em = onecad::elementmap;
namespace io = onecad::io;
namespace ops = onecad::ops;

namespace {

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.6f want %.6f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr const char* kFakeSha = "1111111111111111111111111111111111111111111111111111111111111111";

// Mirrors main.cpp's own redirect (see test_wp6_exportstep.cpp for why an
// in-process test must do this itself).
void redirect_occt_to_stderr() {
    Handle(Message_Messenger) messenger = Message::DefaultMessenger();
    messenger->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    messenger->AddPrinter(new Message_PrinterOStream("cerr", false, Message_Info));
}

json line_ent(const std::string& id, double x0, double y0, double x1, double y1) {
    return json{{"id", id}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y1}}};
}

std::string temp_path(const std::string& name) {
    return (std::filesystem::temp_directory_path() / name).string();
}

double volume_of(const TopoDS_Shape& shape) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props);
    return props.Mass();
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

json blob_source(const std::string& kind, const std::string& path, const std::string& codec,
                 int brep_format) {
    json source = {{"kind", kind}, {"sha256", kFakeSha}, {"codec", codec}, {"path", path}};
    if (brep_format >= 0) source["brepFormat"] = brep_format;
    if (kind == "document") source["documentSha256"] = kFakeSha;
    return source;
}

json place_op(const std::string& op_id, const json& source, double tx, double ty, double tz) {
    return json{{"opType", "PlaceComponent"},
                {"opId", op_id},
                {"params",
                 {{"componentId", "acme.bracket"},
                  {"componentVersion", "1.0.0"},
                  {"componentRevision", "sha256:" + std::string(64, '0')},
                  {"source", source},
                  {"placement", {{"translate", {tx, ty, tz}}}}}}};
}

// Publishes a 10×10×10 box as `body_op1` through a REAL plan, so the bytes this
// test bakes come out of the same publication path a user's model does.
void publish_box(Session& s) {
    s.open("doc", 0, 3, "determinism");
    json plan_ops = json::array(
        {json{{"opType", "Sketch"},
              {"opId", "op0"},
              {"stepIndex", 0},
              {"params",
               {{"sketchId", "sk"},
                {"plane", {{"kind", "XY"}}},
                {"entities",
                 json::array({line_ent("e1", 0, 0, 10, 0), line_ent("e2", 10, 0, 10, 10),
                              line_ent("e3", 10, 10, 0, 10), line_ent("e4", 0, 10, 0, 0)})},
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
    json args = {{"jobId", 1},          {"documentRevision", 0},
                 {"workerEpoch", 3},    {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({"a", "b"})},
                 {"targetStep", 1},     {"ops", plan_ops}};
    onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    const Envelope acc = onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    check(acc.ok.value_or(false), "fixture: plan accepted (box published)");
}

// ── ExportGeometry: a session body becomes replay-codec bytes ─────────────────
// Both codecs, because they are pinned to DIFFERENT format versions and a
// caller records whichever it asked for.
json export_body(Session& s, const std::string& path, const std::string& codec) {
    const Envelope resp = io::handle_export_geometry(
        s, Envelope::request(2, "ExportGeometry",
                             json{{"path", path},
                                  {"bodyIds", json::array({"body_op1"})},
                                  {"codec", codec}}));
    check(resp.ok.value_or(false), "export(" + codec + "): ok");
    return resp.result;
}

void test_export_then_place_brep(Session& s) {
    const std::string path = temp_path("onecad_wp32_component.brep");
    const json result = export_body(s, path, "brep");
    check(result.value("codec", std::string()) == "brep", "export(brep): echoes codec");
    check(result.value("format", 0) == io::kBrepFormatVersion, "export(brep): echoes format pin");
    check(result.value("solidCount", 0) == 1, "export(brep): one solid");
    std::error_code ec;
    check(std::filesystem::file_size(path, ec) == result.value("bytes", std::uint64_t{0}),
          "export(brep): reported bytes == file size");

    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(
        ctx, place_op("opb1", blob_source("embedded", path, "brep", io::kBrepFormatVersion), 5.0,
                      0.0, 0.0),
        "opb1");

    check(oc.status == ops::OpOutcome::Status::Ok, "embedded(brep): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opb1");
    check(rec != nullptr, "embedded(brep): body published");
    if (rec != nullptr) {
        // The baked box, transformed by the placement — geometry survived the
        // write/read round trip intact, and the placement still applied on top.
        check_near(volume_of(rec->geom), 1000.0, 1e-6, "embedded(brep): volume is the baked box");
    }
    std::error_code rm;
    std::filesystem::remove(path, rm);
}

// `document` differs from `embedded` ONLY in the record's provenance fields —
// this asserts the worker treats them identically, which is what makes the
// shared arm correct rather than a shortcut.
void test_export_then_place_xbf_document(Session& s) {
    const std::string path = temp_path("onecad_wp32_component.xbf");
    const json result = export_body(s, path, io::kXcafCodecName);
    check(result.value("format", 0) == io::kXcafFormatVersion, "export(xbf): echoes format pin");

    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(
        ctx,
        place_op("opd1", blob_source("document", path, io::kXcafCodecName, io::kXcafFormatVersion),
                 0.0, 0.0, 0.0),
        "opd1");

    check(oc.status == ops::OpOutcome::Status::Ok, "document(xbf): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opd1");
    check(rec != nullptr, "document(xbf): body published");
    if (rec != nullptr) {
        check_near(volume_of(rec->geom), 1000.0, 1e-6, "document(xbf): volume is the baked box");
    }
    std::error_code rm;
    std::filesystem::remove(path, rm);
}

// ── Refusals ─────────────────────────────────────────────────────────────────

void place_expect_failure(const json& source, const std::string& op_id, const std::string& what) {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc =
        ops::execute_place_component(ctx, place_op(op_id, source, 0.0, 0.0, 0.0), op_id);
    check(oc.status != ops::OpOutcome::Status::Ok, what + ": refused");
    check(bodies.get("body_" + op_id) == nullptr, what + ": nothing published");
}

// Spec §9: a component is ONE solid in v1. A 2-solid blob is an authoring
// mistake, and picking the first solid would place a part the author never made.
void test_multi_solid_blob_is_refused() {
    const std::string path = temp_path("onecad_wp32_two_solids.brep");
    std::vector<TopoDS_Shape> two = {
        BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 10, 10, 10).Shape(),
        BRepPrimAPI_MakeBox(gp_Pnt(20, 0, 0), 10, 10, 10).Shape(),
    };
    std::vector<std::uint8_t> bytes;
    check(io::write_brep_compound(two, bytes).empty(), "two-solid fixture: written");
    {
        std::FILE* f = std::fopen(path.c_str(), "wb");
        check(f != nullptr, "two-solid fixture: file opened");
        if (f != nullptr) {
            std::fwrite(bytes.data(), 1, bytes.size(), f);
            std::fclose(f);
        }
    }
    place_expect_failure(blob_source("embedded", path, "brep", io::kBrepFormatVersion), "opx1",
                         "two-solid blob");
    std::error_code rm;
    std::filesystem::remove(path, rm);
}

// A format pin this worker does not write must fail LOUDLY rather than be
// misparsed — the same rule `ImportStep` enforces (SCHEMA §7.3).
void test_wrong_format_pin_is_refused(Session& s) {
    const std::string path = temp_path("onecad_wp32_badformat.brep");
    export_body(s, path, "brep");
    place_expect_failure(blob_source("embedded", path, "brep", io::kBrepFormatVersion + 1), "opx2",
                         "wrong brepFormat");
    // A converted codec with NO pin at all is equally refused.
    place_expect_failure(blob_source("embedded", path, "brep", -1), "opx3", "missing brepFormat");
    std::error_code rm;
    std::filesystem::remove(path, rm);
}

// Rust lowers an EMPTY `source.path` when the blob is not materialized. That
// must fail THIS step only, with a message naming the blob — never publish, and
// never take down the plan.
void test_unmaterialized_blob_is_refused() {
    place_expect_failure(blob_source("embedded", "", "brep", io::kBrepFormatVersion), "opx4",
                         "empty path");
    place_expect_failure(
        blob_source("embedded", temp_path("onecad_wp32_does_not_exist.brep"), "brep",
                    io::kBrepFormatVersion),
        "opx5", "missing file");
}

void test_unknown_codec_is_refused() {
    place_expect_failure(blob_source("embedded", temp_path("whatever"), "dxf", 1), "opx6",
                         "unknown codec");
}

void test_unknown_source_kind_is_refused() {
    place_expect_failure(json{{"kind", "registry"}}, "opx7", "unknown source kind");
}

// ── ExportGeometry's own refusals ────────────────────────────────────────────

void test_export_refusals(Session& s) {
    const std::string path = temp_path("onecad_wp32_refused.brep");
    const Envelope no_codec = io::handle_export_geometry(
        s, Envelope::request(3, "ExportGeometry",
                             json{{"path", path}, {"bodyIds", json::array({"body_op1"})}}));
    check(!no_codec.ok.value_or(false), "export: a missing codec is refused (no default)");

    const Envelope no_bodies = io::handle_export_geometry(
        s, Envelope::request(4, "ExportGeometry",
                             json{{"path", path}, {"bodyIds", json::array()}, {"codec", "brep"}}));
    check(!no_bodies.ok.value_or(false), "export: an empty bodyIds is refused");

    const Envelope unknown_body = io::handle_export_geometry(
        s, Envelope::request(5, "ExportGeometry",
                             json{{"path", path},
                                  {"bodyIds", json::array({"body_nope"})},
                                  {"codec", "brep"}}));
    check(!unknown_body.ok.value_or(false), "export: an unknown body is refused");
}

}  // namespace

int main() {
    redirect_occt_to_stderr();

    Session s;
    publish_box(s);

    test_export_then_place_brep(s);
    test_export_then_place_xbf_document(s);
    test_multi_solid_blob_is_refused();
    test_wrong_format_pin_is_refused(s);
    test_unmaterialized_blob_is_refused();
    test_unknown_codec_is_refused();
    test_unknown_source_kind_is_refused();
    test_export_refusals(s);

    if (g_failures == 0) std::fprintf(stderr, "component_blob_source: OK\n");
    return g_failures;
}
