#ifndef ONECAD_RENDER_BODYRENDERER_H
#define ONECAD_RENDER_BODYRENDERER_H

#include <QColor>
#include <QMatrix4x4>
#include <QOpenGLBuffer>
#include <QOpenGLFunctions>
#include <QOpenGLShaderProgram>
#include <QOpenGLVertexArrayObject>
#include <QVector3D>
#include <memory>
#include <vector>

#include "scene/SceneMeshStore.h"

namespace onecad::render {

class BodyRenderer : protected QOpenGLFunctions {
public:
    struct RenderStyle {
        QColor baseColor{200, 200, 200};
        QColor edgeColor{0, 0, 0};
        float baseAlpha = 1.0f;
        float edgeAlpha = 1.0f;
        float previewAlpha = 0.35f;
        float ghostFactor = 1.0f;
        bool ghosted = false;
        bool drawEdges = true;
    };

    BodyRenderer();
    ~BodyRenderer();
    BodyRenderer(const BodyRenderer&) = delete;
    BodyRenderer& operator=(const BodyRenderer&) = delete;
    BodyRenderer(BodyRenderer&&) = delete;
    BodyRenderer& operator=(BodyRenderer&&) = delete;

    void initialize();
    void cleanup();
    bool isInitialized() const { return m_initialized; }

    void setMeshes(const SceneMeshStore& store);
    void setMeshes(const std::vector<SceneMeshStore::Mesh>& meshes);
    void setPreviewMeshes(const std::vector<SceneMeshStore::Mesh>& meshes);
    void clearPreview();

    void render(const QMatrix4x4& viewProjection,
                const QVector3D& lightDir,
                const RenderStyle& style);

private:
    struct CpuBuffers {
        std::vector<float> triangles;
        std::vector<float> edges;
    };

    struct DrawBuffers {
        QOpenGLVertexArrayObject vao;
        QOpenGLBuffer vbo{QOpenGLBuffer::VertexBuffer};
        int vertexCount = 0;
    };

    struct RenderBuffers {
        DrawBuffers triangles;
        DrawBuffers edges;
    };

    void buildBuffers(const std::vector<SceneMeshStore::Mesh>& meshes, CpuBuffers* outBuffers) const;
    void buildBuffers(const SceneMeshStore& store, CpuBuffers* outBuffers) const;
    void appendMeshBuffers(const SceneMeshStore::Mesh& mesh, CpuBuffers* outBuffers) const;
    void ensureBuffers(RenderBuffers* buffers, QOpenGLBuffer::UsagePattern usage);
    void uploadBuffers(const CpuBuffers& cpu, RenderBuffers* buffers);
    void renderBatch(RenderBuffers& buffers,
                     const QMatrix4x4& viewProjection,
                     const QVector3D& lightDir,
                     const RenderStyle& style,
                     float alphaOverride);

    std::unique_ptr<QOpenGLShaderProgram> m_triangleShader;
    std::unique_ptr<QOpenGLShaderProgram> m_edgeShader;
    RenderBuffers m_mainBuffers;
    RenderBuffers m_previewBuffers;
    CpuBuffers m_mainCpu;
    CpuBuffers m_previewCpu;
    bool m_mainDirty = false;
    bool m_previewDirty = false;
    bool m_initialized = false;
};

} // namespace onecad::render

#endif // ONECAD_RENDER_BODYRENDERER_H
