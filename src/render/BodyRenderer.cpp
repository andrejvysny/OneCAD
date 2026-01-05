#include "BodyRenderer.h"

#include <QDebug>
#include <QVector4D>
#include <algorithm>
#include <cmath>
#include <unordered_map>
#include <unordered_set>
#include <utility>

namespace onecad::render {

namespace {
constexpr float kAmbient = 0.25f;
constexpr float kPolygonOffsetFactor = 1.0f;
constexpr float kPolygonOffsetUnits = 1.0f;

const char* kTriangleVertexShader = R"(
#version 410 core
layout(location = 0) in vec3 aPos;
layout(location = 1) in vec3 aNormal;

uniform mat4 uMVP;

out vec3 vNormal;

void main() {
    gl_Position = uMVP * vec4(aPos, 1.0);
    vNormal = aNormal;
}
)";

const char* kTriangleFragmentShader = R"(
#version 410 core
in vec3 vNormal;

uniform vec3 uBaseColor;
uniform vec3 uLightDir;
uniform float uAlpha;
uniform float uAmbient;

out vec4 FragColor;

void main() {
    vec3 n = normalize(vNormal);
    vec3 lightDir = normalize(uLightDir);
    float diffuse = max(dot(n, lightDir), 0.0);
    float intensity = max(diffuse, uAmbient);
    vec3 color = uBaseColor * intensity;
    FragColor = vec4(color, uAlpha);
}
)";

const char* kEdgeVertexShader = R"(
#version 410 core
layout(location = 0) in vec3 aPos;

uniform mat4 uMVP;

void main() {
    gl_Position = uMVP * vec4(aPos, 1.0);
}
)";

const char* kEdgeFragmentShader = R"(
#version 410 core
uniform vec4 uColor;
out vec4 FragColor;

void main() {
    FragColor = uColor;
}
)";

QVector3D colorToVec(const QColor& color, float scale) {
    return QVector3D(color.redF() * scale,
                     color.greenF() * scale,
                     color.blueF() * scale);
}

} // namespace

BodyRenderer::BodyRenderer() = default;

BodyRenderer::~BodyRenderer() {
    cleanup();
}

void BodyRenderer::initialize() {
    if (m_initialized) {
        return;
    }

    initializeOpenGLFunctions();

    m_triangleShader = std::make_unique<QOpenGLShaderProgram>();
    if (!m_triangleShader->addShaderFromSourceCode(QOpenGLShader::Vertex, kTriangleVertexShader)) {
        qWarning() << "BodyRenderer: Triangle vertex shader compile failed:" << m_triangleShader->log();
        m_triangleShader.reset();
        return;
    }
    if (!m_triangleShader->addShaderFromSourceCode(QOpenGLShader::Fragment, kTriangleFragmentShader)) {
        qWarning() << "BodyRenderer: Triangle fragment shader compile failed:" << m_triangleShader->log();
        m_triangleShader.reset();
        return;
    }
    if (!m_triangleShader->link()) {
        qWarning() << "BodyRenderer: Triangle shader link failed:" << m_triangleShader->log();
        m_triangleShader.reset();
        return;
    }

    m_edgeShader = std::make_unique<QOpenGLShaderProgram>();
    if (!m_edgeShader->addShaderFromSourceCode(QOpenGLShader::Vertex, kEdgeVertexShader)) {
        qWarning() << "BodyRenderer: Edge vertex shader compile failed:" << m_edgeShader->log();
        m_edgeShader.reset();
        m_triangleShader.reset();
        return;
    }
    if (!m_edgeShader->addShaderFromSourceCode(QOpenGLShader::Fragment, kEdgeFragmentShader)) {
        qWarning() << "BodyRenderer: Edge fragment shader compile failed:" << m_edgeShader->log();
        m_edgeShader.reset();
        m_triangleShader.reset();
        return;
    }
    if (!m_edgeShader->link()) {
        qWarning() << "BodyRenderer: Edge shader link failed:" << m_edgeShader->log();
        m_edgeShader.reset();
        m_triangleShader.reset();
        return;
    }

    ensureBuffers(&m_mainBuffers, QOpenGLBuffer::StaticDraw);
    ensureBuffers(&m_previewBuffers, QOpenGLBuffer::DynamicDraw);

    m_initialized = true;
}

