/**
 * @file SketchRenderer.cpp
 * @brief OpenGL-based sketch rendering implementation
 *
 * Renders sketch geometry (lines, arcs, circles, points) and constraints.
 * Uses VBO batching for performance with adaptive arc tessellation.
 */
#include "SketchRenderer.h"

#include "Sketch.h"
#include "SketchArc.h"
#include "SketchCircle.h"
#include "SketchLine.h"
#include "SketchPoint.h"

#include <QMatrix4x4>
#include <QOpenGLBuffer>
#include <QOpenGLFunctions>
#include <QOpenGLShaderProgram>
#include <QOpenGLVertexArrayObject>
#include <QVector3D>
#include <QVector4D>

#include <algorithm>
#include <cmath>
#include <iostream>
#include <memory>
#include <numbers>
#include <unordered_set>

namespace onecad::core::sketch {

namespace {

// GLSL 410 core for macOS Metal compatibility
constexpr const char* kLineVertexShader = R"(
#version 410 core
layout(location = 0) in vec2 aPos;
layout(location = 1) in vec4 aColor;

uniform mat4 uMVP;

out vec4 vColor;

void main() {
    gl_Position = uMVP * vec4(aPos, 0.0, 1.0);
    vColor = aColor;
}
)";

constexpr const char* kLineFragmentShader = R"(
#version 410 core
in vec4 vColor;
out vec4 FragColor;

void main() {
    FragColor = vColor;
}
)";

constexpr const char* kPointVertexShader = R"(
#version 410 core
layout(location = 0) in vec2 aPos;
layout(location = 1) in vec4 aColor;
layout(location = 2) in float aSize;

uniform mat4 uMVP;

out vec4 vColor;

void main() {
    gl_Position = uMVP * vec4(aPos, 0.0, 1.0);
    gl_PointSize = aSize;
    vColor = aColor;
}
)";

constexpr const char* kPointFragmentShader = R"(
#version 410 core
in vec4 vColor;
out vec4 FragColor;

void main() {
    // Circular point with smooth edges
    vec2 coord = gl_PointCoord - vec2(0.5);
    float dist = length(coord);
    if (dist > 0.5) discard;

    float alpha = 1.0 - smoothstep(0.4, 0.5, dist);
    FragColor = vec4(vColor.rgb, vColor.a * alpha);
}
)";

Vec3d colorForState(SelectionState state, bool isConstruction, bool hasError,
                    const SketchColors& colors) {
    if (hasError) {
        return colors.errorGeometry;
    }
    switch (state) {
        case SelectionState::Selected:
        case SelectionState::Dragging:
            return colors.selectedGeometry;
        case SelectionState::Hover:
            return {colors.selectedGeometry.x * 0.7 + colors.normalGeometry.x * 0.3,
                    colors.selectedGeometry.y * 0.7 + colors.normalGeometry.y * 0.3,
                    colors.selectedGeometry.z * 0.7 + colors.normalGeometry.z * 0.3};
        default:
            return isConstruction ? colors.constructionGeometry : colors.normalGeometry;
    }
}

float lineWidthForState(SelectionState state, bool isConstruction,
                        const SketchRenderStyle& style) {
    switch (state) {
        case SelectionState::Selected:
        case SelectionState::Dragging:
            return style.selectedLineWidth;
        case SelectionState::Hover:
            return (style.normalLineWidth + style.selectedLineWidth) / 2.0f;
        default:
            return isConstruction ? style.constructionLineWidth : style.normalLineWidth;
    }
}

void appendSegment(std::vector<float>& data, const Vec2d& p1, const Vec2d& p2, const Vec3d& color) {
    data.push_back(static_cast<float>(p1.x));
    data.push_back(static_cast<float>(p1.y));
    data.push_back(static_cast<float>(color.x));
    data.push_back(static_cast<float>(color.y));
    data.push_back(static_cast<float>(color.z));
    data.push_back(1.0f);

    data.push_back(static_cast<float>(p2.x));
    data.push_back(static_cast<float>(p2.y));
    data.push_back(static_cast<float>(color.x));
    data.push_back(static_cast<float>(color.y));
    data.push_back(static_cast<float>(color.z));
    data.push_back(1.0f);
}

void appendSolidPolyline(std::vector<float>& data, const std::vector<Vec2d>& vertices,
                         const Vec3d& color) {
    if (vertices.size() < 2) {
        return;
    }
    for (size_t i = 0; i + 1 < vertices.size(); ++i) {
        appendSegment(data, vertices[i], vertices[i + 1], color);
    }
}

void appendDashedPolyline(std::vector<float>& data, const std::vector<Vec2d>& vertices,
                          const Vec3d& color, double dashLength, double gapLength) {
    if (vertices.size() < 2) {
        return;
    }
    if (dashLength <= 0.0 || gapLength < 0.0) {
        appendSolidPolyline(data, vertices, color);
        return;
    }

    double patternLength = dashLength + gapLength;
    if (patternLength <= 0.0) {
        appendSolidPolyline(data, vertices, color);
        return;
    }

    double patternPos = 0.0;
    for (size_t i = 0; i + 1 < vertices.size(); ++i) {
        const auto& p1 = vertices[i];
        const auto& p2 = vertices[i + 1];
        double dx = p2.x - p1.x;
        double dy = p2.y - p1.y;
        double segLen = std::hypot(dx, dy);
        if (segLen < 1e-12) {
            continue;
        }
        double invLen = 1.0 / segLen;
        double segPos = 0.0;

        while (segPos < segLen) {
            double remainingInPattern = patternLength - patternPos;
            double step = std::min(remainingInPattern, segLen - segPos);

            if (patternPos < dashLength && step > 0.0) {
                double startT = segPos * invLen;
                double endT = (segPos + step) * invLen;
                Vec2d start{p1.x + dx * startT, p1.y + dy * startT};
                Vec2d end{p1.x + dx * endT, p1.y + dy * endT};
                appendSegment(data, start, end, color);
            }

            segPos += step;
            patternPos += step;
            if (patternPos >= patternLength) {
                patternPos = std::fmod(patternPos, patternLength);
            }
        }
    }
}

