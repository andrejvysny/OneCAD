// test_tessellation_quality.cpp — display LOD contract for curved OCCT bodies.
//
// Fine meshes are the committed viewport/export representation. Keep a concrete
// cylinder probe here so a future tier retune cannot regress it to visibly flat
// round surfaces or restore the old fixed 16-span selected-edge outlines.
#include <algorithm>
#include <cstdint>
#include <cmath>
#include <cstring>
#include <array>
#include <cstdlib>
#include <fstream>
#include <cstdio>
#include <string>
#include <vector>

#include <BRep_Tool.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BinTools.hxx>
#include <Poly_PolygonOnTriangulation.hxx>
#include <future>
#include <sstream>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRep_Builder.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <Geom_Plane.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pln.hxx>
#include <gp_Ax1.hxx>
#include <gp_Trsf.hxx>

#include "tess/Tessellate.h"
#include "session/Tessellate.h"
#include "session/Session.h"
#include "session/ScratchJob.h"
#include "protocol/Dispatcher.h"

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
    constexpr std::uint32_t kEdgeRanges = 7;
    const Section ranges = find_section(bytes, kEdgeRanges);
    const Section positions = find_section(bytes, 8);
    const std::uint32_t edge_count = u32(bytes, 0x14);
    const std::uint32_t point_count = u32(bytes, 0x18);
    check(ranges.length == edge_count * 8, "edge range length matches topology count");
    check(positions.length == point_count * 12, "edge position length matches point count");
    if (ranges.type == 0 || ranges.length != edge_count * 8 ||
        static_cast<std::size_t>(ranges.offset) + ranges.length > bytes.size()) {
        return 0;
    }

    std::uint32_t maximum = 0;
    std::uint32_t cursor = 0;
    for (std::uint32_t offset = 0; offset < ranges.length; offset += 8) {
        const auto first = u32(bytes, ranges.offset + offset);
        const auto count = u32(bytes, ranges.offset + offset + 4);
        check(first == cursor && count <= point_count - std::min(first, point_count),
              "edge ranges are contiguous and bounded");
        cursor = first + count;
        maximum = std::max(maximum, count);
    }
    check(cursor == point_count, "edge ranges cover every polyline point");
    return maximum;
}

double scalar(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    const auto word = u32(bytes, offset);
    float value = 0;
    std::memcpy(&value, &word, sizeof(value));
    return value;
}

void check_circular_chord_error(const std::vector<std::uint8_t>& bytes) {
    const auto ranges = find_section(bytes, 7);
    const auto positions = find_section(bytes, 8);
    std::uint32_t circular_segments = 0;
    for (std::uint32_t r = 0; r < ranges.length; r += 8) {
        const auto first = u32(bytes, ranges.offset + r);
        const auto count = u32(bytes, ranges.offset + r + 4);
        for (std::uint32_t i = 1; i < count; ++i) {
            const auto a = positions.offset + (first + i - 1) * 12;
            const auto b = a + 12;
            if (std::abs(scalar(bytes, a + 8) - scalar(bytes, b + 8)) > 1e-5) continue;
            const double x = (scalar(bytes, a) + scalar(bytes, b)) * 0.5;
            const double y = (scalar(bytes, a + 4) + scalar(bytes, b + 4)) * 0.5;
            check(10.0 - std::hypot(x, y) <= 0.05 + 1e-5,
                  "fine circle chord deviation is bounded by 0.05 model units");
            check(std::abs(std::hypot(scalar(bytes, a), scalar(bytes, a + 4)) - 10.0) < 1e-5,
                  "circle samples lie on analytic radius");
            ++circular_segments;
        }
    }
    check(circular_segments >= 64, "geometric error check visits substantial circular coverage");
}

