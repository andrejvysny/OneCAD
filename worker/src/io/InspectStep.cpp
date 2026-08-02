// InspectStep.cpp — see InspectStep.h.
#include "io/InspectStep.h"

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

#include <BRepBndLib.hxx>
#include <Bnd_Box.hxx>
#include <TopoDS_Shape.hxx>

#include "io/StepRead.h"
#include "io/XcafCodec.h"
#include "io/XcafRead.h"
#include "nlohmann/json.hpp"
#include "util/Log.h"

namespace onecad::io {

using nlohmann::json;
using protocol::Envelope;

namespace {

Envelope fail(std::uint64_t id, const std::string& message) {
    return Envelope::error_response(id,
                                    protocol::ErrorInfo{"OP_FAILED", message, /*retriable=*/false});
}

// Axis-aligned bounds over every solid. An empty read yields an all-zero box
// rather than an absent field, so the result shape does not depend on the file.
json bbox_json(const std::vector<TopoDS_Shape>& solids) {
    if (solids.empty()) {
        return json{{"min", json::array({0.0, 0.0, 0.0})}, {"max", json::array({0.0, 0.0, 0.0})}};
    }
    Bnd_Box box;
    for (const TopoDS_Shape& s : solids) BRepBndLib::Add(s, box);
    if (box.IsVoid()) {
        return json{{"min", json::array({0.0, 0.0, 0.0})}, {"max", json::array({0.0, 0.0, 0.0})}};
    }
    double xmin = 0, ymin = 0, zmin = 0, xmax = 0, ymax = 0, zmax = 0;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    return json{{"min", json::array({xmin, ymin, zmin})},
                {"max", json::array({xmax, ymax, zmax})}};
}

// Read diagnostics are advisory findings about the geometry, so they carry
// severity "warning" — the probe itself SUCCEEDED (a failure is an error resp).
json diagnostics_json(const std::vector<StepReadDiagnostic>& diags) {
    json out = json::array();
    for (const StepReadDiagnostic& d : diags) {
        out.push_back(json{{"severity", "warning"}, {"code", d.code}, {"message", d.message}});
    }
    return out;
}

// One entry per ordinal solid, "" where no product name was recoverable. The
// LENGTH is the contract: Rust discards the whole list when it does not equal the
// number of minted bodies, so a short "best effort" list would silently name
// nothing (see `document_runtime::apply_import_body_names`).
json product_names_json(const BoundAttributes& bound) {
    json out = json::array();
    for (const SolidAttributes& a : bound.per_solid) out.push_back(a.name);
    return out;
}

}  // namespace

Envelope handle_inspect_step(const Envelope& req, const onecad::CancelToken& cancel) {
    const json& args = req.args;
    const std::string path =
        (args.is_object() && args.contains("path") && args["path"].is_string())
            ? args["path"].get<std::string>()
            : std::string{};
    if (path.empty()) return fail(req.id, "InspectStep: empty path");

    std::error_code ec;
    if (!std::filesystem::is_regular_file(path, ec)) {
        return fail(req.id, "InspectStep: path is not a readable file: " + path);
    }
    const bool include_geometry = args.is_object() && args.contains("includeGeometry") &&
                                  args["includeGeometry"].is_boolean() &&
                                  args["includeGeometry"].get<bool>();

    const StepReadResult read = read_step(path, StepReadPolicy{}, &cancel);
    if (read.cancelled) {
        return Envelope::error_response(
            req.id, protocol::ErrorInfo{"CANCELLED", "InspectStep cancelled", /*retriable=*/false});
    }
    if (!read.ok()) return fail(req.id, "InspectStep: " + *read.error);

    // The XCAF attribute pass. ADVISORY: it runs unconditionally (so `productNames`
    // does not depend on whether the caller also asked for the bytes) and a failure
    // degrades to unnamed/uncolored solids rather than failing the probe.
    const StepAttributes attrs = read_step_attributes(path, StepReadPolicy{}, &cancel);
    if (cancel.cancelled()) {
        return Envelope::error_response(
            req.id, protocol::ErrorInfo{"CANCELLED", "InspectStep cancelled", /*retriable=*/false});
    }
    if (!attrs.error.empty()) {
        WLOG_WARN("InspectStep: XCAF attribute pass unavailable (%s); names/colors dropped",
                  attrs.error.c_str());
    }
    const BoundAttributes bound = bind_attributes(read.solids, attrs);
    WLOG_INFO("InspectStep: %zu solid(s), names %zu/%zu, face colors %zu/%zu",
              read.solids.size(), bound.names_matched, read.solids.size(), bound.faces_matched,
              bound.faces_total);

    json result = {
        {"solidCount", static_cast<std::uint64_t>(read.solids.size())},
        {"sourceUnit", read.source_unit},
        {"bbox", bbox_json(read.solids)},
        // PER ORDINAL SOLID, always `solidCount` long (empty string where no name is
        // recoverable) — Rust zips it against the minted bodies by ordinal.
        {"productNames", product_names_json(bound)},
        // Reported unconditionally: the codec + version a document should pin in
        // §7.3 `ImportStep.sourceCodec` / `brepFormat`, which the caller needs even
        // when it defers fetching the bytes.
        {"geometryCodec", kXcafCodecName},
        {"geometryFormat", kXcafFormatVersion},
        {"diagnostics", diagnostics_json(read.diagnostics)},
    };

    Envelope resp = Envelope::ok_response(req.id, std::move(result));
    if (!include_geometry) return resp;

    // The conversion lane. `read.solids` is ALREADY in ordinal order (W0's
    // `ops::ordered_solids`), and that order becomes the xbf document's free-shape
    // order — the single point where the §7.3 ordinals are baked into the bytes.
    std::vector<std::uint8_t> blob;
    const std::string err = write_xcaf_document(read.solids, bound.per_solid, blob);
    if (!err.empty()) return fail(req.id, "InspectStep: " + err);
    resp.bin.push_back(protocol::BinSection{"geometry", /*off=*/0, blob.size()});
    resp.out_bin = std::move(blob);
    return resp;
}

}  // namespace onecad::io
