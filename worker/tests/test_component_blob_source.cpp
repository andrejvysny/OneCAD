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
// WP-C adds the third blob-backed kind, `profile` (a LENGTH-PARAMETRIC prism of
// an embedded canonical planar face), on the same loop: the stick is baked
// through `ExtractPrismProfile` and the face it writes is placed back at a
// DIFFERENT length than the stick was.
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

#include <BRepBuilderAPI_Transform.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GProp_GProps.hxx>
#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "io/BrepCodec.h"
#include "io/ExportGeometry.h"
#include "io/XcafCodec.h"
#include "nlohmann/json.hpp"
#include "ops/ComponentOp.h"
#include "ops/OpTypes.h"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/ExtractPrismProfile.h"
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

// As `place_expect_failure`, but also pins the §8 `diagnostics[].reasonCode` —
// the machine-readable half of a `profile` refusal. The top-level code stays
// `OP_FAILED`; the taxonomy does not grow for a new refusal reason.
void place_expect_reason(const json& source, const std::string& op_id,
                         const std::string& reason_code, const std::string& what) {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc =
        ops::execute_place_component(ctx, place_op(op_id, source, 0.0, 0.0, 0.0), op_id);
    check(oc.status == ops::OpOutcome::Status::Failed, what + ": recoverable OP_FAILED");
    check(oc.error_code == "OP_FAILED", what + ": top-level code stays OP_FAILED, got " +
                                            oc.error_code);
    check(bodies.get("body_" + op_id) == nullptr, what + ": nothing published");
    const bool named = !oc.diagnostics.empty() &&
                       oc.diagnostics.front().value("reasonCode", std::string()) == reason_code;
    check(named, what + ": reasonCode == " + reason_code);
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

// ── The `profile` source kind (WP-C) ─────────────────────────────────────────
// A vendor extrusion arrives as ONE fixed length. Here the 500 mm stick is baked
// down to its canonical end-cap FACE by `ExtractPrismProfile`, and that face is
// placed back at 120 mm — the length the stick never had, which is the entire
// point of the kind.

constexpr double kProfileArea = 380.3650459150638;  // 20x20 minus a Ø5 hole

// Publishes `body_op1`: the 20x20-with-hole profile extruded 500 mm.
void publish_stick(Session& s) {
    s.open("doc", 0, 3, "determinism");
    const json sketch = {
        {"sketchId", "sk"},
        {"plane", {{"kind", "XY"}}},
        {"entities",
         json::array({line_ent("e1", 0, 0, 20, 0), line_ent("e2", 20, 0, 20, 20),
                      line_ent("e3", 20, 20, 0, 20), line_ent("e4", 0, 20, 0, 0),
                      json{{"id", "c1"},
                           {"type", "Circle"},
                           {"center", {10.0, 10.0}},
                           {"radius", 2.5}}})},
        {"constraints", json::array()}};
    const json plan_ops = json::array(
        {json{{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", 0}, {"params", sketch}},
         json{{"opType", "Extrude"},
              {"opId", "op1"},
              {"stepIndex", 1},
              {"params",
               {{"sketchId", "sk"},
                {"distance", 500.0},
                {"extrudeMode", "Blind"},
                {"booleanMode", "NewBody"}}}}});
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    const json args = {{"jobId", 1},
                       {"documentRevision", 0},
                       {"workerEpoch", 3},
                       {"expectedBaseHash", kEmpty},
                       {"prefixHashes", json::array({"a", "b"})},
                       {"targetStep", 1},
                       {"ops", plan_ops}};
    onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    const Envelope acc = onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    check(acc.ok.value_or(false), "fixture: stick published");
}

json profile_source(const std::string& path, const std::string& codec, int brep_format,
                    const json& length) {
    json source = {{"kind", "profile"},
                   {"sha256", kFakeSha},
                   {"codec", codec},
                   {"path", path},
                   {"params", {{"length", length}}}};
    if (brep_format >= 0) source["brepFormat"] = brep_format;
    return source;
}

void write_bytes(const std::string& path, const std::vector<std::uint8_t>& bytes) {
    std::FILE* f = std::fopen(path.c_str(), "wb");
    check(f != nullptr, "fixture: opened " + path);
    if (f == nullptr) return;
    std::fwrite(bytes.data(), 1, bytes.size(), f);
    std::fclose(f);
}

void test_profile_places_at_a_new_length() {
    Session s;
    publish_stick(s);
    const std::string path = temp_path("onecad_wpc_profile.brep");
    const Envelope baked = onecad::session::handle_extract_prism_profile(
        s, Envelope::request(7, "ExtractPrismProfile",
                             json{{"snapshotId", s.current_snapshot_id()},
                                  {"bodyId", "body_op1"},
                                  {"path", path}}));
    check(baked.ok.value_or(false) && baked.result.value("written", false),
          "profile: the canonical face was baked");

    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(
        ctx, place_op("opp1", profile_source(path, "brep", io::kBrepFormatVersion, 120.0), 0.0,
                      0.0, 0.0),
        "opp1");
    check(oc.status == ops::OpOutcome::Status::Ok, "profile: Ok (" + oc.error_message + ")");
    const onecad::session::BodyRecord* rec = bodies.get("body_opp1");
    check(rec != nullptr, "profile: body published");
    if (rec != nullptr) {
        const double want = kProfileArea * 120.0;
        check_near(volume_of(rec->geom), want, want * 1e-6,
                   "profile: volume is the section swept 120 mm, not the stick's 500");
    }

    // The refusals, all against the SAME good blob so only the named field moves.
    place_expect_reason(profile_source(path, "brep", io::kBrepFormatVersion, -1.0), "opp2",
                        "PROFILE_LENGTH_INVALID", "profile length -1");
    place_expect_reason(profile_source(path, "brep", io::kBrepFormatVersion, json()), "opp3",
                        "PROFILE_LENGTH_INVALID", "profile length null");
    place_expect_reason(profile_source(path, io::kXcafCodecName, io::kXcafFormatVersion, 120.0),
                        "opp4", "PROFILE_CODEC_UNSUPPORTED", "profile codec xbf");

    // A face lifted off z = 0 is refused: the worker enforces exactly the two
    // canonicality properties a prism rebuild depends on.
    const io::BrepShapeResult read = io::read_brep_shape(path);
    check(read.ok(), "profile: the baked face reads back");
    gp_Trsf lift;
    lift.SetTranslation(gp_Vec(0.0, 0.0, 3.0));
    std::vector<std::uint8_t> lifted;
    check(io::write_brep_shape(
              BRepBuilderAPI_Transform(read.shape, lift, Standard_True).Shape(), lifted)
              .empty(),
          "fixture: lifted face written");
    const std::string lifted_path = temp_path("onecad_wpc_profile_z3.brep");
    write_bytes(lifted_path, lifted);
    place_expect_reason(profile_source(lifted_path, "brep", io::kBrepFormatVersion, 120.0), "opp5",
                        "PROFILE_FACE_NOT_CANONICAL", "profile face at z = 3");

    // AN OFF-CENTRE FACE IS THE DANGEROUS ONE. It is still on z = 0 with a +Z
    // normal, so the plane and normal checks both pass — and the component would
    // land 37, 12 mm from its `placement.translate` with nothing anywhere saying
    // so. That is the silent-wrong-position failure, which must be a refusal.
    gp_Trsf slide;
    slide.SetTranslation(gp_Vec(37.0, 12.0, 0.0));
    std::vector<std::uint8_t> slid;
    check(io::write_brep_shape(
              BRepBuilderAPI_Transform(read.shape, slide, Standard_True).Shape(), slid)
              .empty(),
          "fixture: off-centre face written");
    const std::string slid_path = temp_path("onecad_wpc_profile_offcentre.brep");
    write_bytes(slid_path, slid);
    place_expect_reason(profile_source(slid_path, "brep", io::kBrepFormatVersion, 120.0), "opp7",
                        "PROFILE_FACE_NOT_CANONICAL", "profile face centroid off the origin");

    // A format pin this worker does not write is named too — §7.3 promises every
    // FACE-shaped refusal on this kind carries a reasonCode.
    place_expect_reason(profile_source(path, "brep", io::kBrepFormatVersion + 1, 120.0), "opp8",
                        "PROFILE_FORMAT_UNSUPPORTED", "profile brepFormat pin mismatch");
    place_expect_reason(profile_source(path, "brep", -1, 120.0), "opp9",
                        "PROFILE_FORMAT_UNSUPPORTED", "profile brepFormat absent");

    // A SOLID blob answers the face question with a solid — named, not misread.
    const std::string solid_path = temp_path("onecad_wpc_profile_solid.brep");
    std::vector<std::uint8_t> solid_bytes;
    check(io::write_brep_compound({BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 10, 10, 10).Shape()},
                                  solid_bytes)
              .empty(),
          "fixture: solid blob written");
    write_bytes(solid_path, solid_bytes);
    place_expect_reason(profile_source(solid_path, "brep", io::kBrepFormatVersion, 120.0), "opp6",
                        "PROFILE_BLOB_NOT_ONE_FACE", "profile blob is a solid");

    std::error_code rm;
    std::filesystem::remove(path, rm);
    std::filesystem::remove(lifted_path, rm);
    std::filesystem::remove(slid_path, rm);
    std::filesystem::remove(solid_path, rm);
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

// ── Union-at-bake (WP-F1.2, spec §9) ─────────────────────────────────────────
// A V1 LinearPattern with `fuseResult:false` gathers source + instances into ONE
// compound body — the everyday way an author ends up with a body that is not one
// solid, and exactly the case "Save as Component" now offers to fuse.

// Publishes `body_op2`: two 10³ boxes `spacing` apart in X, as one compound body.
void publish_two_solid_body(Session& s, double spacing) {
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
                {"booleanMode", "NewBody"}}}},
         json{{"opType", "LinearPattern"},
              {"opId", "op2"},
              {"stepIndex", 2},
              {"params",
               {{"sourceBodyId", "body_op1"},
                {"direction", {1.0, 0.0, 0.0}},
                {"spacing", spacing},
                {"count", 2},
                {"fuseResult", false}}}}});

    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", 1},          {"documentRevision", 0},
                 {"workerEpoch", 3},    {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({"a", "b", "c"})},
                 {"targetStep", 2},     {"ops", plan_ops}};
    onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    const Envelope acc = onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    check(acc.ok.value_or(false), "fixture: two-solid body published");
}