void check_cylinder_surface_error(const std::vector<std::uint8_t>& bytes) {
    const auto positions = find_section(bytes, 1);
    const auto indices = find_section(bytes, 3);
    check(positions.length == u32(bytes, 8) * 12 && indices.length == u32(bytes, 12) * 12,
          "surface section lengths match header counts");
    std::uint32_t side_triangles = 0;
    for (std::uint32_t i = 0; i < indices.length; i += 12) {
        std::array<std::size_t, 3> vertices{};
        for (std::size_t j = 0; j < 3; ++j) {
            vertices[j] = positions.offset + u32(bytes, indices.offset + i + j * 4) * 12;
        }
        const double z = scalar(bytes, vertices[0] + 8);
        if (std::abs(z - scalar(bytes, vertices[1] + 8)) < 1e-5 &&
            std::abs(z - scalar(bytes, vertices[2] + 8)) < 1e-5) continue;
        for (std::size_t j = 0; j < 3; ++j) {
            const auto a = vertices[j], b = vertices[(j + 1) % 3];
            const double x = (scalar(bytes, a) + scalar(bytes, b)) * 0.5;
            const double y = (scalar(bytes, a + 4) + scalar(bytes, b + 4)) * 0.5;
            check(std::abs(10.0 - std::hypot(x, y)) <= 0.05 + 1e-5,
                  "fine side triangles stay within 0.05 of analytic cylinder");
        }
        ++side_triangles;
    }
    check(side_triangles >= 64, "surface error check visits cylinder side triangles");
}

void test_fine_cylinder_is_materially_denser_than_coarse() {
    const TopoDS_Shape coarse_cylinder = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
    const TopoDS_Shape fine_cylinder = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
    const onecad::tess::BodyMesh coarse = onecad::tess::tessellate_body(
        coarse_cylinder, "body_coarse", "coarse", true, nullptr);
    const onecad::tess::BodyMesh fine = onecad::tess::tessellate_body(
        fine_cylinder, "body_fine", "fine", true, nullptr);

    check(coarse.ok && fine.ok, "cylinder tessellation succeeds for coarse and fine");
    if (!coarse.ok || !fine.ok) return;
    check(fine.triangle_count >= coarse.triangle_count * 3,
          "fine cylinder has at least 3x the coarse triangle density");

    const std::uint32_t coarse_edge_points = greatest_edge_point_count(coarse.blob);
    const std::uint32_t fine_edge_points = greatest_edge_point_count(fine.blob);
    check(fine_edge_points > 16,
          "fine circular edge exceeds the retired fixed 16-span sampling");
    check(fine_edge_points >= coarse_edge_points * 3,
          "fine circular edge density tracks the finer face LOD");
    check_circular_chord_error(fine.blob);
    check_cylinder_surface_error(fine.blob);
}

std::array<std::uint32_t, 7> check_metadata(const TopoDS_Shape& shape,
                                         const std::string& name) {
    const auto mesh = onecad::tess::tessellate_body(shape, name, "fine", true, nullptr);
    const auto repeated = onecad::tess::tessellate_body(shape, name, "fine", true, nullptr);
    check(mesh.ok && repeated.ok, name + ": repeated tessellation succeeds");
    if (!mesh.ok || !repeated.ok) return {};
    for (const std::uint32_t type : {5U, 6U, 9U, 10U, 14U, 15U, 16U}) {
        const auto a = find_section(mesh.blob, type);
        const auto b = find_section(repeated.blob, type);
        check(a.length == b.length && std::equal(mesh.blob.begin() + a.offset,
                  mesh.blob.begin() + a.offset + a.length, repeated.blob.begin() + b.offset),
              name + ": deterministic topology IDs and metadata");
    }
    const auto classes = find_section(mesh.blob, 14);
    const auto ranges = find_section(mesh.blob, 15);
    const auto faces = find_section(mesh.blob, 16);
    const auto count = u32(mesh.blob, 0x14);
    check(classes.type == 14 && classes.length == 4 * count,
          name + ": one classification per topology row");
    check(ranges.type == 15 && ranges.length == 8 * count && faces.length % 4 == 0,
          name + ": adjacency section lengths");
    std::array<std::uint32_t, 7> histogram{};
    std::uint32_t cursor = 0;
    for (std::uint32_t edge = 0; edge < count; ++edge) {
        const auto code = u32(mesh.blob, classes.offset + 4 * edge);
        check(code < histogram.size(), name + ": valid class code");
        if (code < histogram.size()) ++histogram[code];
        const auto first = u32(mesh.blob, ranges.offset + edge * 8);
        const auto incident_count = u32(mesh.blob, ranges.offset + edge * 8 + 4);
        check(first == cursor && (first + incident_count) * 4 <= faces.length,
              name + ": contiguous bounded adjacency");
        std::uint32_t previous = 0;
        for (std::uint32_t i = 0; i < incident_count; ++i) {
            const auto ordinal = u32(mesh.blob, faces.offset + (first + i) * 4);
            check(ordinal < u32(mesh.blob, 0x10) && (i == 0 || ordinal > previous),
                  name + ": ascending unique bounded face ordinals");
            previous = ordinal;
        }
        cursor = first + incident_count;
    }
    check(cursor * 4 == faces.length, name + ": full adjacency coverage");
    if (const char* directory = std::getenv("ONECAD_MESH_FIXTURES")) {
        std::ofstream file(std::string(directory) + "/" + name + ".mesh1", std::ios::binary);
        file.write(reinterpret_cast<const char*>(mesh.blob.data()), mesh.blob.size());
        check(file.good(), name + ": exported interoperability fixture");
    }
    return histogram;
}

