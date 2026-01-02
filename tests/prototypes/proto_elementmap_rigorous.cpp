#include <algorithm>
#include <cmath>
#include <iostream>
#include <string>
#include <vector>

// OCCT
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Tool.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Pnt.hxx>

#include "kernel/elementmap/ElementMap.h"

using onecad::kernel::elementmap::ElementDescriptor;
using onecad::kernel::elementmap::ElementId;
using onecad::kernel::elementmap::ElementKind;
using onecad::kernel::elementmap::ElementMap;

namespace {

struct TestContext {
    int failures{0};

    void expect(bool condition, const std::string& message) {
        if (!condition) {
            ++failures;
            std::cerr << "FAIL: " << message << std::endl;
        }
    }
};

bool startsWith(const std::string& value, const std::string& prefix) {
    return value.size() >= prefix.size() && value.compare(0, prefix.size(), prefix) == 0;
}

bool nearlyEqual(double a, double b, double tol = 1e-6) {
    return std::abs(a - b) <= tol;
}

TopoDS_Face findTopFace(const TopoDS_Shape& shape) {
    TopoDS_Face topFace;
    double maxZ = -1e9;

    for (TopExp_Explorer exp(shape, TopAbs_FACE); exp.More(); exp.Next()) {
        const TopoDS_Face face = TopoDS::Face(exp.Current());
        TopExp_Explorer expWire(face, TopAbs_WIRE);
        if (!expWire.More()) {
            continue;
        }
        TopExp_Explorer expEdge(expWire.Current(), TopAbs_EDGE);
        if (!expEdge.More()) {
            continue;
        }
        const TopoDS_Edge edge = TopoDS::Edge(expEdge.Current());
        const gp_Pnt p = BRep_Tool::Pnt(TopExp::FirstVertex(edge));
        if (p.Z() > maxZ) {
            maxZ = p.Z();
            topFace = face;
        }
    }

    return topFace;
}

TopoDS_Shape makeSplitCutter() {
    // Thin slab that cuts entirely through the box to force a face split.
    return BRepPrimAPI_MakeBox(gp_Pnt(4.5, -1.0, -1.0), gp_Pnt(5.5, 11.0, 11.0)).Shape();
}

void testBasicCutPreservesFace(TestContext& ctx) {
    ElementMap emap;

    BRepPrimAPI_MakeBox mkBox(10.0, 10.0, 10.0);
    mkBox.Build();
    const TopoDS_Shape boxShape = mkBox.Shape();

    const TopoDS_Face topFace = findTopFace(boxShape);
    ctx.expect(!topFace.IsNull(), "Top face should be found");

    const ElementId topId{"face-top"};
    emap.registerElement(topId, ElementKind::Face, topFace, "op-box");

    BRepPrimAPI_MakeBox mkTool(gp_Pnt(3.0, 3.0, -1.0), gp_Pnt(7.0, 7.0, 11.0));
    mkTool.Build();

    BRepAlgoAPI_Cut cut(boxShape, mkTool.Shape());
    cut.Build();
    ctx.expect(cut.IsDone(), "Cut should succeed");

    emap.update(cut, "op-cut");
    const auto* entry = emap.find(topId);
    ctx.expect(entry != nullptr, "Top face ID should remain after cut");
    ctx.expect(entry != nullptr && !entry->shape.IsNull(), "Top face should have a shape after cut");
}

void testSplitCreatesSiblingIds(TestContext& ctx) {
    ElementMap emap;

    BRepPrimAPI_MakeBox mkBox(10.0, 10.0, 10.0);
    mkBox.Build();
    const TopoDS_Shape boxShape = mkBox.Shape();

    const TopoDS_Face topFace = findTopFace(boxShape);
    ctx.expect(!topFace.IsNull(), "Top face should be found for split test");

    const ElementId topId{"face-top"};
    emap.registerElement(topId, ElementKind::Face, topFace, "op-box");

    BRepAlgoAPI_Cut cut(boxShape, makeSplitCutter());
    cut.Build();
    ctx.expect(cut.IsDone(), "Split cut should succeed");

    emap.update(cut, "op-split");

    const auto ids = emap.ids();
    ctx.expect(ids.size() >= 2, "Split should create at least one sibling ID");

    const std::string prefix = topId.value + "/face-split-";
    bool foundSplit = false;
    for (const auto& id : ids) {
        if (id.value != topId.value && startsWith(id.value, prefix)) {
            foundSplit = true;
            if (const auto* entry = emap.find(id)) {
                bool hasSource = false;
                for (const auto& source : entry->sources) {
                    if (source.value == topId.value) {
                        hasSource = true;
                        break;
                    }
                }
                ctx.expect(hasSource, "Split child should reference source face");
            }
        }
    }
    ctx.expect(foundSplit, "Split sibling ID should exist with expected prefix");
}

std::vector<std::string> collectSplitIds() {
    ElementMap emap;

    BRepPrimAPI_MakeBox mkBox(10.0, 10.0, 10.0);
    mkBox.Build();
    const TopoDS_Shape boxShape = mkBox.Shape();

    const TopoDS_Face topFace = findTopFace(boxShape);
    const ElementId topId{"face-top"};
    emap.registerElement(topId, ElementKind::Face, topFace, "op-box");

    BRepAlgoAPI_Cut cut(boxShape, makeSplitCutter());
    cut.Build();
    emap.update(cut, "op-split");

    std::vector<std::string> ids;
    for (const auto& id : emap.ids()) {
        ids.push_back(id.value);
    }
    std::sort(ids.begin(), ids.end());
    return ids;
}

void testDeterministicIds(TestContext& ctx) {
    const auto first = collectSplitIds();
    const auto second = collectSplitIds();
    ctx.expect(first == second, "Split IDs should be deterministic across runs");
}

void testSerializationRoundTrip(TestContext& ctx) {
    ElementMap emap;

    BRepPrimAPI_MakeBox mkBox(10.0, 10.0, 10.0);
    mkBox.Build();
    const TopoDS_Shape boxShape = mkBox.Shape();

    const TopoDS_Face topFace = findTopFace(boxShape);
    ctx.expect(!topFace.IsNull(), "Top face should be found for serialization test");

    const ElementId topId{"face-top"};
    emap.registerElement(topId, ElementKind::Face, topFace, "op-box");

    const std::string serialized = emap.toString();

    ElementMap restored;
    ctx.expect(restored.fromString(serialized), "ElementMap should deserialize");
    const auto* entry = restored.find(topId);
    ctx.expect(entry != nullptr, "Restored map should contain top face id");
    if (entry) {
        ctx.expect(nearlyEqual(entry->descriptor.center.Z(), 10.0), "Restored descriptor center Z");
        ctx.expect(entry->descriptor.shapeType == TopAbs_FACE, "Restored descriptor shape type");
    }
}

void testReverseMapMultiId(TestContext& ctx) {
    ElementMap emap;

    BRepPrimAPI_MakeBox mkBox(10.0, 10.0, 10.0);
    mkBox.Build();
    const TopoDS_Shape boxShape = mkBox.Shape();
    const TopoDS_Face topFace = findTopFace(boxShape);

    const ElementId idA{"face-a"};
    const ElementId idB{"face-b"};
    emap.registerElement(idA, ElementKind::Face, topFace, "op-box");
    emap.registerElement(idB, ElementKind::Face, topFace, "op-box");

    const auto ids = emap.findIdsByShape(topFace);
    bool hasA = false;
    bool hasB = false;
    for (const auto& id : ids) {
        hasA = hasA || id.value == idA.value;
        hasB = hasB || id.value == idB.value;
    }
    ctx.expect(hasA && hasB, "Reverse map should keep multiple IDs for same shape");
}

} // namespace

int main() {
    std::cout << "--- ElementMap Rigorous Tests ---" << std::endl;

    TestContext ctx;
    testBasicCutPreservesFace(ctx);
    testSplitCreatesSiblingIds(ctx);
    testDeterministicIds(ctx);
    testSerializationRoundTrip(ctx);
    testReverseMapMultiId(ctx);

    if (ctx.failures > 0) {
        std::cerr << "Tests failed: " << ctx.failures << std::endl;
        return 1;
    }

    std::cout << "All ElementMap tests passed." << std::endl;
    return 0;
}