QMatrix4x4 buildSketchModelMatrix(const SketchPlane& plane) {
    QVector3D origin(plane.origin.x, plane.origin.y, plane.origin.z);
    QVector3D normal(plane.normal.x, plane.normal.y, plane.normal.z);
    QVector3D xAxis(plane.xAxis.x, plane.xAxis.y, plane.xAxis.z);
    QVector3D yAxis(plane.yAxis.x, plane.yAxis.y, plane.yAxis.z);

    if (normal.lengthSquared() < 1e-12f) {
        normal = QVector3D::crossProduct(xAxis, yAxis);
    }
    if (normal.lengthSquared() < 1e-12f) {
        normal = QVector3D(0.0f, 0.0f, 1.0f);
    }
    normal.normalize();

    if (xAxis.lengthSquared() < 1e-12f) {
        xAxis = QVector3D::crossProduct(yAxis, normal);
    }
    if (xAxis.lengthSquared() < 1e-12f) {
        xAxis = (std::abs(normal.z()) < 0.9f) ? QVector3D::crossProduct(normal, QVector3D(0, 0, 1))
                                              : QVector3D::crossProduct(normal, QVector3D(0, 1, 0));
    }

    xAxis -= normal * QVector3D::dotProduct(normal, xAxis);
    if (xAxis.lengthSquared() < 1e-12f) {
        xAxis = (std::abs(normal.z()) < 0.9f) ? QVector3D::crossProduct(normal, QVector3D(0, 0, 1))
                                              : QVector3D::crossProduct(normal, QVector3D(0, 1, 0));
    }
    xAxis.normalize();

    QVector3D yOrtho = QVector3D::crossProduct(normal, xAxis).normalized();

    QMatrix4x4 model;
    model.setColumn(0, QVector4D(xAxis, 0.0f));
    model.setColumn(1, QVector4D(yOrtho, 0.0f));
    model.setColumn(2, QVector4D(normal, 0.0f));
    model.setColumn(3, QVector4D(origin, 1.0f));
    return model;
}

} // anonymous namespace

// Implementation class (PIMPL)
class SketchRendererImpl : protected QOpenGLFunctions {
public:
    SketchRendererImpl() = default;
    ~SketchRendererImpl() { cleanup(); }

    bool initialize();
    void cleanup();
    void buildVBOs(const std::vector<EntityRenderData>& entities,
                   const SketchRenderStyle& style,
                   const std::unordered_map<EntityID, SelectionState>& selections,
                   EntityID hoverEntity,
                   const Viewport& viewport,
                   double pixelScale,
                   const std::vector<ConstraintRenderData>& constraints,
                   bool snapActive,
                   const Vec2d& snapPos,
                   float snapSize,
                   const Vec3d& snapColor);
    void render(const QMatrix4x4& mvp, const SketchRenderStyle& style);
    void renderPoints(const QMatrix4x4& mvp);
    void renderPreview(const QMatrix4x4& mvp, const std::vector<Vec2d>& vertices,
                       const Vec3d& color, float lineWidth);

    bool initialized_ = false;

    // Line rendering
    std::unique_ptr<QOpenGLShaderProgram> lineShader_;
    std::unique_ptr<QOpenGLBuffer> lineVBO_;
    std::unique_ptr<QOpenGLVertexArrayObject> lineVAO_;
    int lineVertexCount_ = 0;
    std::unique_ptr<QOpenGLBuffer> constructionLineVBO_;
    std::unique_ptr<QOpenGLVertexArrayObject> constructionLineVAO_;
    int constructionLineVertexCount_ = 0;
    std::unique_ptr<QOpenGLBuffer> highlightLineVBO_;
    std::unique_ptr<QOpenGLVertexArrayObject> highlightLineVAO_;
    int highlightLineVertexCount_ = 0;

    // Point rendering
    std::unique_ptr<QOpenGLShaderProgram> pointShader_;
    std::unique_ptr<QOpenGLBuffer> pointVBO_;
    std::unique_ptr<QOpenGLVertexArrayObject> pointVAO_;
    int pointVertexCount_ = 0;

    // Preview line rendering
    std::unique_ptr<QOpenGLBuffer> previewVBO_;
    std::unique_ptr<QOpenGLVertexArrayObject> previewVAO_;
};

