#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <unistd.h>
#include <filesystem>
#include <fstream>
#include <string>
#include <type_traits>
#include <sstream>
#include <set>
#include <mutex>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <sys/stat.h>
#include <BRep_Builder.hxx>
#include <TopoDS_Compound.hxx>
#include <STEPControl_Writer.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <TDocStd_Application.hxx>
#include <TDocStd_Document.hxx>
#include <TDocStd_FormatVersion.hxx>
#include <TNaming_Builder.hxx>
#include <PCDM_StoreStatus.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <TopoDS.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "io/SourceCapture.h"
#include "io/SourceClaimBatch.h"
#include "io/SourceIdentity.h"
#include "io/OcafApp.h"
#include "io/BrepCodec.h"
#include "io/StepRead.h"
#include "io/XcafCodec.h"
#include "ops/ImportOp.h"
#include "ops/OpTypes.h"
#include "nlohmann/json.hpp"
#include "util/Hashing.h"

static_assert(!std::is_copy_constructible_v<onecad::io::SourceClaimBatch>);
static_assert(std::is_same_v<decltype(std::declval<onecad::io::SourceClaimBatch&>().mutable_items()),
                             std::span<onecad::io::SourceFaceClaim>>);
namespace fs = std::filesystem;
static int failures = 0;
static void check(bool value, const char* message) {
    if (!value) { std::fprintf(stderr, "FAIL: %s\n", message); ++failures; }
}

static std::vector<std::uint8_t> legacy_xbf(const TopoDS_Shape& box, bool colored, bool duplicate = false) {
    std::lock_guard lock(onecad::io::ocaf_mutex());
    Handle(TDocStd_Document) doc;
    onecad::io::ocaf_application()->NewDocument("BinXCAF", doc);
    if (doc.IsNull()) return {};
    onecad::io::OcafDocGuard guard(doc);
    doc->ChangeStorageFormatVersion(TDocStd_FormatVersion_VERSION_12);
    const auto shapes = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
    const auto root = shapes->AddShape(box, Standard_False);
    if (colored) {
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(box, TopAbs_FACE, faces);
        const auto face = shapes->AddSubShape(root, faces(1));
        if (duplicate) {
            TNaming_Builder alias(root.NewChild());
            alias.Generated(faces(1));
        }
        const auto colors = XCAFDoc_DocumentTool::ColorTool(doc->Main());
        colors->SetColor(face, onecad::io::unpack_srgb(0xff0000ff), XCAFDoc_ColorSurf);
    }
    std::ostringstream stream(std::ios::out | std::ios::binary);
    if (onecad::io::ocaf_application()->SaveAs(doc, stream) != PCDM_SS_OK) return {};
    const std::string bytes = stream.str();
    return {bytes.begin(), bytes.end()};
}