void BodyRenderer::cleanup() {
    if (!m_initialized) {
        return;
    }

    m_mainBuffers.triangles.vao.destroy();
    m_mainBuffers.triangles.vbo.destroy();
    m_mainBuffers.edges.vao.destroy();
    m_mainBuffers.edges.vbo.destroy();

    m_previewBuffers.triangles.vao.destroy();
    m_previewBuffers.triangles.vbo.destroy();
    m_previewBuffers.edges.vao.destroy();
    m_previewBuffers.edges.vbo.destroy();

    m_triangleShader.reset();
    m_edgeShader.reset();
    m_initialized = false;
}

void BodyRenderer::setMeshes(const std::vector<SceneMeshStore::Mesh>& meshes) {
    buildBuffers(meshes, &m_mainCpu);
    m_mainDirty = true;
}

void BodyRenderer::setMeshes(const SceneMeshStore& store) {
    buildBuffers(store, &m_mainCpu);
    m_mainDirty = true;
}

void BodyRenderer::setPreviewMeshes(const std::vector<SceneMeshStore::Mesh>& meshes) {
    buildBuffers(meshes, &m_previewCpu);
    m_previewDirty = true;
}

void BodyRenderer::clearPreview() {
    m_previewCpu.triangles.clear();
    m_previewCpu.edges.clear();
    m_previewDirty = true;
}

void BodyRenderer::render(const QMatrix4x4& viewProjection,
                          const QVector3D& lightDir,
                          const RenderStyle& style) {
    if (!m_initialized || !m_triangleShader || !m_edgeShader) {
        return;
    }

    if (m_mainDirty) {
        uploadBuffers(m_mainCpu, &m_mainBuffers);
        m_mainDirty = false;
    }
    if (m_previewDirty) {
        uploadBuffers(m_previewCpu, &m_previewBuffers);
        m_previewDirty = false;
    }

    renderBatch(m_mainBuffers, viewProjection, lightDir, style, -1.0f);
    if (m_previewBuffers.triangles.vertexCount > 0 || m_previewBuffers.edges.vertexCount > 0) {
        renderBatch(m_previewBuffers, viewProjection, lightDir, style, style.previewAlpha);
    }
}

void BodyRenderer::buildBuffers(const std::vector<SceneMeshStore::Mesh>& meshes,
                                CpuBuffers* outBuffers) const {
    if (!outBuffers) {
        return;
    }
    outBuffers->triangles.clear();
    outBuffers->edges.clear();

    for (const auto& mesh : meshes) {
        appendMeshBuffers(mesh, outBuffers);
    }
}

void BodyRenderer::buildBuffers(const SceneMeshStore& store, CpuBuffers* outBuffers) const {
    if (!outBuffers) {
        return;
    }
    outBuffers->triangles.clear();
    outBuffers->edges.clear();

    store.forEachMesh([&](const SceneMeshStore::Mesh& mesh) {
        appendMeshBuffers(mesh, outBuffers);
    });
}