Envelope export_union(Session& s, const std::string& path, bool do_union) {
    return io::handle_export_geometry(
        s, Envelope::request(6, "ExportGeometry",
                             json{{"path", path},
                                  {"bodyIds", json::array({"body_op2"})},
                                  {"codec", "brep"},
                                  {"union", do_union}}));
}

// Overlapping instances fuse into one solid — the whole point of the offer.
void test_union_bakes_one_solid() {
    Session s;
    publish_two_solid_body(s, 5.0);
    const std::string path = temp_path("onecad_f12_union.brep");

    const Envelope plain = export_union(s, path, false);
    check(plain.ok.value_or(false) && plain.result.value("solidCount", 0) == 2,
          "union off: the body still bakes as two solids (Rust refuses that)");

    const Envelope fused = export_union(s, path, true);
    check(fused.ok.value_or(false), "union on: ok");
    check(fused.result.value("solidCount", 0) == 1, "union on: exactly one solid written");

    // The bytes on disk are the fused solid, not the compound — read them back
    // the way a placement does.
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(
        ctx, place_op("opu1", blob_source("document", path, "brep", io::kBrepFormatVersion), 0.0,
                      0.0, 0.0),
        "opu1");
    check(oc.status == ops::OpOutcome::Status::Ok, "union on: the fused blob places");
    const onecad::session::BodyRecord* rec = bodies.get("body_opu1");
    check(rec != nullptr, "union on: body published");
    if (rec != nullptr) {
        // 0..15 × 10 × 10 — the two boxes overlap over 5mm.
        check_near(volume_of(rec->geom), 1500.0, 1e-6, "union on: volume is the fused pair");
    }
    std::error_code rm;
    std::filesystem::remove(path, rm);
}

