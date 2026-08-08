// One-way persistence gate: artifacts produced by OCCT 7.9.3 must reopen under
// the shipping OCCT 8.0.1 worker. No reverse-compatibility promise is made.
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>
#include <vector>

#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GProp_GProps.hxx>
#include <TopoDS_Shape.hxx>

#include "io/BrepCodec.h"
#include "io/XcafCodec.h"

namespace {

int failures = 0;

void check(bool condition, const std::string& message) {
    if (condition) return;
    std::fprintf(stderr, "FAIL: %s\n", message.c_str());
    ++failures;
}

double volume(const TopoDS_Shape& shape) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props);
    return props.Mass();
}

void write_bytes(const std::filesystem::path& path, const std::vector<std::uint8_t>& bytes) {
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    out.write(reinterpret_cast<const char*>(bytes.data()),
              static_cast<std::streamsize>(bytes.size()));
    check(out.good(), "write " + path.string());
}

void produce(const std::filesystem::path& directory) {
    std::filesystem::create_directories(directory);
    const TopoDS_Shape shape = BRepPrimAPI_MakeBox(10.0, 20.0, 30.0).Shape();
    const std::vector<TopoDS_Shape> solids{shape};
    const std::vector<onecad::io::SolidAttributes> attrs{{"onecad-persistence-box", {}}};

    std::vector<std::uint8_t> brep;
    const std::string brep_error = onecad::io::write_brep_compound(solids, brep);
    check(brep_error.empty(), "produce brep: " + brep_error);
    if (brep_error.empty()) write_bytes(directory / "occt-7.9.3.brep", brep);

    std::vector<std::uint8_t> xbf;
    const std::string xbf_error = onecad::io::write_xcaf_document(solids, attrs, xbf);
    check(xbf_error.empty(), "produce xbf: " + xbf_error);
    if (xbf_error.empty()) write_bytes(directory / "occt-7.9.3.xbf", xbf);
}

void check_shape(const TopoDS_Shape& shape, const std::string& codec) {
    check(!shape.IsNull(), codec + ": non-null");
    check(BRepCheck_Analyzer(shape).IsValid(), codec + ": BRep valid");
    check(std::abs(volume(shape) - 6000.0) <= 1e-7, codec + ": volume preserved");
}

void consume(const std::filesystem::path& directory) {
    const auto brep = onecad::io::read_brep_solids((directory / "occt-7.9.3.brep").string());
    check(brep.ok(), "consume brep: " + brep.error);
    check(brep.solids.size() == 1, "consume brep: one solid");
    if (brep.solids.size() == 1) check_shape(brep.solids.front(), "brep");

    const auto xbf = onecad::io::read_xcaf_solids((directory / "occt-7.9.3.xbf").string());
    check(xbf.ok(), "consume xbf: " + xbf.error);
    check(xbf.solids.size() == 1, "consume xbf: one solid");
    check(xbf.attributes.size() == 1, "consume xbf: one attribute record");
    if (xbf.solids.size() == 1) check_shape(xbf.solids.front(), "xbf");
    if (xbf.attributes.size() == 1) {
        check(xbf.attributes.front().name == "onecad-persistence-box",
              "consume xbf: product name preserved");
    }
}

}  // namespace

int main(int argc, char** argv) {
    if (argc != 3) {
        std::fprintf(stderr, "usage: test_occt_persistence <produce|consume> <directory>\n");
        return 2;
    }
    const std::string mode = argv[1];
    if (mode == "produce") {
        produce(argv[2]);
    } else if (mode == "consume") {
        consume(argv[2]);
    } else {
        std::fprintf(stderr, "unknown mode: %s\n", mode.c_str());
        return 2;
    }
    if (failures == 0) std::fprintf(stderr, "occt_persistence %s: OK\n", mode.c_str());
    return failures;
}