void BodyRenderer::appendMeshBuffers(const SceneMeshStore::Mesh& mesh, CpuBuffers* outBuffers) const {
    if (!outBuffers) {
        return;
    }
    std::vector<QVector3D> transformedVertices;
    transformedVertices.reserve(mesh.vertices.size());
    for (const auto& v : mesh.vertices) {
        QVector4D transformed = mesh.modelMatrix * QVector4D(v, 1.0f);
        transformedVertices.emplace_back(transformed.x(), transformed.y(), transformed.z());
    }

    for (const auto& tri : mesh.triangles) {
        if (tri.i0 >= transformedVertices.size() ||
            tri.i1 >= transformedVertices.size() ||
            tri.i2 >= transformedVertices.size()) {
            continue;
        }
        const QVector3D& v0 = transformedVertices[tri.i0];
        const QVector3D& v1 = transformedVertices[tri.i1];
        const QVector3D& v2 = transformedVertices[tri.i2];
        QVector3D normal = QVector3D::crossProduct(v1 - v0, v2 - v0);
        if (normal.lengthSquared() < 1e-8f) {
            normal = QVector3D(0.0f, 0.0f, 1.0f);
        } else {
            normal.normalize();
        }
        const QVector3D verts[3] = {v0, v1, v2};
        for (const auto& pos : verts) {
            outBuffers->triangles.push_back(pos.x());
            outBuffers->triangles.push_back(pos.y());
            outBuffers->triangles.push_back(pos.z());
            outBuffers->triangles.push_back(normal.x());
            outBuffers->triangles.push_back(normal.y());
            outBuffers->triangles.push_back(normal.z());
        }
    }

    std::unordered_set<std::string> seenEdges;
    if (!mesh.topologyByFace.empty()) {
        for (const auto& [faceId, topo] : mesh.topologyByFace) {
            (void)faceId;
            for (const auto& edge : topo.edges) {
                if (seenEdges.find(edge.edgeId) != seenEdges.end()) {
                    continue;
                }
                seenEdges.insert(edge.edgeId);
                if (edge.points.size() < 2) {
                    continue;
                }
                for (size_t i = 0; i + 1 < edge.points.size(); ++i) {
                    QVector4D p0 = mesh.modelMatrix * QVector4D(edge.points[i], 1.0f);
                    QVector4D p1 = mesh.modelMatrix * QVector4D(edge.points[i + 1], 1.0f);
                    outBuffers->edges.push_back(p0.x());
                    outBuffers->edges.push_back(p0.y());
                    outBuffers->edges.push_back(p0.z());
                    outBuffers->edges.push_back(p1.x());
                    outBuffers->edges.push_back(p1.y());
                    outBuffers->edges.push_back(p1.z());
                }
            }
        }
    } else {
        struct PairHash {
            size_t operator()(const std::pair<int, int>& value) const noexcept {
                size_t seed = std::hash<int>{}(value.first);
                seed ^= std::hash<int>{}(value.second) + 0x9e3779b9 + (seed << 6) + (seed >> 2);
                return seed;
            }
        };
        auto makeEdgeKey = [](int a, int b) {
            if (a > b) {
                std::swap(a, b);
            }
            return std::make_pair(a, b);
        };
        std::unordered_set<std::pair<int, int>, PairHash> seenEdgesLocal;
        for (const auto& tri : mesh.triangles) {
            int indices[3] = {
                static_cast<int>(tri.i0),
                static_cast<int>(tri.i1),
                static_cast<int>(tri.i2)
            };
            for (int i = 0; i < 3; ++i) {
                int a = indices[i];
                int b = indices[(i + 1) % 3];
                auto key = makeEdgeKey(a, b);
                if (!seenEdgesLocal.insert(key).second) {
                    continue;
                }
                if (a < 0 || b < 0 ||
                    static_cast<size_t>(a) >= transformedVertices.size() ||
                    static_cast<size_t>(b) >= transformedVertices.size()) {
                    continue;
                }
                const QVector3D& p0 = transformedVertices[a];
                const QVector3D& p1 = transformedVertices[b];
                outBuffers->edges.push_back(p0.x());
                outBuffers->edges.push_back(p0.y());
                outBuffers->edges.push_back(p0.z());
                outBuffers->edges.push_back(p1.x());
                outBuffers->edges.push_back(p1.y());
                outBuffers->edges.push_back(p1.z());
            }
        }
    }
}

void BodyRenderer::ensureBuffers(RenderBuffers* buffers, QOpenGLBuffer::UsagePattern usage) {
    if (!buffers) {
        return;
    }

    if (!buffers->triangles.vao.isCreated()) {
        buffers->triangles.vao.create();
    }
    if (!buffers->triangles.vbo.isCreated()) {
        buffers->triangles.vbo.create();
        buffers->triangles.vbo.setUsagePattern(usage);
    }
    if (!buffers->edges.vao.isCreated()) {
        buffers->edges.vao.create();
    }
    if (!buffers->edges.vbo.isCreated()) {
        buffers->edges.vbo.create();
        buffers->edges.vbo.setUsagePattern(usage);
    }
}