bool SketchRendererImpl::initialize() {
    if (initialized_) return true;

    initializeOpenGLFunctions();

    // Helper to cleanup on failure
    auto cleanupOnFailure = [this]() {
        lineShader_.reset();
        pointShader_.reset();
        if (lineVAO_ && lineVAO_->isCreated()) lineVAO_->destroy();
        if (lineVBO_ && lineVBO_->isCreated()) lineVBO_->destroy();
        if (constructionLineVAO_ && constructionLineVAO_->isCreated()) constructionLineVAO_->destroy();
        if (constructionLineVBO_ && constructionLineVBO_->isCreated()) constructionLineVBO_->destroy();
        if (highlightLineVAO_ && highlightLineVAO_->isCreated()) highlightLineVAO_->destroy();
        if (highlightLineVBO_ && highlightLineVBO_->isCreated()) highlightLineVBO_->destroy();
        if (pointVAO_ && pointVAO_->isCreated()) pointVAO_->destroy();
        if (pointVBO_ && pointVBO_->isCreated()) pointVBO_->destroy();
        if (previewVAO_ && previewVAO_->isCreated()) previewVAO_->destroy();
        if (previewVBO_ && previewVBO_->isCreated()) previewVBO_->destroy();
        lineVAO_.reset();
        lineVBO_.reset();
        constructionLineVAO_.reset();
        constructionLineVBO_.reset();
        highlightLineVAO_.reset();
        highlightLineVBO_.reset();
        pointVAO_.reset();
        pointVBO_.reset();
        previewVAO_.reset();
        previewVBO_.reset();
    };

    // Line shader
    lineShader_ = std::make_unique<QOpenGLShaderProgram>();
    if (!lineShader_->addShaderFromSourceCode(QOpenGLShader::Vertex, kLineVertexShader)) {
        cleanupOnFailure();
        return false;
    }
    if (!lineShader_->addShaderFromSourceCode(QOpenGLShader::Fragment, kLineFragmentShader)) {
        cleanupOnFailure();
        return false;
    }
    if (!lineShader_->link()) {
        cleanupOnFailure();
        return false;
    }

    // Point shader
    pointShader_ = std::make_unique<QOpenGLShaderProgram>();
    if (!pointShader_->addShaderFromSourceCode(QOpenGLShader::Vertex, kPointVertexShader)) {
        cleanupOnFailure();
        return false;
    }
    if (!pointShader_->addShaderFromSourceCode(QOpenGLShader::Fragment, kPointFragmentShader)) {
        cleanupOnFailure();
        return false;
    }
    if (!pointShader_->link()) {
        cleanupOnFailure();
        return false;
    }

    // Create buffers and VAOs
    lineVAO_ = std::make_unique<QOpenGLVertexArrayObject>();
    lineVBO_ = std::make_unique<QOpenGLBuffer>(QOpenGLBuffer::VertexBuffer);
    if (!lineVAO_->create() || !lineVBO_->create()) {
        cleanupOnFailure();
        return false;
    }

    constructionLineVAO_ = std::make_unique<QOpenGLVertexArrayObject>();
    constructionLineVBO_ = std::make_unique<QOpenGLBuffer>(QOpenGLBuffer::VertexBuffer);
    if (!constructionLineVAO_->create() || !constructionLineVBO_->create()) {
        cleanupOnFailure();
        return false;
    }

    highlightLineVAO_ = std::make_unique<QOpenGLVertexArrayObject>();
    highlightLineVBO_ = std::make_unique<QOpenGLBuffer>(QOpenGLBuffer::VertexBuffer);
    if (!highlightLineVAO_->create() || !highlightLineVBO_->create()) {
        cleanupOnFailure();
        return false;
    }

    pointVAO_ = std::make_unique<QOpenGLVertexArrayObject>();
    pointVBO_ = std::make_unique<QOpenGLBuffer>(QOpenGLBuffer::VertexBuffer);
    if (!pointVAO_->create() || !pointVBO_->create()) {
        cleanupOnFailure();
        return false;
    }

    previewVAO_ = std::make_unique<QOpenGLVertexArrayObject>();
    previewVBO_ = std::make_unique<QOpenGLBuffer>(QOpenGLBuffer::VertexBuffer);
    if (!previewVAO_->create() || !previewVBO_->create()) {
        cleanupOnFailure();
        return false;
    }

    lineVBO_->setUsagePattern(QOpenGLBuffer::DynamicDraw);
    constructionLineVBO_->setUsagePattern(QOpenGLBuffer::DynamicDraw);
    highlightLineVBO_->setUsagePattern(QOpenGLBuffer::DynamicDraw);
    pointVBO_->setUsagePattern(QOpenGLBuffer::DynamicDraw);
    previewVBO_->setUsagePattern(QOpenGLBuffer::DynamicDraw);

    initialized_ = true;
    return true;
}

void SketchRendererImpl::cleanup() {
    if (!initialized_) return;

    if (lineVAO_ && lineVAO_->isCreated()) lineVAO_->destroy();
    if (lineVBO_ && lineVBO_->isCreated()) lineVBO_->destroy();
    if (constructionLineVAO_ && constructionLineVAO_->isCreated()) constructionLineVAO_->destroy();
    if (constructionLineVBO_ && constructionLineVBO_->isCreated()) constructionLineVBO_->destroy();
    if (highlightLineVAO_ && highlightLineVAO_->isCreated()) highlightLineVAO_->destroy();
    if (highlightLineVBO_ && highlightLineVBO_->isCreated()) highlightLineVBO_->destroy();
    if (pointVAO_ && pointVAO_->isCreated()) pointVAO_->destroy();
    if (pointVBO_ && pointVBO_->isCreated()) pointVBO_->destroy();
    if (previewVAO_ && previewVAO_->isCreated()) previewVAO_->destroy();
    if (previewVBO_ && previewVBO_->isCreated()) previewVBO_->destroy();

    lineShader_.reset();
    pointShader_.reset();

    initialized_ = false;
}