// Disjoint solids "fuse" in OCCT into a compound that still holds both. Writing
// that would move the failure to whoever places the component, so it is refused
// here, where the author can still split or move the bodies.
void test_union_refuses_disjoint_solids() {
    Session s;
    publish_two_solid_body(s, 40.0);
    const std::string path = temp_path("onecad_f12_union_disjoint.brep");
    const Envelope resp = export_union(s, path, true);
    check(!resp.ok.value_or(false), "union on disjoint solids: refused");
    check(resp.error.has_value() && resp.error->message.find("union refused") != std::string::npos,
          "union on disjoint solids: the message names the refusal");
    std::error_code rm;
    std::filesystem::remove(path, rm);
}

// The fuse helper itself, away from any session — both outcomes in one place.
void test_fuse_helper() {
    TopoDS_Shape out;
    const std::vector<TopoDS_Shape> touching = {
        BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 10, 10, 10).Shape(),
        BRepPrimAPI_MakeBox(gp_Pnt(5, 0, 0), 10, 10, 10).Shape(),
    };
    check(io::fuse_to_single_solid(touching, out).empty(), "fuse helper: touching pair fuses");
    check_near(volume_of(out), 1500.0, 1e-6, "fuse helper: fused volume");

    const std::vector<TopoDS_Shape> apart = {
        BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 10, 10, 10).Shape(),
        BRepPrimAPI_MakeBox(gp_Pnt(40, 0, 0), 10, 10, 10).Shape(),
    };
    TopoDS_Shape unused;
    check(!io::fuse_to_single_solid(apart, unused).empty(),
          "fuse helper: a disjoint pair is refused, not written as a compound");

    const std::vector<TopoDS_Shape> one = {BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 2, 2, 2).Shape()};
    check(io::fuse_to_single_solid(one, out).empty(), "fuse helper: a lone solid passes through");
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
    test_profile_places_at_a_new_length();
    test_export_refusals(s);
    test_fuse_helper();
    test_union_bakes_one_solid();
    test_union_refuses_disjoint_solids();

    if (g_failures == 0) std::fprintf(stderr, "component_blob_source: OK\n");
    return g_failures;
}
