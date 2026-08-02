// BrepCodec.cpp — see BrepCodec.h.
#include "io/BrepCodec.h"

#include <fstream>
#include <sstream>

#include <BRep_Builder.hxx>
#include <BinTools.hxx>
#include <BinTools_FormatVersion.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Iterator.hxx>

namespace onecad::io {

namespace {

// The pin, in OCCT's own type. `static_assert`ed against the wire constant so a
// future OCCT enum renumbering cannot silently change what `brepFormat` means.
constexpr BinTools_FormatVersion kWriteVersion = BinTools_FormatVersion_VERSION_4;
static_assert(static_cast<int>(kWriteVersion) == kBrepFormatVersion,
              "kBrepFormatVersion must equal the BinTools version actually written");

// The exact banner `BinTools::Write` emits at kWriteVersion, verified against the
// produced bytes. See `header_error()` for why this is checked by hand.
constexpr const char* kHeader = "\nOpen CASCADE Topology V4, (c) Open Cascade\n";

// STDOUT HYGIENE (SCHEMA §2 — fd 1 is the frame channel). `BinTools_ShapeSet::Read`
// prints "File was not written with this version of the topology" to `std::cout`
// DIRECTLY — not through `Message_Messenger` — so neither main.cpp's cerr redirect
// nor `io::OcctMessengerGuard` can intercept it, and in the real worker those bytes
// would land inside an OCW1 frame. The only defence available at this layer (a
// `std::cout` rdbuf swap is banned by scripts/check-worker-stdout-hygiene.sh, and an
// fd-level dup2 would swallow the concurrent solver lane's frames — see
// OcctStaticGuard.h) is to REJECT bytes OCCT would complain about before handing
// them over. Returns "" when the header is the one we write.
std::string header_error(const std::string& path) {
    std::ifstream in(path, std::ios::binary);
    if (!in) return "brep file is not readable: " + path;
    std::string head(std::char_traits<char>::length(kHeader), '\0');
    in.read(head.data(), static_cast<std::streamsize>(head.size()));
    if (in.gcount() != static_cast<std::streamsize>(head.size()) || head != kHeader) {
        return "brep bytes do not carry the BinTools v" + std::to_string(kBrepFormatVersion) +
               " header (not a brep blob written by this worker)";
    }
    return "";
}

}  // namespace

std::string write_brep_compound(const std::vector<TopoDS_Shape>& solids,
                                std::vector<std::uint8_t>& bytes_out) {
    try {
        BRep_Builder builder;
        TopoDS_Compound compound;
        builder.MakeCompound(compound);
        for (const TopoDS_Shape& s : solids) {
            if (!s.IsNull()) builder.Add(compound, s);
        }

        std::ostringstream os(std::ios::out | std::ios::binary);
        BinTools::Write(compound, os, /*withTriangles=*/Standard_False,
                        /*withNormals=*/Standard_False, kWriteVersion);
        const std::string blob = os.str();
        bytes_out.assign(blob.begin(), blob.end());
    } catch (const Standard_Failure& f) {
        return std::string("BrepCodec write raised: ") +
               (f.GetMessageString() != nullptr ? f.GetMessageString() : "OCCT failure");
    } catch (const std::exception& e) {
        return std::string("BrepCodec write raised: ") + e.what();
    }
    return "";
}

BrepReadResult read_brep_solids(const std::string& path) {
    BrepReadResult out;
    out.error = header_error(path);
    if (!out.error.empty()) return out;

    TopoDS_Shape shape;
    try {
        if (BinTools::Read(shape, path.c_str()) != Standard_True) {
            out.error = "brep bytes are not readable as BinTools: " + path;
            return out;
        }
    } catch (const Standard_Failure& f) {
        out.error = std::string("brep read raised: ") +
                    (f.GetMessageString() != nullptr ? f.GetMessageString() : "OCCT failure");
        return out;
    } catch (const std::exception& e) {
        out.error = std::string("brep read raised: ") + e.what();
        return out;
    }

    if (shape.IsNull()) {
        out.error = "brep bytes decoded to a null shape: " + path;
        return out;
    }

    // Stored order, verbatim. A bare solid is accepted so a producer that skipped
    // the compound wrapper still replays; anything else is malformed for this lane.
    if (shape.ShapeType() == TopAbs_SOLID) {
        out.solids.push_back(shape);
        return out;
    }
    if (shape.ShapeType() != TopAbs_COMPOUND) {
        out.error = "brep top-level shape is neither a solid nor a compound";
        return out;
    }
    for (TopoDS_Iterator it(shape); it.More(); it.Next()) {
        if (it.Value().ShapeType() != TopAbs_SOLID) {
            out.error = "brep compound carries a non-solid child";
            out.solids.clear();
            return out;
        }
        out.solids.push_back(it.Value());
    }
    if (out.solids.empty()) out.error = "brep compound carries no solid";
    return out;
}

}  // namespace onecad::io