void SketchRendererImpl::buildVBOs(
    const std::vector<EntityRenderData>& entities,
    const SketchRenderStyle& style,
    const std::unordered_map<EntityID, SelectionState>& selections,
    EntityID hoverEntity,
    const Viewport& viewport,
    double pixelScale,
    const std::vector<ConstraintRenderData>& constraints,
    bool snapActive,
    const Vec2d& snapPos,
    float snapSize,
    const Vec3d& snapColor) {

    // Line data: pos(2) + color(4) = 6 floats per vertex
    std::vector<float> lineData;
    std::vector<float> constructionLineData;
    std::vector<float> highlightLineData;
    // Point data: pos(2) + color(4) + size(1) = 7 floats per vertex
    std::vector<float> pointData;

    double dashLength = style.dashLength * std::max(pixelScale, 1e-9);
    double gapLength = style.gapLength * std::max(pixelScale, 1e-9);

    for (const auto& entity : entities) {
        if (viewport.size.x > 0.0 && viewport.size.y > 0.0) {
            if (!viewport.intersects(entity.bounds[0], entity.bounds[1])) {
                continue;
            }
        }

        SelectionState selState = SelectionState::None;
        if (auto it = selections.find(entity.id); it != selections.end()) {
            selState = it->second;
        }
        if (entity.id == hoverEntity && selState == SelectionState::None) {
            selState = SelectionState::Hover;
        }

        Vec3d color = colorForState(selState, entity.isConstruction, entity.hasError,
                                     style.colors);

        if (entity.type == EntityType::Point) {
            if (entity.vertices.empty()) continue;
            const auto& p = entity.vertices[0];
            float size = (selState == SelectionState::Selected || selState == SelectionState::Dragging)
                             ? style.selectedPointSize
                             : style.pointSize;
            pointData.push_back(static_cast<float>(p.x));
            pointData.push_back(static_cast<float>(p.y));
            pointData.push_back(static_cast<float>(color.x));
            pointData.push_back(static_cast<float>(color.y));
            pointData.push_back(static_cast<float>(color.z));
            pointData.push_back(1.0f);  // alpha
            pointData.push_back(size);
        } else {
            // Lines, arcs, circles - render as line segments
            bool isHighlight = (selState == SelectionState::Selected ||
                                selState == SelectionState::Dragging ||
                                selState == SelectionState::Hover);
            if (entity.isConstruction) {
                appendDashedPolyline(constructionLineData, entity.vertices, color,
                                     dashLength, gapLength);
            } else if (isHighlight) {
                appendSolidPolyline(highlightLineData, entity.vertices, color);
            } else {
                appendSolidPolyline(lineData, entity.vertices, color);
            }
        }
    }

    for (const auto& icon : constraints) {
        if (viewport.size.x > 0.0 && viewport.size.y > 0.0) {
            if (!viewport.contains(icon.position)) {
                continue;
            }
        }

        Vec3d color = icon.isConflicting ? style.colors.conflictHighlight : style.colors.constraintIcon;
        pointData.push_back(static_cast<float>(icon.position.x));
        pointData.push_back(static_cast<float>(icon.position.y));
        pointData.push_back(static_cast<float>(color.x));
        pointData.push_back(static_cast<float>(color.y));
        pointData.push_back(static_cast<float>(color.z));
        pointData.push_back(1.0f);
        pointData.push_back(style.constraintIconSize);
    }

    if (snapActive) {
        pointData.push_back(static_cast<float>(snapPos.x));
        pointData.push_back(static_cast<float>(snapPos.y));
        pointData.push_back(static_cast<float>(snapColor.x));
        pointData.push_back(static_cast<float>(snapColor.y));
        pointData.push_back(static_cast<float>(snapColor.z));
        pointData.push_back(1.0f);
        pointData.push_back(snapSize);
    }

    // Upload line data
    lineVertexCount_ = static_cast<int>(lineData.size() / 6);
    if (!lineData.empty()) {
        lineVAO_->bind();
        lineVBO_->bind();
        lineVBO_->allocate(lineData.data(), static_cast<int>(lineData.size() * sizeof(float)));

        // Position (2 floats)
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 6 * sizeof(float), nullptr);
        // Color (4 floats)
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 6 * sizeof(float),
                              reinterpret_cast<void*>(2 * sizeof(float)));

        lineVBO_->release();
        lineVAO_->release();
    }

    constructionLineVertexCount_ = static_cast<int>(constructionLineData.size() / 6);
    if (!constructionLineData.empty()) {
        constructionLineVAO_->bind();
        constructionLineVBO_->bind();
        constructionLineVBO_->allocate(constructionLineData.data(),
                                       static_cast<int>(constructionLineData.size() * sizeof(float)));

        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 6 * sizeof(float), nullptr);
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 6 * sizeof(float),
                              reinterpret_cast<void*>(2 * sizeof(float)));

        constructionLineVBO_->release();
        constructionLineVAO_->release();
    }

    highlightLineVertexCount_ = static_cast<int>(highlightLineData.size() / 6);
    if (!highlightLineData.empty()) {
        highlightLineVAO_->bind();
        highlightLineVBO_->bind();
        highlightLineVBO_->allocate(highlightLineData.data(),
                                   static_cast<int>(highlightLineData.size() * sizeof(float)));

        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 6 * sizeof(float), nullptr);
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 6 * sizeof(float),
                              reinterpret_cast<void*>(2 * sizeof(float)));

        highlightLineVBO_->release();
        highlightLineVAO_->release();
    }

    // Upload point data
    pointVertexCount_ = static_cast<int>(pointData.size() / 7);
    if (!pointData.empty()) {
        pointVAO_->bind();
        pointVBO_->bind();
        pointVBO_->allocate(pointData.data(), static_cast<int>(pointData.size() * sizeof(float)));

        // Position (2 floats)
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 7 * sizeof(float), nullptr);
        // Color (4 floats)
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 7 * sizeof(float),
                              reinterpret_cast<void*>(2 * sizeof(float)));
        // Size (1 float)
        glEnableVertexAttribArray(2);
        glVertexAttribPointer(2, 1, GL_FLOAT, GL_FALSE, 7 * sizeof(float),
                              reinterpret_cast<void*>(6 * sizeof(float)));

        pointVBO_->release();
        pointVAO_->release();
    }
}

