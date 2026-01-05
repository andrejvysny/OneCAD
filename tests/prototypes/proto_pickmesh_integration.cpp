#include "app/document/Document.h"
#include "render/scene/SceneMeshStore.h"
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
    picker.setMeshes({pickMesh});

    QMatrix4x4 projection;
    projection.ortho(-2.5f, 2.5f, -2.5f, 2.5f, -10.0f, 10.0f);
    QMatrix4x4 view;
    view.setToIdentity();
    QMatrix4x4 viewProjection = projection * view;

    auto result = picker.pick(QPoint(50, 50), 6.0, viewProjection, QSize(100, 100));
    bool hasFace = false;
    for (const auto& hit : result.hits) {
        if (hit.kind == onecad::app::selection::SelectionKind::Face) {
            if (hit.id.ownerId == bodyId) {
                hasFace = true;
            }
            break;
        }
    }
    if (!hasFace) {
        std::cerr << "Expected face hit from pick mesh.\n";
        return 1;
    }

    std::cout << "Pick mesh integration prototype passed.\n";
    return 0;
}
