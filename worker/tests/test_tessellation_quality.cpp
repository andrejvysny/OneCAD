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

#include "tess/Mesh1.h"
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

// MESH1 section type codes (protocol/mesh_format.md §4), mirrored from
// worker/src/tess/Mesh1.cpp's SectionType enum. Section 7 is EDGE_RANGES
// (per-edge {firstPoint, pointCount} into EDGE_POSITIONS); section 8 is
// EDGE_POSITIONS itself (f32 x 3 per point). They are easy to transpose
// because both are "edge" sections and both happen to satisfy `length % 8 == 0`
// for common meshes, which let a transposed reader pass silently.
constexpr std::uint32_t kEdgeRanges = 7;
constexpr std::uint32_t kEdgePositions = 8;

std::uint32_t greatest_edge_point_count(const std::vector<std::uint8_t>& bytes) {
    const std::uint32_t edge_count = u32(bytes, 0x14);
    const std::uint32_t edge_point_count = u32(bytes, 0x18);
    const Section ranges = find_section(bytes, kEdgeRanges);
    if (ranges.type == 0 || ranges.length % 8 != 0 ||
        static_cast<std::size_t>(ranges.offset) + ranges.length > bytes.size()) {
        return 0;
    }
    check(ranges.length == 8U * edge_count,
          "EDGE_RANGES payload length == 8*edgeCount (header edgeCount)");

    std::uint32_t maximum = 0;
    for (std::uint32_t offset = 0; offset < ranges.length; offset += 8) {
        const std::uint32_t first_point = u32(bytes, ranges.offset + offset);
        const std::uint32_t point_count = u32(bytes, ranges.offset + offset + 4);
        check(first_point + point_count <= edge_point_count,
              "EDGE_RANGES entry stays within header edgePointCount (P)");
        maximum = std::max(maximum, point_count);
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

// Hand-authored fixture proving the reader distinguishes section 7 (EDGE_RANGES)
// from section 8 (EDGE_POSITIONS) rather than treating either as interchangeable
// "the edge section". Uses the real encoder (worker/src/tess/Mesh1.cpp) to
// produce a byte-exact MESH1 v1 blob per protocol/mesh_format.md, so the fixture
// itself is not hand-rolled bytes prone to drifting from the spec — only the
// input shape (E=2 edges, ranges {0,3},{3,2}) is hand-chosen.
//
// The EDGE_POSITIONS payload is deliberately built from float coordinates
// (1.5, 2.5, ...) whose IEEE-754 bit patterns, reinterpreted as u32 pairs, do
// NOT decode to {0,3},{3,2} — this is the negative control: if the reader were
// still pointed at section 8 (as before the fix), it must NOT recover the
// expected ranges.
std::vector<std::pair<std::uint32_t, std::uint32_t>> read_edge_ranges(
    const std::vector<std::uint8_t>& bytes, std::uint32_t section_type) {
    std::vector<std::pair<std::uint32_t, std::uint32_t>> out;
    const Section sec = find_section(bytes, section_type);
    if (sec.type == 0 || sec.length % 8 != 0) return out;
    for (std::uint32_t offset = 0; offset < sec.length; offset += 8) {
        out.emplace_back(u32(bytes, sec.offset + offset), u32(bytes, sec.offset + offset + 4));
    }
    return out;
}

void test_edge_ranges_section_is_distinguished_from_edge_positions() {
    onecad::tess::Mesh1Input in;
    // Minimal valid required sections: one triangle, one face.
    in.positions = {0.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f, 0.0f, 1.0f, 0.0f};
    in.indices = {0, 1, 2};
    in.face_ranges = {{0, 1}};
    in.face_ids = {"f:1"};

    // E=2 edges, P=5 points: ranges {0,3},{3,2}.
    in.has_edges = true;
    in.edge_ranges = {{0, 3}, {3, 2}};
    in.edge_ids = {"e:1", "e:2"};
    in.edge_positions = {
        1.5f, 2.5f, 3.5f,  // point 0
        4.5f, 5.5f, 6.5f,  // point 1
        7.5f, 8.5f, 9.5f,  // point 2
        10.5f, 11.5f, 12.5f,  // point 3
        13.5f, 14.5f, 15.5f,  // point 4
    };

    in.bbox_min[0] = in.bbox_min[1] = in.bbox_min[2] = 0.0f;
    in.bbox_max[0] = in.bbox_max[1] = in.bbox_max[2] = 20.0f;
    in.lod = 2;

    const std::vector<std::uint8_t> blob = onecad::tess::encode_mesh1(in);

    const std::vector<std::pair<std::uint32_t, std::uint32_t>> expected = {{0, 3}, {3, 2}};
    const std::vector<std::pair<std::uint32_t, std::uint32_t>> from_section_7 =
        read_edge_ranges(blob, kEdgeRanges);
    check(from_section_7 == expected,
          "reading section 7 (EDGE_RANGES) recovers the authored {0,3},{3,2} ranges");

    // Negative control: section 8 (EDGE_POSITIONS) holds float bit patterns, not
    // ranges. Reinterpreting them as {firstPoint,pointCount} pairs must NOT
    // reproduce the expected ranges — proving the two sections are not
    // interchangeable and that the fix actually reads the right one.
    const std::vector<std::pair<std::uint32_t, std::uint32_t>> from_section_8 =
        read_edge_ranges(blob, kEdgePositions);
    check(from_section_8 != expected,
          "reading section 8 (EDGE_POSITIONS) as ranges does NOT reproduce {0,3},{3,2} "
          "(negative control distinguishing the two sections)");
}

}  // namespace

int main() {
    test_fine_cylinder_is_materially_denser_than_coarse();
    test_edge_ranges_section_is_distinguished_from_edge_positions();
    if (g_failures == 0) std::fprintf(stderr, "tessellation_quality: OK\n");
    return g_failures;
}