void SketchRendererImpl::render(const QMatrix4x4& mvp, const SketchRenderStyle& style) {
    if (!initialized_) return;

    lineShader_->bind();
    lineShader_->setUniformValue("uMVP", mvp);

    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    glEnable(GL_LINE_SMOOTH);

    if (constructionLineVertexCount_ > 0) {
        glLineWidth(style.constructionLineWidth);
        constructionLineVAO_->bind();
        glDrawArrays(GL_LINES, 0, constructionLineVertexCount_);
        constructionLineVAO_->release();
    }

    if (lineVertexCount_ > 0) {
        glLineWidth(style.normalLineWidth);
        lineVAO_->bind();
        glDrawArrays(GL_LINES, 0, lineVertexCount_);
        lineVAO_->release();
    }

    if (highlightLineVertexCount_ > 0) {
        glLineWidth(style.selectedLineWidth);
        highlightLineVAO_->bind();
        glDrawArrays(GL_LINES, 0, highlightLineVertexCount_);
        highlightLineVAO_->release();
    }

    glDisable(GL_LINE_SMOOTH);
    glDisable(GL_BLEND);
    glLineWidth(1.0f);

    lineShader_->release();
}

void SketchRendererImpl::renderPoints(const QMatrix4x4& mvp) {
    if (!initialized_ || pointVertexCount_ == 0) return;

    pointShader_->bind();
    pointShader_->setUniformValue("uMVP", mvp);

    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    glEnable(GL_PROGRAM_POINT_SIZE);

    pointVAO_->bind();
    glDrawArrays(GL_POINTS, 0, pointVertexCount_);
    pointVAO_->release();

    glDisable(GL_PROGRAM_POINT_SIZE);
    glDisable(GL_BLEND);

    pointShader_->release();
}

void SketchRendererImpl::renderPreview(const QMatrix4x4& mvp,
                                        const std::vector<Vec2d>& vertices,
                                        const Vec3d& color, float lineWidth) {
    if (!initialized_ || vertices.size() < 2) return;

    std::vector<float> data;
    for (size_t i = 0; i + 1 < vertices.size(); ++i) {
        const auto& p1 = vertices[i];
        const auto& p2 = vertices[i + 1];
        data.push_back(static_cast<float>(p1.x));
        data.push_back(static_cast<float>(p1.y));
        data.push_back(static_cast<float>(color.x));
        data.push_back(static_cast<float>(color.y));
        data.push_back(static_cast<float>(color.z));
        data.push_back(0.7f);  // semi-transparent preview
        data.push_back(static_cast<float>(p2.x));
        data.push_back(static_cast<float>(p2.y));
        data.push_back(static_cast<float>(color.x));
        data.push_back(static_cast<float>(color.y));
        data.push_back(static_cast<float>(color.z));
        data.push_back(0.7f);
    }

    previewVAO_->bind();
    previewVBO_->bind();
    previewVBO_->allocate(data.data(), static_cast<int>(data.size() * sizeof(float)));

    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 6 * sizeof(float), nullptr);
    glEnableVertexAttribArray(1);
    glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 6 * sizeof(float),
                          reinterpret_cast<void*>(2 * sizeof(float)));

    lineShader_->bind();
    lineShader_->setUniformValue("uMVP", mvp);

    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    glLineWidth(lineWidth);

    glDrawArrays(GL_LINES, 0, static_cast<int>(data.size() / 6));

    glDisable(GL_BLEND);
    glLineWidth(1.0f);
    lineShader_->release();

    previewVBO_->release();
    previewVAO_->release();
}

// ============================================================================
// SketchRenderer public implementation
// ============================================================================

SketchRenderer::SketchRenderer() = default;

SketchRenderer::~SketchRenderer() = default;

bool SketchRenderer::initialize() {
    if (!impl_) {
        impl_ = std::make_unique<SketchRendererImpl>();
    }
    return impl_->initialize();
}

void SketchRenderer::cleanup() {
    if (impl_) {
        impl_->cleanup();
    }
}

void SketchRenderer::setSketch(Sketch* sketch) {
    sketch_ = sketch;
    geometryDirty_ = true;
    constraintsDirty_ = true;
    vboDirty_ = true;
}

