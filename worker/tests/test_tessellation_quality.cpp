// test_tessellation_quality.cpp — display LOD contract for curved OCCT bodies.
//
// Fine meshes are the committed viewport/export representation. Keep a concrete
// cylinder probe here so a future tier retune cannot regress it to visibly flat
// round surfaces or restore the old fixed 16-span selected-edge outlines.
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepPrimAPI_MakeCylinder.hxx>
#include <TopoDS_Shape.hxx>

#include "tess/Tessellate.h"

namespace {

int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

std::uint16_t u16(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    return static_cast<std::uint16_t>(bytes[offset] | (bytes[offset + 1] << 8));
}

std::uint32_t u32(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    std::uint32_t value = 0;
    for (int i = 0; i < 4; ++i) {
        value |= static_cast<std::uint32_t>(bytes[offset + i]) << (i * 8);
    }
    return value;
}

struct Section {
    std::uint32_t type = 0;
    std::uint32_t offset = 0;
    std::uint32_t length = 0;
};

Section find_section(const std::vector<std::uint8_t>& bytes, std::uint32_t type) {
    if (bytes.size() < 64) return {};
    const std::uint16_t count = u16(bytes, 0x1E);
    if (bytes.size() < 64U + static_cast<std::size_t>(count) * 16U) return {};
    for (std::uint16_t i = 0; i < count; ++i) {
        const std::size_t entry = 64U + static_cast<std::size_t>(i) * 16U;
        if (u32(bytes, entry) == type) {
            return {type, u32(bytes, entry + 8), u32(bytes, entry + 12)};
        }
    }
    return {};
}

std::uint32_t greatest_edge_point_count(const std::vector<std::uint8_t>& bytes) {
    constexpr std::uint32_t kEdgeRanges = 8;
    const Section ranges = find_section(bytes, kEdgeRanges);
    if (ranges.type == 0 || ranges.length % 8 != 0 ||
        static_cast<std::size_t>(ranges.offset) + ranges.length > bytes.size()) {
        return 0;
    }

    std::uint32_t maximum = 0;
    for (std::uint32_t offset = 0; offset < ranges.length; offset += 8) {
        maximum = std::max(maximum, u32(bytes, ranges.offset + offset + 4));
    }
    return maximum;
}

void test_fine_cylinder_is_materially_denser_than_coarse() {
    const TopoDS_Shape coarse_cylinder = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
    const TopoDS_Shape fine_cylinder = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
    const onecad::tess::BodyMesh coarse = onecad::tess::tessellate_body(
        coarse_cylinder, "body_coarse", "coarse", true, nullptr);
    const onecad::tess::BodyMesh fine = onecad::tess::tessellate_body(
        fine_cylinder, "body_fine", "fine", true, nullptr);

    check(coarse.ok && fine.ok, "cylinder tessellation succeeds for coarse and fine");
    check(fine.triangle_count >= coarse.triangle_count * 3,
          "fine cylinder has at least 3x the coarse triangle density");

    const std::uint32_t coarse_edge_points = greatest_edge_point_count(coarse.blob);
    const std::uint32_t fine_edge_points = greatest_edge_point_count(fine.blob);
    check(fine_edge_points > 16,
          "fine circular edge exceeds the retired fixed 16-span sampling");
    check(fine_edge_points >= coarse_edge_points * 3,
          "fine circular edge density tracks the finer face LOD");
}

}  // namespace

int main() {
    test_fine_cylinder_is_materially_denser_than_coarse();
    if (g_failures == 0) std::fprintf(stderr, "tessellation_quality: OK\n");
    return g_failures;
}
