// XcafCodec.cpp — see XcafCodec.h.
#include "io/XcafCodec.h"

#include "io/OcafApp.h"

#include <algorithm>
#include <fstream>
#include <mutex>
#include <sstream>

#include <PCDM_ReaderStatus.hxx>
#include <PCDM_StoreStatus.hxx>
#include <Quantity_Color.hxx>
#include <Quantity_ColorRGBA.hxx>
#include <Standard_Failure.hxx>
#include <TCollection_AsciiString.hxx>
#include <TCollection_ExtendedString.hxx>
#include <TDF_Label.hxx>
#include <TDF_LabelSequence.hxx>
#include <TDataStd_Name.hxx>
#include <TDocStd_Application.hxx>
#include <TDocStd_Document.hxx>
#include <TDocStd_FormatVersion.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <XCAFDoc_ColorType.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>

namespace onecad::io {

namespace {

// The OCAF binary-file magic every BinXCAF blob starts with.
//
// Same posture as BrepCodec's banner check: reject bytes OCCT would complain about
// before handing them over, so "some unrelated file" becomes a precise message
// instead of an OCCT status code. The STDOUT half of that job (SCHEMA §2 — fd 1 is
// the frame channel) is done by `io::ocaf_application()`, which re-points the OCAF
// application's OWN messenger at stderr; see OcafApp.h for why main.cpp's redirect
// does not cover this path.
constexpr const char* kMagic = "BINFILE";

// The OCAF storage format this worker reads and writes. `BinXCAFDrivers` binds it
// to the binary XDE drivers; the string is part of the on-disk header.
constexpr const char* kStorageFormat = "BinXCAF";

// The pin, in OCCT's own type — `static_assert`ed against the wire constant so a
// future OCCT enum renumbering cannot silently change what `geometryFormat` means
// (same guard BrepCodec puts on `kBrepFormatVersion`).
constexpr TDocStd_FormatVersion kWriteVersion = TDocStd_FormatVersion_VERSION_12;
static_assert(static_cast<int>(kWriteVersion) == kXcafFormatVersion,
              "kXcafFormatVersion must equal the OCAF storage version actually written");

std::string magic_error(const std::string& path, std::string& bytes_out) {
    std::ifstream in(path, std::ios::binary);
    if (!in) return "xbf file is not readable: " + path;
    std::ostringstream buf;
    buf << in.rdbuf();
    bytes_out = buf.str();
    const std::size_t magic_len = std::char_traits<char>::length(kMagic);
    if (bytes_out.size() < magic_len || bytes_out.compare(0, magic_len, kMagic) != 0) {
        return std::string("xbf bytes do not carry the OCAF `") + kMagic +
               "` magic (not a BinXCAF blob written by this worker)";
    }
    return "";
}

std::string label_name(const TDF_Label& label) {
    Handle(TDataStd_Name) attr;
    if (!label.FindAttribute(TDataStd_Name::GetID(), attr)) return {};
    return TCollection_AsciiString(attr->Get()).ToCString();
}

// A color set on the SHAPE'S OWN label rather than on any face. XCAF does NOT
// resolve this downward — `GetColor(face, …)` returns false for a face whose only
// color lives on the enclosing solid's label — so the fallback is explicit.
bool label_color(const Handle(XCAFDoc_ColorTool) & colors, const TDF_Label& label,
                 PackedColor& out) {
    Quantity_ColorRGBA color;
    if (XCAFDoc_ColorTool::GetColor(label, XCAFDoc_ColorSurf, color) ||
        XCAFDoc_ColorTool::GetColor(label, XCAFDoc_ColorGen, color)) {
        out = pack_srgb(color);
        return true;
    }
    (void)colors;
    return false;
}

// Face colors of `solid`, in `TopExp::MapShapes` order. Returns an empty vector
// when no face carries one, so a color-less body costs no bytes downstream
// (mesh_format.md: the section is absent and the flag bit stays clear).
std::vector<PackedColor> read_face_colors(const Handle(XCAFDoc_ColorTool) & colors,
                                          const TDF_Label& label, const TopoDS_Shape& solid) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(solid, TopAbs_FACE, faces);
    std::vector<PackedColor> out(static_cast<std::size_t>(faces.Extent()), kUnsetColor);
    PackedColor whole = kUnsetColor;
    const bool has_whole = label_color(colors, label, whole);
    bool any = has_whole;
    for (int i = 1; i <= faces.Extent(); ++i) {
        Quantity_ColorRGBA color;
        // `ColorSurf` is the surface appearance STEP `surface_style_usage` maps to;
        // `ColorGen` is the generic fallback a producer may have used instead.
        if (colors->GetColor(faces(i), XCAFDoc_ColorSurf, color) ||
            colors->GetColor(faces(i), XCAFDoc_ColorGen, color)) {
            out[static_cast<std::size_t>(i - 1)] = pack_srgb(color);
            any = true;
        } else if (has_whole) {
            out[static_cast<std::size_t>(i - 1)] = whole;
        }
    }
    if (!any) out.clear();
    return out;
}

}  // namespace

