// ExportGeometry.cpp — see ExportGeometry.h.
#include "io/ExportGeometry.h"

#include <cstdint>
#include <fstream>
#include <string>
#include <vector>

#include <TopAbs_ShapeEnum.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS_Shape.hxx>

#include "io/BrepCodec.h"
#include "io/XcafCodec.h"
#include "session/BodyStore.h"
#include "util/Log.h"

namespace onecad::io {

using nlohmann::json;
using protocol::Envelope;

namespace {

std::string get_str(const json& o, const char* key, const std::string& dflt = "") {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return dflt;
}

Envelope fail(std::uint64_t id, const std::string& message) {
    return Envelope::error_response(id,
                                    protocol::ErrorInfo{"OP_FAILED", message, /*retriable=*/false});
}

}  // namespace

Envelope handle_export_geometry(session::Session& session, const Envelope& req) {
    const json& args = req.args;
    const std::string path = get_str(args, "path");
    if (path.empty()) {
        return fail(req.id, "ExportGeometry: empty path");
    }
    // No default: the caller pins the byte form it is going to record as
    // `ImportStep.sourceCodec`, so silently picking one here would let a record
    // claim a codec the bytes are not in.
    const std::string codec = get_str(args, "codec");
    if (codec != "brep" && codec != kXcafCodecName) {
        return fail(req.id, "ExportGeometry: unknown codec '" + codec + "' (known: brep, " +
                                kXcafCodecName + ")");
    }

    const session::BodyStore bodies = session.bodies_copy();
    std::vector<std::string> which;
    if (args.contains("bodyIds") && args["bodyIds"].is_array()) {
        for (const auto& b : args["bodyIds"])
            if (b.is_string()) which.push_back(b.get<std::string>());
    }
    if (which.empty()) {
        return fail(req.id, "ExportGeometry: bodyIds must name at least one body");
    }

    std::vector<TopoDS_Shape> solids;
    std::vector<SolidAttributes> attrs;
    solids.reserve(which.size());
    attrs.reserve(which.size());
    for (const std::string& bid : which) {
        const session::BodyRecord* rec = bodies.get(bid);
        if (rec == nullptr || rec->geom.IsNull()) {
            return fail(req.id, "ExportGeometry: no live body " + bid);
        }
        // A published body is SOLID-LIKE, not necessarily a bare `TopAbs_SOLID`:
        // `single_solid_policy` (ShapeAudit.cpp) admits a compound wrapper, and a
        // fused feature routinely produces one. Both replay readers reject a
        // non-solid child, so flatten to the contained solids here — and refuse a
        // body that contains none, where the message can still name it, rather
        // than at the far end of a save/reopen round trip.
        std::size_t before = solids.size();
        for (TopExp_Explorer e(rec->geom, TopAbs_SOLID); e.More(); e.Next()) {
            solids.push_back(e.Current());
        }
        if (solids.size() == before) {
            return fail(req.id, "ExportGeometry: body " + bid +
                                    " carries no solid (this lane writes the §7.3 replay form, "
                                    "whose readers accept solids only)");
        }
        // Only the xbf lane can carry appearance; `write_brep_compound` ignores
        // it. Colors are indexed by the BODY's own face map, which coincides with
        // the solid's only when the body flattened to exactly one — a multi-solid
        // body would need the same `ModifiedShape` remap `ImportOp::scale_solids`
        // does, and guessing an alignment would paint the wrong faces. So colors
        // ride the single-solid case (what a component actually is, spec §9) and
        // are dropped, not misapplied, otherwise.
        const bool one_solid = solids.size() == before + 1;
        for (std::size_t k = before; k < solids.size(); ++k) {
            SolidAttributes a;
            if (one_solid) a.face_colors = rec->face_colors;
            attrs.push_back(std::move(a));
        }
    }

    std::vector<std::uint8_t> bytes;
    int format = 0;
    if (codec == "brep") {
        const std::string err = write_brep_compound(solids, bytes);
        if (!err.empty()) return fail(req.id, "ExportGeometry: " + err);
        format = kBrepFormatVersion;
    } else {
        const std::string err = write_xcaf_document(solids, attrs, bytes);
        if (!err.empty()) return fail(req.id, "ExportGeometry: " + err);
        format = kXcafFormatVersion;
    }

    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    if (!out) {
        return fail(req.id, "ExportGeometry: cannot open " + path + " for writing");
    }
    out.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
    out.close();
    if (!out) {
        return fail(req.id, "ExportGeometry: write to " + path + " failed");
    }

    WLOG_INFO("ExportGeometry: %zu solid(s) as %s v%d, %zu bytes", solids.size(), codec.c_str(),
              format, bytes.size());
    return Envelope::ok_response(req.id, json{
                                             {"written", true},
                                             {"bytes", static_cast<std::uint64_t>(bytes.size())},
                                             {"codec", codec},
                                             {"format", format},
                                             {"solidCount", static_cast<std::uint64_t>(solids.size())},
                                         });
}

}  // namespace onecad::io