void SketchRenderer::updateGeometry() {
    if (!sketch_) return;

    entityRenderData_.clear();

    // Process all entities
    for (const auto& entityPtr : sketch_->getAllEntities()) {
        if (!entityPtr) continue;

        EntityRenderData data;
        data.id = entityPtr->id();
        data.type = entityPtr->type();
        data.isConstruction = entityPtr->isConstruction();
        data.hasError = false;

        switch (entityPtr->type()) {
            case EntityType::Point: {
                auto* point = dynamic_cast<const SketchPoint*>(entityPtr.get());
                if (point) {
                    data.vertices.push_back({point->x(), point->y()});
                }
                break;
            }
            case EntityType::Line: {
                auto* line = dynamic_cast<const SketchLine*>(entityPtr.get());
                if (line) {
                    auto* startPt = sketch_->getEntityAs<SketchPoint>(line->startPointId());
                    auto* endPt = sketch_->getEntityAs<SketchPoint>(line->endPointId());
                    if (startPt && endPt) {
                        data.vertices.push_back({startPt->x(), startPt->y()});
                        data.vertices.push_back({endPt->x(), endPt->y()});
                    }
                }
                break;
            }
            case EntityType::Arc: {
                auto* arc = dynamic_cast<const SketchArc*>(entityPtr.get());
                if (arc) {
                    auto* center = sketch_->getEntityAs<SketchPoint>(arc->centerPointId());
                    if (center) {
                        Vec2d c{center->x(), center->y()};
                        data.vertices = tessellateArc(c, arc->radius(),
                                                       arc->startAngle(), arc->endAngle());
                    }
                }
                break;
            }
            case EntityType::Circle: {
                auto* circle = dynamic_cast<const SketchCircle*>(entityPtr.get());
                if (circle) {
                    auto* center = sketch_->getEntityAs<SketchPoint>(circle->centerPointId());
                    if (center) {
                        Vec2d c{center->x(), center->y()};
                        // Full circle: 0 to 2π
                        data.vertices = tessellateArc(c, circle->radius(),
                                                       0.0, 2.0 * std::numbers::pi);
                    }
                }
                break;
            }
            default:
                break;
        }

        if (!data.vertices.empty()) {
            // Calculate bounds
            data.bounds[0] = data.vertices[0];
            data.bounds[1] = data.vertices[0];
            for (const auto& v : data.vertices) {
                data.bounds[0].x = std::min(data.bounds[0].x, v.x);
                data.bounds[0].y = std::min(data.bounds[0].y, v.y);
                data.bounds[1].x = std::max(data.bounds[1].x, v.x);
                data.bounds[1].y = std::max(data.bounds[1].y, v.y);
            }
            entityRenderData_.push_back(std::move(data));
        }
    }

    geometryDirty_ = false;
    vboDirty_ = true;
}

void SketchRenderer::updateConstraints() {
    constraintRenderData_.clear();
    if (!sketch_) {
        constraintsDirty_ = false;
        return;
    }

    std::unordered_set<ConstraintID> conflicting(conflictingConstraints_.begin(),
                                                 conflictingConstraints_.end());

    for (const auto& constraintPtr : sketch_->getAllConstraints()) {
        if (!constraintPtr) {
            continue;
        }

        ConstraintRenderData data;
        data.id = constraintPtr->id();
        data.type = constraintPtr->type();
        gp_Pnt2d iconPos = constraintPtr->getIconPosition(*sketch_);
        data.position = {iconPos.X(), iconPos.Y()};
        data.isConflicting = (conflicting.find(data.id) != conflicting.end());

        if (auto* dim = dynamic_cast<const DimensionalConstraint*>(constraintPtr.get())) {
            data.value = dim->value();
        }

        constraintRenderData_.push_back(std::move(data));
    }

    constraintsDirty_ = false;
    vboDirty_ = true;
}

void SketchRenderer::render(const QMatrix4x4& viewMatrix, const QMatrix4x4& projMatrix) {
    if (!impl_->initialized_) return;

    if (geometryDirty_) {
        updateGeometry();
    }
    if (constraintsDirty_) {
        updateConstraints();
    }
    if (vboDirty_) {
        buildVBOs();
    }

    QMatrix4x4 model;
    if (sketch_) {
        model = buildSketchModelMatrix(sketch_->getPlane());
    }
    QMatrix4x4 mvp = projMatrix * viewMatrix * model;

    // Render lines first (construction, then normal)
    impl_->render(mvp, style_);

    // Render points on top
    impl_->renderPoints(mvp);

    // Render preview if active
    if (preview_.active && preview_.vertices.size() >= 2) {
        impl_->renderPreview(mvp, preview_.vertices, style_.colors.previewGeometry,
                             style_.previewLineWidth);
    }
}

void SketchRenderer::setEntitySelection(EntityID id, SelectionState state) {
    if (state == SelectionState::None) {
        entitySelections_.erase(id);
    } else {
        entitySelections_[id] = state;
    }
    vboDirty_ = true;
}

void SketchRenderer::clearSelection() {
    entitySelections_.clear();
    vboDirty_ = true;
}

void SketchRenderer::setHoverEntity(EntityID id) {
    if (hoverEntity_ != id) {
        hoverEntity_ = id;
        vboDirty_ = true;
    }
}

void SketchRenderer::setConflictingConstraints(const std::vector<ConstraintID>& ids) {
    conflictingConstraints_ = ids;
    constraintsDirty_ = true;
    vboDirty_ = true;
}