void test_edge_metadata_fixtures() {
    const auto box = BRepPrimAPI_MakeBox(10, 15, 20).Shape();
    const auto box_classes = check_metadata(box, "box");
    check(box_classes[2] == 12, "box has twelve sharp edges");
    const auto cylinder = check_metadata(BRepPrimAPI_MakeCylinder(10, 20).Shape(), "cylinder");
    check(cylinder[4] == 1 && cylinder[2] == 2, "cylinder suppresses seam and retains rims");
    const auto sphere = check_metadata(BRepPrimAPI_MakeSphere(10).Shape(), "sphere");
    check(sphere[4] > 0 && sphere[5] > 0, "sphere retains periodic seam and degenerate poles");
    const auto plane = BRepBuilderAPI_MakeFace(gp_Pln(gp::XOY()), 0, 10, 0, 10).Shape();
    check(check_metadata(plane, "open_boundary")[1] == 4, "open face has four boundaries");
    BRepFilletAPI_MakeFillet fillet(box);
    TopExp_Explorer edge(box, TopAbs_EDGE);
    fillet.Add(1.0, TopoDS::Edge(edge.Current()));
    fillet.Build();
    check(fillet.IsDone(), "filleted block fixture builds");
    if (fillet.IsDone()) {
        check(check_metadata(fillet.Shape(), "filleted_tangent_block")[3] >= 2,
              "fillet tangent joins classify smooth");
    }
    check(check_metadata(box.Reversed(), "reversed_box")[2] == 12,
          "reversed faces preserve sharp classification");
    gp_Trsf transform;
    transform.SetTranslation(gp_Vec(100, -30, 17));
    check(check_metadata(BRepBuilderAPI_Transform(box, transform, true).Shape(),
                        "transformed_box")[2] == 12,
          "located faces preserve sharp classification");
    check(check_metadata(BRepBuilderAPI_Transform(box, transform, false).Shape(),
                        "relocated_box")[2] == 12,
          "shared-geometry moved faces preserve sharp classification");
    transform.SetRotation(gp_Ax1(gp_Pnt(3, -7, 11), gp_Dir(1, 2, 3)), 0.731);
    const auto located = box.Moved(TopLoc_Location(transform));
    transform.SetTranslation(gp_Vec(19, -23, 31));
    check(check_metadata(located.Moved(TopLoc_Location(transform)).Reversed(),
                        "nested_rotated_box")[2] == 12,
          "composed rigid locations preserve world vertices and winding");
}