int main() {
    {
        constexpr std::size_t bytes = onecad::io::SourceClaimBatch::kBatchOverhead +
                                      onecad::io::SourceClaimBatch::kBytesPerFace;
        onecad::io::SourceClaimBudget budget(bytes);
        auto batch = std::make_shared<onecad::io::SourceClaimBatch>();
        check(batch->admit(1, budget), "claim batch reserves before allocation");
        check(batch->append(onecad::io::SourceFaceClaim{}), "admitted face appended");
        check(!batch->append(onecad::io::SourceFaceClaim{}), "admitted face count enforced");
        onecad::io::SourceClaimBatch oversized;
        onecad::io::SourceClaimBudget another(bytes);
        check(oversized.admit(1, another), "admit bounds test");
        onecad::io::SourceFaceClaim invalid;
        invalid.xbf_face_label = std::string(257, '1');
        check(!oversized.append(std::move(invalid)), "oversize label refused");
        {
            onecad::io::SourceClaimBudget move_budget(bytes);
            onecad::io::SourceClaimBatch original;
            check(original.admit(1, move_budget), "move test reserves original batch");
            onecad::io::SourceClaimBatch moved(std::move(original));
            check(!original.append(onecad::io::SourceFaceClaim{}), "moved-from batch cannot append without admission");
            check(!original.admit(1, move_budget), "moved-from batch cannot bypass shared budget");
            check(moved.append(onecad::io::SourceFaceClaim{}), "moved batch retains admission");
        }
        auto checkpoint = batch;
        batch.reset();
        check(budget.used() == bytes && !budget.acquire(1), "checkpoint alias charges batch once");
        checkpoint.reset();
        check(budget.used() == 0, "last batch owner releases reservation");
    }
    const fs::path source = fs::path(ONECAD_REPO_ROOT) / "fixtures/public-cad/prusa/MINI-fsenzor-box.stp";
    const std::string digest = "c8b06ef5271166c367d27be13e9c8f93a7ae07707033e4d23bd1ed5997285cb6";
    auto captured = onecad::io::capture_source(source.string(), digest);
    check(captured.ok(), "capture pinned STEP bytes");
    check(!onecad::io::capture_source(source.string(), std::string(64, '0')).ok(), "wrong SHA refused");
    if (captured.ok()) {
        auto read = onecad::io::read_step_bytes(captured.bytes);
        check(read.ok(), "parse captured STEP");
        check(read.solids.size() == 1, "one STEP solid");
        check(read.face_claims.size() == 166, "all source faces uniquely mapped");
        bool target = false;
        bool healed_replacement = false;
        for (const auto& c : read.face_claims) {
            if (c.step_face_entity == 3066 && c.source_face_replaced) healed_replacement = true;
            if (c.step_root_entity == 5850 && c.step_face_entity == 3042 &&
                c.source_sha256 == digest && c.reader_policy == "step-v1" &&
                !c.kernel_fingerprint.empty()) target = true;
        }
        check(target, "source root and AdvancedFace entity labels retained");
        check(healed_replacement, "ShapeFix replacement retains exact source face label");
        onecad::io::StepReadPolicy custom_policy;
        custom_policy.precision_mode = 1;
        const auto custom = onecad::io::read_step_bytes(captured.bytes, custom_policy);
        bool custom_partial = false;
        for (const auto& diag : custom.diagnostics) {
            if (diag.code == "SOURCE_FACE_IDENTITY_PARTIAL") custom_partial = true;
        }
        check(custom.ok() && custom.face_claims.empty() && custom_partial,
              "nonstandard STEP policy preserves geometry and refuses claims");
    }
    if (captured.ok()) {
        auto hold = onecad::io::source_claim_budget().acquire(onecad::io::kSourceIdentityBudgetBytes);
        check(static_cast<bool>(hold), "reserve all source identity budget for exhaustion test");
        const auto exhausted = onecad::io::read_step_bytes(captured.bytes);
        bool diagnosed = false;
        for (const auto& diag : exhausted.diagnostics) {
            if (diag.code == "SOURCE_FACE_IDENTITY_PARTIAL") diagnosed = true;
        }
        check(exhausted.ok() && !exhausted.solids.empty() && exhausted.face_claims.empty() && diagnosed,
              "budget exhaustion preserves STEP geometry and explicitly omits claims");
    }
    const fs::path assembly_path = fs::path(ONECAD_REPO_ROOT) /
        "fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp";
    const auto assembly_source = onecad::io::capture_source(
        assembly_path.string(), "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09");
    const auto assembly = onecad::io::read_step_bytes(assembly_source.bytes);
    check(assembly_source.ok() && assembly.ok() && !assembly.solids.empty(),
          "STEP assembly geometry remains readable");
    check(assembly.face_claims.empty(), "unproven STEP assembly occurrence paths emit no claims");
    bool assembly_partial = false;
    for (const auto& diag : assembly.diagnostics) {
        if (diag.code == "SOURCE_FACE_IDENTITY_PARTIAL") assembly_partial = true;
    }
    check(assembly_partial, "STEP assembly claim omission is explicit");
    char tmp_pattern[] = "/tmp/onecad-source-claim-XXXXXX";
    char* dir = mkdtemp(tmp_pattern);
    if (dir == nullptr) return 2;
    const fs::path tmp_root(dir);
    const fs::path swap = tmp_root / "swap.step";
    if (captured.ok()) {
        { std::ofstream out(swap, std::ios::binary); out.write(reinterpret_cast<const char*>(captured.bytes.data()), captured.bytes.size()); }
        const auto frozen = onecad::io::capture_source(swap.string(), digest);
        { std::ofstream out(swap, std::ios::binary | std::ios::app); out.put('\n'); }
        check(!onecad::io::capture_source(swap.string(), digest).ok(), "replaced bytes cannot retain old SHA");
        const auto replay = onecad::io::read_step_bytes(frozen.bytes);
        check(replay.ok() && replay.face_claims.size() == 166, "captured source remains usable after path swap");
        std::ifstream in(swap, std::ios::binary);
        const std::string changed((std::istreambuf_iterator<char>(in)), {});
        const auto new_digest = onecad::hashing::sha256_hex(changed);
        check(new_digest != digest, "equivalent STEP bytes have distinct digest");
        const auto changed_capture = onecad::io::capture_source(swap.string(), new_digest);
        const auto changed_read = onecad::io::read_step_bytes(changed_capture.bytes);
        check(changed_capture.ok() && changed_read.ok() && changed_read.face_claims.size() == 166,
              "equivalent changed bytes parse but retain distinct identity");
        if (!changed_read.face_claims.empty()) {
            check(changed_read.face_claims.front().source_sha256 == new_digest,
                  "equivalent changed STEP claims bind the new bytes");
        }

        onecad::session::BodyStore bodies;
        onecad::elementmap::ElementMapPartition partition;
        std::vector<std::pair<std::string, nlohmann::json>> sketches;
        std::string last;
        onecad::ops::OpContext ctx{bodies, &sketches, partition, &last};
        nlohmann::json params = {{"sourceSha256", digest}, {"sourceCodec", "step"},
                                 {"healPolicy", "v1"}, {"unitScale", 2.0}, {"path", source.string()}};
        const auto out = onecad::ops::execute_import_step(ctx, {{"params", params}}, "00000000-0000-0000-0000-000000000001");
        check(out.status == onecad::ops::OpOutcome::Status::Ok, "real ImportOp succeeds");
        check((out.source_face_claims ? out.source_face_claims->size() : 0) == 166, "ImportOp carries all mapped STEP faces");
        bool mapped = false;
        for (const auto& claim : out.source_face_claims->items()) {
            if (claim.step_root_entity == 5850 && claim.step_face_entity == 3042) {
                BRepAdaptor_Surface surface(TopoDS::Face(claim.face));
                mapped = claim.source_sha256 == digest && claim.producer_id == "00000000-0000-0000-0000-000000000001" &&
                         claim.body_id == "body_00000000-0000-0000-0000-000000000001" && claim.unit_scale == 2.0 &&
                         surface.GetType() == GeomAbs_Cylinder && std::abs(surface.Cylinder().Radius() - 2.8) < 1e-12;
                const auto cert = onecad::io::source_identity_json(claim);
                check(cert["sourceSha256"] == digest && cert["codec"] == "step" &&
                      cert["codecVersion"] == "occt-step-stream-v1" &&
                      cert["readerPolicy"] == "step-v1" && cert["unitScaleBits"] == "4000000000000000" &&
                      cert["sourceAddress"] == nlohmann::json({{"kind", "step"}, {"rootEntityLabel", 5850},
                          {"occurrencePath", nlohmann::json::array()}, {"faceEntityLabel", 3042}}),
                      "producer claim serializes exact typed certificate");
                check(onecad::io::source_identity_matches(claim, cert), "exact certificate matches claim");
                for (const auto& mutation : {"sourceSha256", "bodyId", "readerPolicy", "unitScaleBits"}) {
                    auto forged = cert;
                    forged[mutation] = "wrong";
                    check(!onecad::io::source_identity_matches(claim, forged), "altered scope refused");
                }
                auto forged = cert;
                forged["sourceAddress"]["faceEntityLabel"] = 3043;
                check(!onecad::io::source_identity_matches(claim, forged), "altered source face refused");
                forged = cert;
                forged["version"] = 1.0;
                check(!onecad::io::source_identity_matches(claim, forged), "float version refused");
                forged = cert;
                forged["sourceAddress"]["faceEntityLabel"] = 3042.0;
                check(!onecad::io::source_identity_matches(claim, forged), "float face label refused");
                forged = cert;
                forged["unknown"] = true;
                check(!onecad::io::source_identity_matches(claim, forged), "unknown certificate field refused");
                check(!onecad::io::source_identity_matches(claim, nullptr), "null certificate refused");
            }
        }
        check(mapped, "STEP source face claim survives uniform scale and carries producer scope");
    }
    const TopoDS_Shape first = BRepPrimAPI_MakeBox(2, 3, 4).Shape();
    const TopoDS_Shape second = BRepPrimAPI_MakeBox(gp_Pnt(20, 0, 0), 3, 4, 5).Shape();
    std::vector<std::string> reordered_hashes;
    for (int order = 0; order < 2; ++order) {
        const fs::path ordered_path = tmp_root / (order == 0 ? "ordered.step" : "reordered.step");
        STEPControl_Writer writer;
        check(writer.Transfer(order == 0 ? first : second, STEPControl_AsIs) == IFSelect_RetDone,
              "first STEP root transferred");
        check(writer.Transfer(order == 0 ? second : first, STEPControl_AsIs) == IFSelect_RetDone,
              "second STEP root transferred");
        check(writer.Write(ordered_path.string().c_str()) == IFSelect_RetDone, "multi-root STEP written");
        std::ifstream in(ordered_path, std::ios::binary);
        const std::string bytes((std::istreambuf_iterator<char>(in)), {});
        const std::string sha = onecad::hashing::sha256_hex(bytes);
        reordered_hashes.push_back(sha);
        const auto capture = onecad::io::capture_source(ordered_path.string(), sha);
        const auto read = onecad::io::read_step_bytes(capture.bytes);
        check(read.ok() && read.solids.size() == 2 && read.face_claims.size() == 12,
              "each reordered root retains all source-qualified face claims");
        std::set<int> root_ids;
        for (const auto& claim : read.face_claims) {
            root_ids.insert(claim.step_root_entity);
            check(claim.source_sha256 == sha, "root claim is qualified by its exact source bytes");
        }
        check(root_ids.size() == 2, "independent STEP roots retain distinct labels");
        fs::remove(ordered_path);
    }
    check(reordered_hashes[0] != reordered_hashes[1], "root reorder changes exact source identity");
    const fs::path tmp = tmp_root / "large";
    { std::ofstream out(tmp, std::ios::binary | std::ios::trunc); out.put('x'); }
    fs::resize_file(tmp, onecad::io::kMaxImportSourceBytes + 1);
    check(!onecad::io::capture_source(tmp.string(), std::string(64, '0')).ok(), "oversize source refused");
    const auto fifo = tmp_root / "fifo";
    check(::mkfifo(fifo.c_str(), 0600) == 0, "create FIFO fixture");
    check(!onecad::io::capture_source(fifo.string(), std::string(64, '0')).ok(), "FIFO source refused without blocking");
    check(!onecad::io::capture_source("/dev/null", std::string(64, '0')).ok(), "device source refused");
    check(!onecad::io::capture_source((tmp_root / "missing").string(), std::string(64, '0')).ok(), "missing source refused");
    fs::remove(fifo);
    fs::remove(tmp);
    fs::remove(swap);

    const TopoDS_Shape box = BRepPrimAPI_MakeBox(4, 5, 6).Shape();
    TopTools_IndexedMapOfShape box_faces;
    TopExp::MapShapes(box, TopAbs_FACE, box_faces);
    gp_Trsf scale;
    scale.SetScale(gp_Pnt(0, 0, 0), 2.0);
    TopoDS_Shape reversed_box = box;
    reversed_box.Reverse();
    BRep_Builder nested_builder;
    TopoDS_Compound nested;
    nested_builder.MakeCompound(nested);
    nested_builder.Add(nested, reversed_box);
    BRepBuilderAPI_Transform scaled(nested, scale, Standard_True);
    TopTools_IndexedMapOfShape scaled_faces;
    TopExp::MapShapes(scaled.Shape(), TopAbs_FACE, scaled_faces);
    TopTools_IndexedMapOfShape nested_faces;
    TopExp::MapShapes(nested, TopAbs_FACE, nested_faces);
    for (int i = 1; i <= nested_faces.Extent(); ++i) {
        TopoDS_Shape image = scaled.ModifiedShape(nested_faces(i));
        check(!image.IsNull(), "scaled nested face has a transform image");
        if (image.IsNull()) continue;
        image.Orientation(nested_faces(i).Orientation());
        check(onecad::io::oriented_source_member(scaled_faces, image),
              "positive scale preserves each nested source occurrence orientation");
    }
    TopoDS_Shape oriented = scaled_faces(1);
    check(onecad::io::oriented_source_member(scaled_faces, oriented), "oriented scaled nested face accepted");
    oriented.Reverse();
    check(scaled_faces.FindIndex(oriented) != 0 && !onecad::io::oriented_source_member(scaled_faces, oriented),
          "IsSame reversed scaled face refused");
    const auto complete = [&] {
        std::vector<onecad::io::SourceFaceClaim> claims;
        for (int i = 1; i <= box_faces.Extent(); ++i) {
            onecad::io::SourceFaceClaim claim;
            claim.face = box_faces(i);
            claim.step_face_entity = i;
            claims.push_back(std::move(claim));
        }
        return claims;
    };
    auto valid = complete();
    onecad::io::retain_complete_source_faces(valid, box);
    check(valid.size() == 6, "bijection retains complete oriented face map");
    auto merged = complete();
    merged[1].face = merged[0].face;
    onecad::io::retain_complete_source_faces(merged, box);
    check(merged.empty(), "merged source faces refuse the stage");
    auto missing = complete();
    missing.pop_back();
    onecad::io::retain_complete_source_faces(missing, box);
    check(missing.empty(), "missing source face refuses the stage");
    auto reversed = complete();
    reversed[0].face.Reverse();
    onecad::io::retain_complete_source_faces(reversed, box);
    check(reversed.empty(), "unproven orientation refuses the stage");
    BRep_Builder split_builder;
    TopoDS_Compound split_face;
    split_builder.MakeCompound(split_face);
    split_builder.Add(split_face, box_faces(1));
    split_builder.Add(split_face, box_faces(2));
    auto split = complete();
    split[0].face = split_face;
    onecad::io::retain_complete_source_faces(split, box);
    check(split.empty(), "split source face refuses the stage");

    for (bool colored : {false, true}) {
        const auto old = legacy_xbf(box, colored);
        const auto old_read = onecad::io::read_xcaf_solids_bytes(old);
        check(old_read.ok() && old_read.solids.size() == 1, "legacy XBF geometry remains readable");
        check(old_read.face_claims.size() == (colored ? 1u : 0u), "legacy XBF claims only explicit labels");
        check(old_read.unclaimed_faces == (colored ? 5u : 6u), "legacy XBF reports unlabeled faces");
    }
    const auto duplicate = onecad::io::read_xcaf_solids_bytes(legacy_xbf(box, true, true));
    check(duplicate.ok() && duplicate.face_claims.empty(), "duplicate XBF face labels cannot claim an identity");
    check(duplicate.unclaimed_faces == 6, "duplicate XBF label reports all faces unclaimed");
    std::vector<std::uint8_t> xbf;
    check(onecad::io::write_xcaf_document({box}, {}, xbf).empty(), "write uncolored XBF");
    auto decoded = onecad::io::read_xcaf_solids_bytes(xbf);
    check(decoded.ok(), "read captured XBF");
    check(decoded.solids.size() == 1, "XBF one solid");
    check(decoded.face_claims.size() == 6, "all uncolored faces have persisted labels");
    if (!decoded.attributes.empty()) check(decoded.attributes[0].face_colors.empty(), "no colors invented");
    {
        auto& budget = onecad::io::source_claim_budget();
        auto hold = budget.acquire(onecad::io::kSourceIdentityBudgetBytes - budget.used());
        check(static_cast<bool>(hold), "reserve remaining budget for XBF exhaustion test");
        const auto exhausted = onecad::io::read_xcaf_solids_bytes(xbf);
        check(exhausted.ok() && exhausted.solids.size() == 1 && exhausted.face_claims.empty() &&
              exhausted.unclaimed_faces == 6, "budget exhaustion preserves XBF geometry and reports omitted faces");
    }
    std::vector<std::uint8_t> duplicate_roots;
    check(!onecad::io::write_xcaf_document({box, box}, {}, duplicate_roots).empty(),
          "XBF refuses repeated identical shape handles that collapse root labels");
    const TopoDS_Shape coincident_box = BRepPrimAPI_MakeBox(4, 5, 6).Shape();
    std::vector<std::uint8_t> coincident;
    check(onecad::io::write_xcaf_document({box, coincident_box}, {}, coincident).empty(),
          "XBF writes distinct coincident source roots");
    const auto coincident_read = onecad::io::read_xcaf_solids_bytes(coincident);
    check(coincident_read.ok() && coincident_read.solids.size() == 2 && coincident_read.face_claims.size() == 12,
          "coincident roots retain distinct explicit source identities");
    std::set<std::string> root_labels;
    for (const auto& claim : coincident_read.face_claims) root_labels.insert(claim.xbf_root_label);
    check(root_labels.size() == 2, "coincident XBF roots use distinct label scopes");
    std::vector<std::uint8_t> ordered_xbf, reordered_xbf;
    check(onecad::io::write_xcaf_document({box, second}, {}, ordered_xbf).empty() &&
              onecad::io::write_xcaf_document({second, box}, {}, reordered_xbf).empty(),
          "reordered XBF root fixtures written");
    const auto ordered_sha = onecad::hashing::sha256_hex(ordered_xbf.data(), ordered_xbf.size());
    const auto reordered_sha = onecad::hashing::sha256_hex(reordered_xbf.data(), reordered_xbf.size());
    const auto ordered_read = onecad::io::read_xcaf_solids_bytes(ordered_xbf);
    const auto reordered_read = onecad::io::read_xcaf_solids_bytes(reordered_xbf);
    check(ordered_sha != reordered_sha && ordered_read.face_claims.size() == 12 &&
              reordered_read.face_claims.size() == 12,
          "reordered XBF roots are scoped to distinct blob bytes");
    for (const auto& claim : reordered_read.face_claims) {
        check(claim.source_sha256 == reordered_sha, "reordered XBF claim uses its own digest");
    }
    onecad::io::SolidAttributes color;
    color.face_colors.assign(6, onecad::io::kUnsetColor);
    color.face_colors[0] = 0xff0000ff;
    std::vector<std::uint8_t> colored;
    check(onecad::io::write_xcaf_document({box}, {color}, colored).empty(), "write colored XBF");
    auto colored_read = onecad::io::read_xcaf_solids_bytes(colored);
    check(colored_read.ok() && colored_read.face_claims.size() == 6, "colored XBF has six explicit labels");
    check(!colored_read.attributes.empty() && colored_read.attributes[0].face_colors == color.face_colors,
          "face colors survive alongside labels");
    {
        auto& budget = onecad::io::source_claim_budget();
        auto hold = budget.acquire(onecad::io::kSourceIdentityBudgetBytes - budget.used());
        bool complete = true;
        std::vector<std::uint8_t> fallback;
        check(hold && onecad::io::write_xcaf_document({box}, {color}, fallback, &complete).empty() && !complete,
              "writer reports omitted optional identity labels under budget exhaustion");
        const auto replay = onecad::io::read_xcaf_solids_bytes(fallback);
        check(replay.ok() && replay.solids.size() == 1 && !replay.attributes.empty() &&
              replay.attributes[0].face_colors == color.face_colors,
              "writer budget fallback preserves geometry and authored colors");
    }
    const fs::path xbf_path = tmp_root / "source.xbf";
    { std::ofstream out(xbf_path, std::ios::binary); out.write(reinterpret_cast<const char*>(colored.data()), colored.size()); }
    const std::string xbf_hash = onecad::hashing::sha256_hex(colored.data(), colored.size());
    onecad::session::BodyStore xbf_bodies;
    onecad::elementmap::ElementMapPartition xbf_partition;
    std::vector<std::pair<std::string, nlohmann::json>> xbf_sketches;
    std::string xbf_last;
    onecad::ops::OpContext xbf_ctx{xbf_bodies, &xbf_sketches, xbf_partition, &xbf_last};
    nlohmann::json xbf_params = {{"sourceSha256", xbf_hash}, {"sourceCodec", "xbf"},
                                 {"brepFormat", onecad::io::kXcafFormatVersion},
                                 {"healPolicy", "v1"}, {"unitScale", 2.0}, {"path", xbf_path.string()}};
    const auto xbf_out = onecad::ops::execute_import_step(xbf_ctx, {{"params", xbf_params}}, "00000000-0000-0000-0000-000000000002");
    check(xbf_out.status == onecad::ops::OpOutcome::Status::Ok && (xbf_out.source_face_claims ? xbf_out.source_face_claims->size() : 0) == 6,
          "XBF ImportOp carries six scaled claims");
    onecad::session::BodyStore forged_bodies;
    onecad::elementmap::ElementMapPartition forged_partition;
    onecad::ops::OpContext forged_ctx{forged_bodies, &xbf_sketches, forged_partition, &xbf_last};
    xbf_params["sourceSha256"] = std::string(64, '0');
    const auto forged = onecad::ops::execute_import_step(forged_ctx, {{"params", xbf_params}}, "forged-xbf");
    check(forged.status == onecad::ops::OpOutcome::Status::Failed && forged_bodies.size() == 0,
          "forged XBF SHA is refused before publication");
    for (const auto& claim : xbf_out.source_face_claims->items()) {
        check(claim.address_kind == onecad::io::SourceFaceClaim::AddressKind::XbfLabel &&
                  !claim.xbf_root_label.empty() && !claim.xbf_face_label.empty() &&
                  claim.source_sha256 == xbf_hash && claim.body_id == "body_00000000-0000-0000-0000-000000000002",
              "XBF claim uses actual blob hash and persisted labels");
    }
    std::vector<std::uint8_t> brep;
    check(onecad::io::write_brep_compound({box}, brep).empty(), "write BREP fixture");
    const fs::path brep_path = tmp_root / "source.brep";
    { std::ofstream out(brep_path, std::ios::binary); out.write(reinterpret_cast<const char*>(brep.data()), brep.size()); }
    onecad::session::BodyStore brep_bodies;
    onecad::elementmap::ElementMapPartition brep_partition;
    onecad::ops::OpContext brep_ctx{brep_bodies, &xbf_sketches, brep_partition, &xbf_last};
    nlohmann::json brep_params = {{"sourceSha256", onecad::hashing::sha256_hex(brep.data(), brep.size())},
                                  {"sourceCodec", "brep"}, {"brepFormat", onecad::io::kBrepFormatVersion},
                                  {"healPolicy", "v1"}, {"unitScale", 1.0}, {"path", brep_path.string()}};
    const auto brep_out = onecad::ops::execute_import_step(brep_ctx, {{"params", brep_params}}, "brep-test");
    check(brep_out.status == onecad::ops::OpOutcome::Status::Ok && (!brep_out.source_face_claims || brep_out.source_face_claims->empty()),
          "BREP geometry survives without unproven slot claims");
    check(!brep_out.diagnostics.empty(), "BREP unavailability is explicit");
    fs::remove(brep_path);
    fs::remove(xbf_path);
    fs::remove(tmp_root);
    return failures == 0 ? 0 : 1;
}
