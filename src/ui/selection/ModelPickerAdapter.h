#ifndef ONECAD_UI_SELECTION_MODELPICKERADAPTER_H
#define ONECAD_UI_SELECTION_MODELPICKERADAPTER_H

#include "../../app/selection/SelectionTypes.h"
#include <QMatrix4x4>
#include <QPoint>
#include <QSize>
#include <QVector3D>
#include <array>
#include <cstdint>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace onecad::ui::selection {

class ModelPickerAdapter {
public:
    struct Triangle {
        std::uint32_t i0 = 0;
        std::uint32_t i1 = 0;
        std::uint32_t i2 = 0;
        std::string faceId;
    };

    struct EdgePolyline {
        std::string edgeId;
        std::vector<QVector3D> points;
    };

    struct VertexSample {
        std::string vertexId;
        QVector3D position;
    };

    struct FaceTopology {
        std::vector<EdgePolyline> edges;
        std::vector<VertexSample> vertices;
    };

    struct Mesh {
        std::string bodyId;
        std::vector<QVector3D> vertices;
        std::vector<Triangle> triangles;
        std::unordered_map<std::string, FaceTopology> topologyByFace;
    };

    struct Ray {
        QVector3D origin;
        QVector3D direction;
        bool valid = false;
    };

    void setMeshes(std::vector<Mesh>&& meshes);

    app::selection::PickResult pick(const QPoint& screenPos,
                                    double tolerancePixels,
                                    const QMatrix4x4& viewProjection,
                                    const QSize& viewportSize) const;

    bool getFaceTriangles(const std::string& bodyId,
                          const std::string& faceId,
                          std::vector<std::array<QVector3D, 3>>& outTriangles) const;
    bool getBodyTriangles(const std::string& bodyId,
                          std::vector<std::array<QVector3D, 3>>& outTriangles) const;
    bool getEdgePolyline(const std::string& bodyId,
                         const std::string& edgeId,
                         std::vector<QVector3D>& outPolyline) const;

    bool getEdgeSegment(const std::string& bodyId,
                        const std::string& edgeId,
                        std::array<QVector3D, 2>& outSegment) const;

    bool getVertexPosition(const std::string& bodyId,
                           const std::string& vertexId,
                           QVector3D& outVertex) const;

private:
    struct MeshCache {
        std::string bodyId;
        std::vector<QVector3D> vertices;
        std::unordered_map<std::string, QVector3D> vertexMap;
        std::unordered_set<std::string> pickableVertices;
        std::unordered_map<std::string, std::vector<QVector3D>> edgePolylines;
        std::unordered_map<std::string, std::vector<std::array<QVector3D, 3>>> faceMap;
        struct FaceTopologyCache {
            std::vector<std::string> edgeIds;
            std::vector<std::string> vertexIds;
        };
        std::unordered_map<std::string, FaceTopologyCache> faceTopology;
        std::vector<Triangle> triangles;
    };

    Ray buildRay(const QPoint& screenPos,
                 const QMatrix4x4& viewProjection,
                 const QSize& viewportSize) const;
    bool projectToScreen(const QMatrix4x4& viewProjection,
                         const QVector3D& worldPos,
                         const QSize& viewportSize,
                         QPointF* outPos) const;

    std::vector<MeshCache> meshes_;
};

} // namespace onecad::ui::selection

#endif // ONECAD_UI_SELECTION_MODELPICKERADAPTER_H
