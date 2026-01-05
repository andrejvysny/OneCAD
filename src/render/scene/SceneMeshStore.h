#ifndef ONECAD_RENDER_SCENE_SCENEMESHSTORE_H
#define ONECAD_RENDER_SCENE_SCENEMESHSTORE_H

#include <QMatrix4x4>
#include <QVector3D>
#include <cstddef>
#include <cstdint>
#include <string>
#include <unordered_map>
#include <vector>

namespace onecad::render {

class SceneMeshStore {
public:
    // Not thread-safe; access from the UI/renderer thread or add external synchronization.
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
        std::string faceId;
        std::vector<EdgePolyline> edges;
        std::vector<VertexSample> vertices;
    };

    struct Mesh {
        std::string bodyId;
        QMatrix4x4 modelMatrix;
        std::vector<QVector3D> vertices;
        std::vector<Triangle> triangles;
        std::unordered_map<std::string, FaceTopology> topologyByFace;
    };

    void setBodyMesh(const std::string& bodyId, Mesh mesh);
    bool removeBody(const std::string& bodyId);
    void clear();

    std::vector<Mesh> meshes() const;
    const Mesh* findMesh(const std::string& bodyId) const;
    [[nodiscard]] std::size_t size() const { return meshes_.size(); }
    [[nodiscard]] bool empty() const { return meshes_.empty(); }

    template <typename Func>
    void forEachMesh(Func&& func) const {
        for (const auto& [id, mesh] : meshes_) {
            (void)id;
            func(mesh);
        }
    }

private:
    std::unordered_map<std::string, Mesh> meshes_;
};

} // namespace onecad::render

#endif // ONECAD_RENDER_SCENE_SCENEMESHSTORE_H
