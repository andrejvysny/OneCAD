#include "ui/selection/ModelPickerAdapter.h"

#include <QMatrix4x4>
#include <QSize>
#include <QPoint>
#include <iostream>

using onecad::ui::selection::ModelPickerAdapter;
using onecad::app::selection::SelectionKind;

namespace {
bool hasKind(const onecad::app::selection::PickResult& result, SelectionKind kind) {
    for (const auto& hit : result.hits) {
        if (hit.kind == kind) {
            return true;
        }
    }
    return false;
}
} // namespace

int main() {
    ModelPickerAdapter picker;

    ModelPickerAdapter::Mesh mesh;
    mesh.bodyId = "body0";
    mesh.vertices = {
        {-0.5f, -0.5f, 0.0f},
        {0.5f, -0.5f, 0.0f},
        {0.5f, 0.5f, 0.0f},
        {-0.5f, 0.5f, 0.0f}
    };
    mesh.triangles = {
        {0, 1, 2, "face0"},
        {0, 2, 3, "face0"}
    };
    picker.setMeshes({mesh});

    QMatrix4x4 viewProjection;
    viewProjection.setToIdentity();
    QSize viewportSize(100, 100);
    double tolerance = 6.0; // Screen-space pixel tolerance for picking.

    // With identity viewProjection, NDC maps to screen space:
    // NDC [-1, 1] -> screen [0, 100] for a 100x100 viewport.
    // Point near (-0.5, -0.5) maps to roughly (25, 75) in screen pixels.
    auto vertexPick = picker.pick(QPoint(26, 74), tolerance, viewProjection, viewportSize);
    if (!hasKind(vertexPick, SelectionKind::Vertex)) {
        std::cerr << "Expected vertex pick.\n";
        return 1;
    }

    auto edgePick = picker.pick(QPoint(50, 74), tolerance, viewProjection, viewportSize);
    if (!hasKind(edgePick, SelectionKind::Edge)) {
        std::cerr << "Expected edge pick.\n";
        return 1;
    }

    auto facePick = picker.pick(QPoint(50, 50), tolerance, viewProjection, viewportSize);
    if (!hasKind(facePick, SelectionKind::Face)) {
        std::cerr << "Expected face pick.\n";
        return 1;
    }
    if (hasKind(facePick, SelectionKind::Edge) || hasKind(facePick, SelectionKind::Vertex)) {
        std::cerr << "Unexpected edge/vertex pick at face center. Hits:\n";
        for (const auto& hit : facePick.hits) {
            std::cerr << "  kind=" << static_cast<int>(hit.kind)
                      << " id=" << hit.id.elementId
                      << " dist=" << hit.screenDistance << "\n";
        }
        return 1;
    }

    ModelPickerAdapter::Mesh meshBack;
    meshBack.bodyId = "body1";
    meshBack.vertices = {
        {-0.5f, -0.5f, 0.6f},
        {0.5f, -0.5f, 0.6f},
        {0.5f, 0.5f, 0.6f},
        {-0.5f, 0.5f, 0.6f}
    };
    meshBack.triangles = {
        {0, 1, 2, "face1"},
        {0, 2, 3, "face1"}
    };
    picker.setMeshes({mesh, meshBack});

    auto overlapPick = picker.pick(QPoint(50, 50), tolerance, viewProjection, viewportSize);
    int faceCount = 0;
    bool hasFace0 = false;
    bool hasFace1 = false;
    for (const auto& hit : overlapPick.hits) {
        if (hit.kind == SelectionKind::Face) {
            faceCount++;
            if (hit.id.elementId == "face0") {
                hasFace0 = true;
            }
            if (hit.id.elementId == "face1") {
                hasFace1 = true;
            }
        }
    }
    if (faceCount < 2) {
        std::cerr << "Expected multiple face hits for overlap.\n";
        return 1;
    }
    if (!hasFace0 || !hasFace1) {
        std::cerr << "Expected overlap to include face0 and face1.\n";
        return 1;
    }

    std::cout << "Model picker prototype passed.\n";
    return 0;
}