void test_non_manifold_edge() {
    const gp_Pnt a(0, 0, 0), b(10, 0, 0);
    const auto shared = BRepBuilderAPI_MakeEdge(a, b).Edge();
    BRep_Builder builder;
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    for (const gp_Pnt tip : {gp_Pnt(0, 10, 0), gp_Pnt(0, 0, 10), gp_Pnt(0, -10, 0)}) {
        BRepBuilderAPI_MakeWire wire;
        wire.Add(shared);
        wire.Add(BRepBuilderAPI_MakeEdge(b, tip).Edge());
        wire.Add(BRepBuilderAPI_MakeEdge(tip, a).Edge());
        builder.Add(compound, BRepBuilderAPI_MakeFace(wire.Wire()).Face());
    }
    const auto classes = check_metadata(compound, "non_manifold");
    check(classes[6] == 1 && classes[1] == 6,
          "three-face edge is non-manifold, outer edges remain boundaries");
}

void test_display_angular_tolerance() {
    const gp_Pnt a(0, 0, 0), b(10, 0, 0);
    for (double angle : {0.0005, 0.002}) {
        const auto shared = BRepBuilderAPI_MakeEdge(a, b).Edge();
        BRep_Builder builder;
        TopoDS_Compound compound;
        builder.MakeCompound(compound);
        for (int side : {1, -1}) {
            const gp_Pnt tip(0, side * 10 * std::cos(angle),
                             side == 1 ? 0 : 10 * std::sin(angle));
            BRepBuilderAPI_MakeWire wire;
            wire.Add(side == 1 ? shared : TopoDS::Edge(shared.Reversed()));
            wire.Add(BRepBuilderAPI_MakeEdge(side == 1 ? b : a, tip).Edge());
            wire.Add(BRepBuilderAPI_MakeEdge(tip, side == 1 ? a : b).Edge());
            builder.Add(compound, BRepBuilderAPI_MakeFace(wire.Wire()).Face());
        }
        const bool smooth = angle < 0.001;
        const auto classes = check_metadata(compound, smooth ? "near_tangent" : "small_crease");
        check(classes[smooth ? 3 : 2] == 1,
              "unencoded continuity honors 0.001 rad display tolerance");
    }
}

void test_partial_face_failure_is_not_published() {
    BRep_Builder builder;
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    builder.Add(compound, BRepPrimAPI_MakeBox(10, 15, 20).Shape());
    TopoDS_Face missing;
    builder.MakeFace(missing);
    builder.Add(compound, missing);
    const auto mesh = onecad::tess::tessellate_body(compound, "partial", "fine", true, nullptr);
    check(!mesh.ok && mesh.blob.empty() && mesh.triangle_count == 0,
          "one missing face rejects the complete body without a partial blob");
    onecad::session::Session session;
    session.open("partial-face-test", 0, 3, "determinism");
    onecad::session::ScratchJob seed;
    seed.job_id = 7;
    seed.plan_document_revision = 0;
    seed.prepared_snapshot_id = 1;
    seed.bodies.create("aa_good", "fixture", BRepPrimAPI_MakeBox(2, 2, 2).Shape());
    seed.bodies.create("zz_bad", "fixture", compound);
    session.store_prepared(std::move(seed));
    check(session.accept_prepared(7, 0, 3).ok, "partial-face request fixture seeded");
    onecad::protocol::Dispatcher dispatcher;
    dispatcher.register_verb("Tessellate", [&session](const auto& request, const auto&, auto&) {
        return onecad::session::handle_tessellate(session, request);
    });
    const auto response = dispatcher.dispatch_once(onecad::protocol::Envelope::request(
        8, "Tessellate", {{"lod", "fine"}, {"bodyIds", "all"}}));
    check(!response.ok.value_or(true) && response.error && response.error->code == "OP_FAILED",
          "Tessellate request reports OP_FAILED for incomplete body");
    check(response.error && response.error->detail &&
              response.error->detail->value("bodyId", std::string()) == "zz_bad",
          "Tessellate failure names the rejected body");
    check(response.bin.empty() && response.out_bin.empty(),
          "failed batch drops earlier valid-body binary attachments");
}

