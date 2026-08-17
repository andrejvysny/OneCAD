// ExportStep.cpp — see ExportStep.h.
#include "io/ExportStep.h"

#include <algorithm>
#include <cstdint>
#include <filesystem>
#include <mutex>
#include <string>
#include <vector>

#include <IFSelect_ReturnStatus.hxx>
#include <STEPCAFControl_Writer.hxx>
#include <STEPControl_Controller.hxx>
#include <STEPControl_StepModelType.hxx>
#include <Standard_Failure.hxx>
#include <TCollection_ExtendedString.hxx>
#include <TDF_Label.hxx>
#include <TDataStd_Name.hxx>
#include <TDocStd_Document.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <XCAFDoc_ColorType.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>

#include "elementmap/ElementMapPartition.h"
#include "io/OcafApp.h"
#include "io/OcctStaticGuard.h"
#include "io/XcafCodec.h"
#include "session/BodyStore.h"
#include "util/Log.h"

namespace onecad::io {

namespace em = onecad::elementmap;

using nlohmann::json;
using protocol::Envelope;

namespace {

std::string get_str(const json& o, const char* key, const std::string& dflt = "") {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return dflt;
}

// The `bodyNames` / `bodyColors` / `faceColors` sub-objects (SCHEMA §7.8). A field
// that is absent or not an object reads as "not supplied" ⇒ pre-DI-5 behaviour.
const json* object_at(const json& args, const char* key) {
    if (!args.is_object()) return nullptr;
    const auto it = args.find(key);
    if (it == args.end() || !it->is_object()) return nullptr;
    return &*it;
}

// `[r,g,b,a]` u8 sRGB → the packed `io::PackedColor` byte order. Anything that is
// not four integers in 0..255 reads as UNSET and is dropped, never written as a
// partially-decoded colour. (`is_number_integer`, not `is_number_unsigned`: a JSON
// integer's signedness is a property of how the producer's encoder happened to
// store it, not of the value.)
//
// Alpha 0 is the "no authored colour" sentinel the whole appearance lane shares
// (XcafCodec.h, mesh_format.md §4), so `[r,g,b,0]` is also unset — the one shape a
// caller could send that means "leave this face alone".
PackedColor packed_from_json(const json& value) {
    if (!value.is_array() || value.size() != 4) return kUnsetColor;
    std::uint32_t packed = 0;
    for (const json& channel : value) {
        if (!channel.is_number_integer()) return kUnsetColor;
        const std::int64_t n = channel.get<std::int64_t>();
        if (n < 0 || n > 255) return kUnsetColor;
        packed = (packed << 8) | static_cast<std::uint32_t>(n);
    }
    return (packed & 0xffu) == 0u ? kUnsetColor : packed;
}

// The colour every face of `rec` should be written with, in `TopExp::MapShapes`
// order (index 0 == TopoKey "f:1").
//
// Two layers, in this order:
//  1. the body's OWN import-derived colours (`BodyRecord::face_colors`, populated by
//     the ImportStep op) — a file that came in coloured leaves coloured;
//  2. the wire-sent AUTHORED table, keyed by TopoKey — the user painted over it, so
//     it wins.
//
// A TopoKey that does not address a face of THIS body is DROPPED and counted, never
// nudged onto a neighbour: Rust resolved every key against the current snapshot, so
// a miss here means the snapshot moved under the export, and painting the user's
// colour onto whichever face happens to hold that ordinal is exactly the silent
// mis-bind (H5-B) this stack refuses to make.
std::vector<PackedColor> effective_face_colors(const session::BodyRecord& rec,
                                               const json* authored,
                                               const TopTools_IndexedMapOfShape& faces,
                                               std::size_t& unresolved) {
    std::vector<PackedColor> out(static_cast<std::size_t>(faces.Extent()), kUnsetColor);
    const std::size_t imported = std::min(rec.face_colors.size(), out.size());
    for (std::size_t i = 0; i < imported; ++i) out[i] = rec.face_colors[i];
    if (authored == nullptr) return out;

    for (const auto& entry : authored->items()) {
        const PackedColor colour = packed_from_json(entry.value());
        if (colour == kUnsetColor) {
            ++unresolved;
            continue;
        }
        const TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(rec.geom, entry.key());
        const int index = sub.IsNull() ? 0 : faces.FindIndex(sub);
        if (index < 1) {
            ++unresolved;
            WLOG_WARN("export_step body=%s topoKey=%s outcome=unaddressable (colour dropped)",
                      rec.id.c_str(), entry.key().c_str());
            continue;
        }
        out[static_cast<std::size_t>(index - 1)] = colour;
    }
    return out;
}

}  // namespace

Envelope handle_export_step(session::Session& session, const Envelope& req) {
    const json& args = req.args;
    const std::string path = get_str(args, "path");
    if (path.empty()) {
        return Envelope::error_response(
            req.id, protocol::ErrorInfo{"OP_FAILED", "ExportStep: empty path", /*retriable=*/false});
    }
    // SCHEMA §7.8; forwarded to OCCT's write.step.schema knob, which the guard below
    // restores — un-guarded it leaked process-wide and silently re-schema'd every
    // later STEP write in this worker.
    const std::string schema = get_str(args, "schema", "AP214IS");
    const json* names = object_at(args, "bodyNames");
    const json* body_colors = object_at(args, "bodyColors");
    const json* face_colors = object_at(args, "faceColors");

    const session::BodyStore bodies = session.bodies_copy();
    std::vector<std::string> which;
    if (args.contains("bodyIds") && args["bodyIds"].is_array()) {
        for (const auto& b : args["bodyIds"])
            if (b.is_string()) which.push_back(b.get<std::string>());
    } else {
        which = bodies.ids();  // "all"
    }

    // Same prologue as the read lane: register the norm + its knobs BEFORE the guard
    // snapshots them, and keep OCCT chatter off fd 1 (SCHEMA §2 — fd 1 carries OCW1
    // frames and nothing else).
    STEPControl_Controller::Init();
    OcctMessengerGuard quiet;
    InterfaceStaticGuard knobs;

    const std::lock_guard<std::mutex> lock(ocaf_mutex());
    std::size_t transferred = 0;
    std::size_t named = 0;
    std::size_t coloured_faces = 0;
    std::size_t unresolved = 0;
    try {
        if (!knobs.set_cstr("write.step.schema", schema)) {
            return Envelope::error_response(
                req.id, protocol::ErrorInfo{"OP_FAILED",
                                            "ExportStep: OCCT did not accept write.step.schema=" +
                                                schema,
                                            /*retriable=*/false});
        }

        Handle(TDocStd_Document) doc;
        ocaf_application()->NewDocument("BinXCAF", doc);
        if (doc.IsNull()) {
            return Envelope::error_response(
                req.id, protocol::ErrorInfo{"OP_FAILED",
                                            "ExportStep: could not create an XCAF document",
                                            /*retriable=*/false});
        }
        OcafDocGuard doc_guard(doc);
        Handle(XCAFDoc_ShapeTool) shapes = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
        Handle(XCAFDoc_ColorTool) colours = XCAFDoc_DocumentTool::ColorTool(doc->Main());

        std::vector<TDF_Label> labelled;
        for (const std::string& bid : which) {
            const session::BodyRecord* rec = bodies.get(bid);
            if (rec == nullptr || rec->geom.IsNull()) continue;
            const TDF_Label label = shapes->AddShape(rec->geom, Standard_False);
            if (label.IsNull()) {
                return Envelope::error_response(
                    req.id,
                    protocol::ErrorInfo{"OP_FAILED", "ExportStep: AddShape produced no label for " + bid,
                                        /*retriable=*/false});
            }
            ++transferred;

            // `AddShape` REUSES the label of a shape the tool already holds (`IsSame`
            // semantics), so two bodies over one TShape land on ONE label and the
            // second body's name/colours would silently overwrite the first's. The
            // geometry is already in the document; the ATTRIBUTES are what must not
            // be guessed, so they are skipped and reported.
            if (std::any_of(labelled.begin(), labelled.end(),
                            [&label](const TDF_Label& seen) { return seen.IsEqual(label); })) {
                WLOG_WARN("export_step body=%s outcome=label_shared (name/colours skipped)",
                          bid.c_str());
                continue;
            }
            labelled.push_back(label);

            const std::string name =
                names != nullptr ? get_str(*names, bid.c_str()) : std::string();
            if (!name.empty()) {
                TDataStd_Name::Set(label, TCollection_ExtendedString(name.c_str(), Standard_True));
                ++named;
            }

            // On the body's OWN label with `ColorSurf` — precisely what XcafRead's
            // inherited-colour pass reads back as "the whole part is this colour", so
            // a re-import reproduces it. Per-face colours below still win on read.
            const PackedColor whole =
                body_colors != nullptr && body_colors->contains(bid)
                    ? packed_from_json((*body_colors)[bid])
                    : kUnsetColor;
            if (whole != kUnsetColor) {
                colours->SetColor(label, unpack_srgb(whole), XCAFDoc_ColorSurf);
            }

            TopTools_IndexedMapOfShape faces;
            TopExp::MapShapes(rec->geom, TopAbs_FACE, faces);
            const json* authored = nullptr;
            if (face_colors != nullptr) {
                const auto it = face_colors->find(bid);
                if (it != face_colors->end() && it->is_object()) authored = &*it;
            }
            const std::vector<PackedColor> effective =
                effective_face_colors(*rec, authored, faces, unresolved);
            for (int i = 1; i <= faces.Extent(); ++i) {
                const PackedColor colour = effective[static_cast<std::size_t>(i - 1)];
                if (colour == kUnsetColor) continue;
                const TDF_Label sub = shapes->AddSubShape(label, faces(i));
                if (sub.IsNull()) {
                    ++unresolved;  // face not addressable in XCAF — colour dropped
                    continue;
                }
                colours->SetColor(sub, unpack_srgb(colour), XCAFDoc_ColorSurf);
                ++coloured_faces;
            }
        }
        if (transferred == 0) {
            return Envelope::error_response(
                req.id,
                protocol::ErrorInfo{"OP_FAILED", "ExportStep: no bodies to export", /*retriable=*/false});
        }

        STEPCAFControl_Writer writer;
        writer.SetColorMode(Standard_True);
        writer.SetNameMode(Standard_True);
        writer.SetLayerMode(Standard_False);
        writer.SetPropsMode(Standard_False);
        if (!writer.Transfer(doc, STEPControl_AsIs)) {
            return Envelope::error_response(
                req.id, protocol::ErrorInfo{"OP_FAILED", "ExportStep: XCAF transfer failed",
                                            /*retriable=*/false});
        }
        const IFSelect_ReturnStatus wst = writer.Write(path.c_str());
        if (wst != IFSelect_RetDone) {
            return Envelope::error_response(
                req.id, protocol::ErrorInfo{"OP_FAILED", "ExportStep: write failed", /*retriable=*/false});
        }
    } catch (const Standard_Failure& f) {
        return Envelope::error_response(
            req.id, protocol::ErrorInfo{"OP_FAILED",
                                        std::string("ExportStep raised: ") +
                                            (f.GetMessageString() ? f.GetMessageString() : "OCCT"),
                                        /*retriable=*/false});
    }

    std::error_code ec;
    const std::uintmax_t bytes = std::filesystem::file_size(path, ec);
    return Envelope::ok_response(req.id,
                                 json{{"written", true},
                                      {"bytes", ec ? 0 : static_cast<std::uint64_t>(bytes)},
                                      {"namedBodies", named},
                                      {"coloredFaces", coloured_faces},
                                      {"unresolvedFaceColors", unresolved}});
}

}  // namespace onecad::io