PackedColor pack_srgb(const Quantity_ColorRGBA& color) {
    const auto to_u8 = [](float linear) {
        const float srgb = Quantity_Color::Convert_LinearRGB_To_sRGB(linear);
        const float clamped = std::clamp(srgb, 0.0f, 1.0f);
        return static_cast<std::uint32_t>(clamped * 255.0f + 0.5f);
    };
    const Quantity_Color& rgb = color.GetRGB();
    // Alpha is NOT sRGB-encoded (it is a linear coverage term), and alpha 0 is the
    // "unset" sentinel — an authored fully transparent face would be
    // indistinguishable from no color, so it is clamped to 1 rather than lost.
    const float alpha = color.Alpha();
    const std::uint32_t a = alpha <= 0.0f
                                ? 255u
                                : static_cast<std::uint32_t>(std::clamp(alpha, 0.0f, 1.0f) * 255.0f +
                                                             0.5f);
    return (to_u8(static_cast<float>(rgb.Red())) << 24) |
           (to_u8(static_cast<float>(rgb.Green())) << 16) |
           (to_u8(static_cast<float>(rgb.Blue())) << 8) | a;
}

Quantity_ColorRGBA unpack_srgb(PackedColor color) {
    // Round-trip in the SAME space: the packed bytes are sRGB, and
    // `Quantity_Color(…, Quantity_TOC_sRGB)` converts back to OCCT's internal linear
    // form, so `pack_srgb` on the read side returns exactly these bytes again.
    const Quantity_Color rgb(static_cast<double>((color >> 24) & 0xff) / 255.0,
                             static_cast<double>((color >> 16) & 0xff) / 255.0,
                             static_cast<double>((color >> 8) & 0xff) / 255.0,
                             Quantity_TOC_sRGB);
    return Quantity_ColorRGBA(rgb, static_cast<float>(color & 0xff) / 255.0f);
}