void test_degenerate_face_retains_topology_row() {
    BRep_Builder builder;
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    builder.Add(compound, BRepPrimAPI_MakeBox(10, 15, 20).Shape());
    const auto sphere = BRepPrimAPI_MakeSphere(1).Shape();
    TopExp_Explorer edges(sphere, TopAbs_EDGE);
    while (edges.More() && !BRep_Tool::Degenerated(TopoDS::Edge(edges.Current()))) edges.Next();
    check(edges.More(), "sphere fixture supplies a degenerate pole edge");
    if (!edges.More()) return;
    TopoDS_Wire wire;
    builder.MakeWire(wire);
    builder.Add(wire, edges.Current());
    TopoDS_Face face;
    builder.MakeFace(face, new Geom_Plane(gp::XOY()), 1e-7);
    builder.Add(face, wire);
    builder.Add(compound, face);
    const auto mesh = onecad::tess::tessellate_body(compound, "degenerate_face", "fine", true, nullptr);
    check(mesh.ok && mesh.triangle_count == 12,
          "topology-only degenerate face does not reject valid body triangles");
    if (!mesh.ok) return;
    const auto ranges = find_section(mesh.blob, 4);
    check(u32(mesh.blob, 0x10) == 7 && u32(mesh.blob, ranges.offset + 6 * 8 + 4) == 0,
          "degenerate face keeps its ordinal and zero-triangle range");
}

}  // namespace

void test_lod_request_order_is_not_geometry_state() {
    const TopoDS_Shape shape = BRepPrimAPI_MakeCylinder(10, 20).Shape();
    const auto coarse = onecad::tess::tessellate_body(shape, "cache", "coarse", true, nullptr);
    const auto fine = onecad::tess::tessellate_body(shape, "cache", "fine", true, nullptr);
    const auto coarse_after_fine = onecad::tess::tessellate_body(shape, "cache", "coarse", true, nullptr);
    check(coarse.ok && fine.ok && coarse_after_fine.ok, "LOD sequence meshes succeed");
    check(coarse.blob == coarse_after_fine.blob,
          "coarse MESH1 is byte-identical before and after fine request");
    const auto raw = onecad::tess::tessellate_raw(shape, "coarse");
    check(raw.triangle_count == coarse.triangle_count,
          "raw coarse export matches cold coarse viewport after fine request");
    bool original_has_mesh = false;
    for (TopExp_Explorer faces(shape, TopAbs_FACE); faces.More(); faces.Next()) {
        TopLoc_Location location;
        original_has_mesh = original_has_mesh ||
            !BRep_Tool::Triangulation(TopoDS::Face(faces.Current()), location).IsNull();
    }
    check(!original_has_mesh, "meshing does not mutate published source triangulation");
}

std::string brep_bytes(const TopoDS_Shape& shape) {
    std::ostringstream stream(std::ios::out | std::ios::binary);
    BinTools::Write(shape, stream);
    return stream.str();
}

struct CacheState {
    std::vector<const void*> handles;
    std::vector<double> deflections;
    std::vector<bool> flags;
    bool operator==(const CacheState&) const = default;
};

CacheState cache_state(const TopoDS_Shape& shape) {
    CacheState state;
    for (TopExp_Explorer faces(shape, TopAbs_FACE); faces.More(); faces.Next()) {
        const auto face = TopoDS::Face(faces.Current());
        TopLoc_Location location;
        const auto tri = BRep_Tool::Triangulation(face, location);
        state.handles.push_back(tri.get());
        state.deflections.push_back(tri.IsNull() ? 0.0 : tri->Deflection());
        state.flags.insert(state.flags.end(), {face.Free(), face.Locked(), face.Modified(), face.Checked()});
        for (TopExp_Explorer edges(face, TopAbs_EDGE); edges.More(); edges.Next()) {
            const auto edge = TopoDS::Edge(edges.Current());
            state.flags.insert(state.flags.end(), {edge.Free(), edge.Locked(), edge.Modified(), edge.Checked(),
                                                  BRep_Tool::SameParameter(edge), BRep_Tool::SameRange(edge)});
            const auto polygon = tri.IsNull() ? Handle(Poly_PolygonOnTriangulation){}
                : BRep_Tool::PolygonOnTriangulation(edge, tri, location);
            state.handles.push_back(polygon.get());
            state.deflections.push_back(polygon.IsNull() ? 0.0 : polygon->Deflection());
        }
    }
    return state;
}