void SketchRenderer::setPreviewLine(const Vec2d& start, const Vec2d& end) {
    preview_.active = true;
    preview_.type = EntityType::Line;
    preview_.vertices = {start, end};
}

void SketchRenderer::setPreviewArc(const Vec2d& center, double radius,
                                    double startAngle, double endAngle) {
    preview_.active = true;
    preview_.type = EntityType::Arc;
    preview_.vertices = tessellateArc(center, radius, startAngle, endAngle);
}

void SketchRenderer::setPreviewCircle(const Vec2d& center, double radius) {
    // Circle is arc from 0 to 2π
    setPreviewArc(center, radius, 0.0, 2.0 * M_PI);
    preview_.type = EntityType::Circle;
}

void SketchRenderer::setPreviewRectangle(const Vec2d& corner1, const Vec2d& corner2) {
    preview_.active = true;
    preview_.type = EntityType::Line;  // Render as lines

    double minX = std::min(corner1.x, corner2.x);
    double maxX = std::max(corner1.x, corner2.x);
    double minY = std::min(corner1.y, corner2.y);
    double maxY = std::max(corner1.y, corner2.y);

    // 4 corners
    Vec2d bl{minX, minY};  // bottom-left
    Vec2d br{maxX, minY};  // bottom-right
    Vec2d tr{maxX, maxY};  // top-right
    Vec2d tl{minX, maxY};  // top-left

    // Store as line segments (pairs of vertices)
    preview_.vertices = {bl, br, br, tr, tr, tl, tl, bl};
}

void SketchRenderer::clearPreview() {
    preview_.active = false;
    preview_.vertices.clear();
}

void SketchRenderer::showSnapIndicator(const Vec2d& pos, SnapType type) {
    snapIndicator_.active = true;
    snapIndicator_.position = pos;
    snapIndicator_.type = type;
    vboDirty_ = true;
}

void SketchRenderer::hideSnapIndicator() {
    snapIndicator_.active = false;
    vboDirty_ = true;
}

void SketchRenderer::setStyle(const SketchRenderStyle& style) {
    style_ = style;
    vboDirty_ = true;
}

void SketchRenderer::setViewport(const Viewport& viewport) {
    constexpr double kViewportEpsilon = 1e-9;
    bool changed = std::abs(viewport_.center.x - viewport.center.x) > kViewportEpsilon ||
                   std::abs(viewport_.center.y - viewport.center.y) > kViewportEpsilon ||
                   std::abs(viewport_.size.x - viewport.size.x) > kViewportEpsilon ||
                   std::abs(viewport_.size.y - viewport.size.y) > kViewportEpsilon;
    viewport_ = viewport;
    if (changed) {
        vboDirty_ = true;
    }
}

void SketchRenderer::setPixelScale(double scale) {
    constexpr double kScaleEpsilon = 1e-9;
    if (std::abs(scale - pixelScale_) < kScaleEpsilon) {
        return;
    }
    pixelScale_ = scale;
    geometryDirty_ = true;
    vboDirty_ = true;
}

void SketchRenderer::setDOF(int dof) {
    currentDOF_ = dof;
    vboDirty_ = true;
}

void SketchRenderer::setShowDOF(bool show) {
    showDOF_ = show;
    vboDirty_ = true;
}

EntityID SketchRenderer::pickEntity(const Vec2d& screenPos, double tolerance) const {
    // Note: screenPos must be in world/sketch coordinates, not pixel screen coordinates.
    // Caller should transform screen pixels to sketch space before calling.
    EntityID closest;
    double minDist = tolerance;

    for (const auto& data : entityRenderData_) {
        if (!isEntityVisible(data)) continue;

        if (data.type == EntityType::Point) {
            if (!data.vertices.empty()) {
                double dx = screenPos.x - data.vertices[0].x;
                double dy = screenPos.y - data.vertices[0].y;
                double dist = std::sqrt(dx * dx + dy * dy);
                if (dist < minDist) {
                    minDist = dist;
                    closest = data.id;
                }
            }
        } else {
            // Check distance to line segments
            for (size_t i = 0; i + 1 < data.vertices.size(); ++i) {
                const auto& p1 = data.vertices[i];
                const auto& p2 = data.vertices[i + 1];

                // Distance from point to line segment
                double dx = p2.x - p1.x;
                double dy = p2.y - p1.y;
                double lenSq = dx * dx + dy * dy;
                if (lenSq < 1e-10) continue;

                double t = std::clamp(
                    ((screenPos.x - p1.x) * dx + (screenPos.y - p1.y) * dy) / lenSq,
                    0.0, 1.0);

                double projX = p1.x + t * dx;
                double projY = p1.y + t * dy;
                double dist = std::sqrt((screenPos.x - projX) * (screenPos.x - projX) +
                                         (screenPos.y - projY) * (screenPos.y - projY));

                if (dist < minDist) {
                    minDist = dist;
                    closest = data.id;
                }
            }
        }
    }

    return closest;
}

ConstraintID SketchRenderer::pickConstraint(const Vec2d& /*screenPos*/,
                                            double /*tolerance*/) const {
    // Not yet implemented
    return {};
}