std::string write_xcaf_document(const std::vector<TopoDS_Shape>& solids,
                                const std::vector<SolidAttributes>& attrs,
                                std::vector<std::uint8_t>& bytes_out) {
    const std::lock_guard<std::mutex> lock(ocaf_mutex());
    try {
        Handle(TDocStd_Document) doc;
        ocaf_application()->NewDocument(kStorageFormat, doc);
        if (doc.IsNull()) return "XcafCodec: could not create a BinXCAF document";
        OcafDocGuard guard(doc);
        doc->ChangeStorageFormatVersion(kWriteVersion);

        Handle(XCAFDoc_ShapeTool) shapes = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
        Handle(XCAFDoc_ColorTool) colors = XCAFDoc_DocumentTool::ColorTool(doc->Main());

        std::size_t added = 0;
        for (std::size_t k = 0; k < solids.size(); ++k) {
            if (solids[k].IsNull()) continue;
            ++added;
            // `makeAssembly=false`: each solid is a flat top-level shape, so the
            // free-shape sequence read back IS the ordinal list.
            const TDF_Label label = shapes->AddShape(solids[k], Standard_False);
            if (label.IsNull()) return "XcafCodec: AddShape produced no label";
            if (k >= attrs.size()) continue;
            const SolidAttributes& a = attrs[k];
            if (!a.name.empty()) {
                TDataStd_Name::Set(label, TCollection_ExtendedString(a.name.c_str()));
            }
            if (a.face_colors.empty()) continue;

            TopTools_IndexedMapOfShape faces;
            TopExp::MapShapes(solids[k], TopAbs_FACE, faces);
            for (int i = 1; i <= faces.Extent(); ++i) {
                const std::size_t idx = static_cast<std::size_t>(i - 1);
                if (idx >= a.face_colors.size() || a.face_colors[idx] == kUnsetColor) continue;
                const PackedColor c = a.face_colors[idx];
                const TDF_Label sub = shapes->AddSubShape(label, faces(i));
                if (sub.IsNull()) continue;  // face not addressable — color dropped
                colors->SetColor(sub, unpack_srgb(c), XCAFDoc_ColorSurf);
            }
        }

        // `AddShape` REUSES an existing label for a shape it already holds
        // (`FindShape`, `IsSame` semantics). Two identical solids would therefore
        // collapse to one free label and silently shift every later ordinal — the
        // exact class of identity loss this migration exists to prevent — so the
        // count is checked rather than assumed.
        TDF_LabelSequence written;
        shapes->GetFreeShapes(written);
        if (static_cast<std::size_t>(written.Length()) != added) {
            return "XcafCodec: " + std::to_string(added) + " solid(s) collapsed to " +
                   std::to_string(written.Length()) +
                   " free label(s) — the ordinal order would shift";
        }

        std::ostringstream os(std::ios::out | std::ios::binary);
        const PCDM_StoreStatus status = ocaf_application()->SaveAs(doc, os);
        if (status != PCDM_SS_OK) {
            return "XcafCodec: BinXCAF store failed (status " +
                   std::to_string(static_cast<int>(status)) + ")";
        }
        const std::string blob = os.str();
        if (blob.empty()) return "XcafCodec: BinXCAF store produced no bytes";
        bytes_out.assign(blob.begin(), blob.end());
    } catch (const Standard_Failure& f) {
        return std::string("XcafCodec write raised: ") +
               (f.GetMessageString() != nullptr ? f.GetMessageString() : "OCCT failure");
    } catch (const std::exception& e) {
        return std::string("XcafCodec write raised: ") + e.what();
    }
    return "";
}

XcafReadResult read_xcaf_solids(const std::string& path) {
    XcafReadResult out;
    std::string bytes;
    out.error = magic_error(path, bytes);
    if (!out.error.empty()) return out;

    const std::lock_guard<std::mutex> lock(ocaf_mutex());
    try {
        std::istringstream is(bytes, std::ios::in | std::ios::binary);
        Handle(TDocStd_Document) doc;
        const PCDM_ReaderStatus status = ocaf_application()->Open(is, doc);
        if (status != PCDM_RS_OK || doc.IsNull()) {
            out.error = "xbf bytes are not readable as BinXCAF (status " +
                        std::to_string(static_cast<int>(status)) + "): " + path;
            return out;
        }
        OcafDocGuard guard(doc);

        Handle(XCAFDoc_ShapeTool) shapes = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
        Handle(XCAFDoc_ColorTool) colors = XCAFDoc_DocumentTool::ColorTool(doc->Main());
        TDF_LabelSequence free_labels;
        shapes->GetFreeShapes(free_labels);

        for (int i = 1; i <= free_labels.Length(); ++i) {
            const TDF_Label label = free_labels.Value(i);
            const TopoDS_Shape shape = XCAFDoc_ShapeTool::GetShape(label);
            if (shape.IsNull()) {
                out.error = "xbf free shape " + std::to_string(i) + " is null";
                out.solids.clear();
                out.attributes.clear();
                return out;
            }
            if (shape.ShapeType() != TopAbs_SOLID) {
                out.error = "xbf free shape " + std::to_string(i) + " is not a solid";
                out.solids.clear();
                out.attributes.clear();
                return out;
            }
            SolidAttributes attrs;
            attrs.name = label_name(label);
            attrs.face_colors = read_face_colors(colors, label, shape);
            out.solids.push_back(shape);
            out.attributes.push_back(std::move(attrs));
        }
        if (out.solids.empty()) out.error = "xbf document carries no free solid";
    } catch (const Standard_Failure& f) {
        out.solids.clear();
        out.attributes.clear();
        out.error = std::string("xbf read raised: ") +
                    (f.GetMessageString() != nullptr ? f.GetMessageString() : "OCCT failure");
    } catch (const std::exception& e) {
        out.solids.clear();
        out.attributes.clear();
        out.error = std::string("xbf read raised: ") + e.what();
    }
    return out;
}

}  // namespace onecad::io
