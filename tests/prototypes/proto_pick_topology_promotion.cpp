#include "app/document/Document.h"
#include "ui/selection/ModelPickerAdapter.h"

#include <BRepPrimAPI_MakeBox.hxx>
#include <gp_Pnt.hxx>
#include <QCoreApplication>
#include <QMatrix4x4>
#include <iostream>

int main(int argc, char** argv) {
    QCoreApplication app(argc, argv);

    onecad::app::Document document;
    TopoDS_Shape shape = BRepPrimAPI_MakeBox(gp_Pnt(-1.0, -1.0, -1.0), 2.0, 2.0, 2.0).Shape();
    std::string bodyId = document.addBody(shape);
    if (bodyId.empty()) {
        std::cerr << "Failed to add body.\n";
        return 1;
    }

    const auto& store = document.meshStore();
    const auto* mesh = store.findMesh(bodyId);
    if (!mesh) {
        std::cerr << "Mesh not found.\n";
        return 1;
    }

    onecad::ui::selection::ModelPickerAdapter picker;
    onecad::ui::selection::ModelPickerAdapter::Mesh pickMesh;
    pickMesh.bodyId = mesh->bodyId;
    pickMesh.vertices = mesh->vertices;
    pickMesh.triangles.reserve(mesh->triangles.size());
    for (const auto& tri : mesh->triangles) {
        onecad::ui::selection::ModelPickerAdapter::Triangle t;
        t.i0 = tri.i0;
        t.i1 = tri.i1;
        t.i2 = tri.i2;
        t.faceId = tri.faceId;
        pickMesh.triangles.push_back(t);
    }
    for (const auto& [faceId, topo] : mesh->topologyByFace) {
        onecad::ui::selection::ModelPickerAdapter::FaceTopology faceTopo;
        for (const auto& edge : topo.edges) {
            onecad::ui::selection::ModelPickerAdapter::EdgePolyline edgeLine;
            edgeLine.edgeId = edge.edgeId;
            edgeLine.points = edge.points;
            faceTopo.edges.push_back(std::move(edgeLine));
        }
        for (const auto& vertex : topo.vertices) {
            onecad::ui::selection::ModelPickerAdapter::VertexSample sample;
            sample.vertexId = vertex.vertexId;
            sample.position = vertex.position;
            faceTopo.vertices.push_back(std::move(sample));
        }
        pickMesh.topologyByFace[faceId] = std::move(faceTopo);
    }
    picker.setMeshes({pickMesh});

    QMatrix4x4 projection;
    projection.ortho(-2.5f, 2.5f, -2.5f, 2.5f, -10.0f, 10.0f);
    QMatrix4x4 view;
    view.setToIdentity();
    QMatrix4x4 viewProjection = projection * view;

    auto result = picker.pick(QPoint(30, 50), 8.0, viewProjection, QSize(100, 100));
    bool hasEdge = false;
    std::string edgeId;
    for (const auto& hit : result.hits) {
        if (hit.kind == onecad::app::selection::SelectionKind::Edge) {
            hasEdge = true;
            edgeId = hit.id.elementId;
            break;
        }
    }
    if (!hasEdge) {
        std::cerr << "Expected edge promotion.\n";
        return 1;
    }
    if (!edgeId.starts_with(bodyId + "/edge/")) {
        std::cerr << "Edge id not ElementMap-based: " << edgeId << "\n";
        return 1;
    }

    std::cout << "Pick topology promotion prototype passed.\n";
    return 0;
}