std::vector<Vec2d> SketchRenderer::tessellateArc(const Vec2d& center, double radius,
                                                  double startAngle, double endAngle) const {
    std::vector<Vec2d> result;

    // Calculate sweep angle
    double sweep = endAngle - startAngle;
    if (sweep < 0) {
        sweep += 2.0 * std::numbers::pi;
    }

    double arcAngleDeg = style_.arcTessellationAngle > 0 ? style_.arcTessellationAngle : 5.0;
    double arcAngleRad = arcAngleDeg * std::numbers::pi / 180.0;
    double pixelsPerUnit = pixelScale_ > 0.0 ? (1.0 / pixelScale_) : 1.0;
    double arcLengthPixels = radius * pixelsPerUnit * std::abs(sweep);
    double segmentsByPixels = arcLengthPixels > 0.0 ? (arcLengthPixels / 5.0) : 1.0;
    double segmentsByAngle = std::abs(sweep) / arcAngleRad;
    int segments = static_cast<int>(std::ceil(std::max(segmentsByPixels, segmentsByAngle)));
    segments = std::clamp(segments, style_.minArcSegments, style_.maxArcSegments);

    double step = sweep / static_cast<double>(segments);

    for (int i = 0; i <= segments; ++i) {
        double angle = startAngle + step * static_cast<double>(i);
        result.push_back({center.x + radius * std::cos(angle),
                          center.y + radius * std::sin(angle)});
    }

    return result;
}

void SketchRenderer::buildVBOs() {
    if (!impl_->initialized_) return;
    if (geometryDirty_) {
        updateGeometry();
    }
    SketchRenderStyle renderStyle = style_;
    if (showDOF_) {
        if (currentDOF_ == 0) {
            renderStyle.colors.normalGeometry = style_.colors.fullyConstrained;
        } else if (currentDOF_ > 0) {
            renderStyle.colors.normalGeometry = style_.colors.underConstrained;
        } else {
            renderStyle.colors.normalGeometry = style_.colors.overConstrained;
        }
    }

    bool snapActive = snapIndicator_.active;
    Vec2d snapPos = snapIndicator_.position;
    float snapSize = renderStyle.snapPointSize;
    Vec3d snapColor = renderStyle.colors.constraintIcon;
    impl_->buildVBOs(entityRenderData_, renderStyle, entitySelections_, hoverEntity_,
                     viewport_, pixelScale_, constraintRenderData_,
                     snapActive, snapPos, snapSize, snapColor);
    vboDirty_ = false;
}

bool SketchRenderer::isEntityVisible(const EntityRenderData& data) const {
    return viewport_.intersects(data.bounds[0], data.bounds[1]);
}

Vec2d SketchRenderer::calculateConstraintIconPosition(
    const SketchConstraint* /*constraint*/) const {
    // Not yet implemented
    return {0, 0};
}

// ============================================================================
// SnapManager implementation (minimal)
// ============================================================================

SnapResult SnapManager::findSnap(const Vec2d& cursorPos, const Sketch& sketch,
                                  EntityID excludeEntity) const {
    auto allSnaps = findAllSnaps(cursorPos, sketch, excludeEntity);
    if (allSnaps.empty()) {
        return {};
    }

    std::sort(allSnaps.begin(), allSnaps.end());
    return allSnaps.front();
}

void SnapManager::setSnapEnabled(SnapType type, bool enabled) {
    snapEnabled_[type] = enabled;
}

void SnapManager::setGridSnap(bool enabled, double gridSize) {
    gridSnapEnabled_ = enabled;
    gridSize_ = gridSize;
}

std::vector<SnapResult> SnapManager::findAllSnaps(const Vec2d& cursorPos,
                                                   const Sketch& sketch,
                                                   EntityID excludeEntity) const {
    std::vector<SnapResult> results;
    double radiusSq = snapRadius_ * snapRadius_;

    // Helper to check if snap type is enabled (default: enabled)
    auto isSnapEnabled = [this](SnapType type) {
        auto it = snapEnabled_.find(type);
        return it == snapEnabled_.end() || it->second;
    };

    // Check points (vertices)
    if (isSnapEnabled(SnapType::Vertex)) {
        for (const auto& entityPtr : sketch.getAllEntities()) {
            if (!entityPtr || entityPtr->id() == excludeEntity) continue;
            if (entityPtr->type() != EntityType::Point) continue;

            auto* point = dynamic_cast<const SketchPoint*>(entityPtr.get());
            if (!point) continue;

            double dx = cursorPos.x - point->x();
            double dy = cursorPos.y - point->y();
            double distSq = dx * dx + dy * dy;

            if (distSq <= radiusSq) {
                SnapResult snap;
                snap.snapped = true;
                snap.type = SnapType::Vertex;
                snap.position = {point->x(), point->y()};
                snap.entityId = point->id();
                snap.distance = std::sqrt(distSq);
                results.push_back(snap);
            }
        }
    }

    // Grid snap
    if (gridSnapEnabled_ && gridSize_ > 0 && isSnapEnabled(SnapType::Grid)) {
        double gx = std::round(cursorPos.x / gridSize_) * gridSize_;
        double gy = std::round(cursorPos.y / gridSize_) * gridSize_;
        double dx = cursorPos.x - gx;
        double dy = cursorPos.y - gy;
        double distSq = dx * dx + dy * dy;

        if (distSq <= radiusSq) {
            SnapResult snap;
            snap.snapped = true;
            snap.type = SnapType::Grid;
            snap.position = {gx, gy};
            snap.distance = std::sqrt(distSq);
            results.push_back(snap);
        }
    }

    return results;
}

} // namespace onecad::core::sketch