void BodyRenderer::uploadBuffers(const CpuBuffers& cpu, RenderBuffers* buffers) {
    if (!buffers) {
        return;
    }
    if (!buffers->triangles.vbo.isCreated() || !buffers->edges.vbo.isCreated() ||
        !buffers->triangles.vao.isCreated() || !buffers->edges.vao.isCreated()) {
        return;
    }

    buffers->triangles.vertexCount = 0;
    if (!cpu.triangles.empty()) {
        buffers->triangles.vertexCount = static_cast<int>(cpu.triangles.size() / 6);
        buffers->triangles.vao.bind();
        buffers->triangles.vbo.bind();
        buffers->triangles.vbo.allocate(cpu.triangles.data(),
                                        static_cast<int>(cpu.triangles.size() * sizeof(float)));
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 6 * sizeof(float),
                              reinterpret_cast<void*>(0));
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 3, GL_FLOAT, GL_FALSE, 6 * sizeof(float),
                              reinterpret_cast<void*>(3 * sizeof(float)));
        buffers->triangles.vbo.release();
        buffers->triangles.vao.release();
    }

    buffers->edges.vertexCount = 0;
    if (!cpu.edges.empty()) {
        buffers->edges.vertexCount = static_cast<int>(cpu.edges.size() / 3);
        buffers->edges.vao.bind();
        buffers->edges.vbo.bind();
        buffers->edges.vbo.allocate(cpu.edges.data(),
                                    static_cast<int>(cpu.edges.size() * sizeof(float)));
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 3 * sizeof(float),
                              reinterpret_cast<void*>(0));
        buffers->edges.vbo.release();
        buffers->edges.vao.release();
    }
}

void BodyRenderer::renderBatch(RenderBuffers& buffers,
                               const QMatrix4x4& viewProjection,
                               const QVector3D& lightDir,
                               const RenderStyle& style,
                               float alphaOverride) {
    const float colorScale = style.ghosted ? style.ghostFactor : 1.0f;
    const QVector3D baseColor = colorToVec(style.baseColor, colorScale);
    const QVector3D edgeColor = colorToVec(style.edgeColor, colorScale);
    const float baseAlpha = alphaOverride >= 0.0f ? alphaOverride : style.baseAlpha;
    const float edgeAlpha = alphaOverride >= 0.0f ? alphaOverride : style.edgeAlpha;

    if (buffers.triangles.vertexCount > 0) {
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glDisable(GL_CULL_FACE);
        glEnable(GL_POLYGON_OFFSET_FILL);
        glPolygonOffset(kPolygonOffsetFactor, kPolygonOffsetUnits);

        if (baseAlpha < 0.999f) {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        } else {
            glDisable(GL_BLEND);
        }

        m_triangleShader->bind();
        m_triangleShader->setUniformValue("uMVP", viewProjection);
        m_triangleShader->setUniformValue("uBaseColor", baseColor);
        m_triangleShader->setUniformValue("uLightDir", lightDir);
        m_triangleShader->setUniformValue("uAlpha", baseAlpha);
        m_triangleShader->setUniformValue("uAmbient", kAmbient);

        buffers.triangles.vao.bind();
        glDrawArrays(GL_TRIANGLES, 0, buffers.triangles.vertexCount);
        buffers.triangles.vao.release();

        m_triangleShader->release();

        glDisable(GL_POLYGON_OFFSET_FILL);
        glDisable(GL_BLEND);
    }

    if (style.drawEdges && buffers.edges.vertexCount > 0) {
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LEQUAL);

        if (edgeAlpha < 0.999f) {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        } else {
            glDisable(GL_BLEND);
        }

        m_edgeShader->bind();
        m_edgeShader->setUniformValue("uMVP", viewProjection);
        m_edgeShader->setUniformValue("uColor", QVector4D(edgeColor, edgeAlpha));

        glLineWidth(1.5f);
        buffers.edges.vao.bind();
        glDrawArrays(GL_LINES, 0, buffers.edges.vertexCount);
        buffers.edges.vao.release();
        glLineWidth(1.0f);

        m_edgeShader->release();

        glDisable(GL_BLEND);
    }
}

} // namespace onecad::render