void test_existing_cache_and_snapshot_identity_are_preserved() {
    const TopoDS_Shape shape = BRepPrimAPI_MakeCylinder(10, 20).Shape();
    const TopoDS_Shape snapshot = shape;
    BRepMesh_IncrementalMesh seed(shape, 0.001, false, 0.02, false);
    check(seed.IsDone(), "seed cached fine triangulation");
    const auto before = brep_bytes(shape);
    const auto caches = cache_state(shape);
    onecad::elementmap::ElementMapPartition partition;
    TopExp_Explorer faces(shape, TopAbs_FACE);
    partition.mint("cache", "el_side", onecad::kernel::elementmap::ElementKind::Face,
                   faces.Current(), shape);
    const std::vector<std::uint32_t> colors{0x112233ff, 0x445566ff, 0x778899ff};
    const auto first = onecad::tess::tessellate_body(shape, "cache", "coarse", true, &partition, &colors);
    const auto fine = onecad::tess::tessellate_raw(shape, "fine");
    auto call = [&] { return onecad::tess::tessellate_body(snapshot, "cache", "coarse", true, &partition, &colors); };
    auto a = std::async(std::launch::async, call);
    auto b = std::async(std::launch::async, call);
    const auto second = a.get();
    const auto third = b.get();
    check(first.ok && second.ok && third.ok && fine.triangle_count > first.triangle_count,
          "isolated mixed-LOD and concurrent requests succeed");
    check(first.blob == second.blob && first.blob == third.blob,
          "snapshot requests retain exact bounds, IDs, colors and geometry");
    check(cache_state(shape) == caches && cache_state(snapshot) == caches,
          "original cache handles, deflections and shape/edge flags are unchanged");
    check(brep_bytes(shape) == before && brep_bytes(snapshot) == before,
          "source BREP, triangulation, edge polygons and serialized flags remain unchanged");
    const auto cold = BRepPrimAPI_MakeCylinder(10, 20).Shape();
    const auto raw = onecad::tess::tessellate_raw(shape, "coarse");
    const auto cold_raw = onecad::tess::tessellate_raw(cold, "coarse");
    check(raw.positions == cold_raw.positions && raw.normals == cold_raw.normals && raw.indices == cold_raw.indices,
          "raw export geometry independent of source cached triangulation");
}

void test_polygon_only_face_refuses_without_partial_geometry() {
    Handle(Poly_Triangulation) tri = new Poly_Triangulation(3, 1, false);
    tri->SetNode(1, gp_Pnt(0, 0, 0));
    tri->SetNode(2, gp_Pnt(1, 0, 0));
    tri->SetNode(3, gp_Pnt(0, 1, 0));
    tri->SetTriangle(1, Poly_Triangle(1, 2, 3));
    BRep_Builder builder;
    TopoDS_Face face;
    builder.MakeFace(face, tri);
    const auto mesh = onecad::tess::tessellate_body(face, "polygon_only", "fine", true, nullptr);
    const auto raw = onecad::tess::tessellate_raw(face, "fine");
    check(!mesh.ok && mesh.blob.empty() && !raw.ok && raw.indices.empty(),
          "polygon-only input refuses explicitly without silently losing geometry");
}

int main() {
    test_polygon_only_face_refuses_without_partial_geometry();
    test_existing_cache_and_snapshot_identity_are_preserved();
    test_lod_request_order_is_not_geometry_state();
    test_fine_cylinder_is_materially_denser_than_coarse();
    test_edge_metadata_fixtures();
    test_non_manifold_edge();
    test_display_angular_tolerance();
    test_partial_face_failure_is_not_published();
    test_degenerate_face_retains_topology_row();
    if (g_failures == 0) std::fprintf(stderr, "tessellation_quality: OK\n");
    return g_failures;
}
