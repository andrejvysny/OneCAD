#include "Grid3D.h"
#include <QtMath>
#include <QLoggingCategory>
#include <algorithm>
#include <cmath>

namespace onecad {
namespace render {

Q_LOGGING_CATEGORY(logGrid3D, "onecad.render.grid")

// Use GLSL 410 core for macOS compatibility (Metal backend)
static const char* vertexShaderSource = R"(
#version 410 core
layout(location = 0) in vec3 aPos;
layout(location = 1) in vec4 aColor;

uniform mat4 uMVP;

out vec4 vColor;
out vec2 vPlanePos;

void main() {
    gl_Position = uMVP * vec4(aPos, 1.0);
    vColor = aColor;
    vPlanePos = aPos.xy;
}
)";

static const char* fragmentShaderSource = R"(
#version 410 core
in vec4 vColor;
in vec2 vPlanePos;
out vec4 FragColor;

uniform vec2 uFadeOriginPlane;
uniform float uFadeStart;
uniform float uFadeEnd;

void main() {
    float dist = length(vPlanePos - uFadeOriginPlane);
    float fade = 1.0 - smoothstep(uFadeStart, uFadeEnd, dist);
    FragColor = vec4(vColor.rgb, vColor.a * fade);
}
)";

Grid3D::Grid3D() 
    : m_vertexBuffer(QOpenGLBuffer::VertexBuffer)
    , m_majorColor(80, 80, 80)
    , m_minorColor(50, 50, 50) {
}

Grid3D::~Grid3D() {
    cleanup();
}

void Grid3D::initialize() {
    if (m_initialized) return;
    
    initializeOpenGLFunctions();
    
    // Create shader with error checking
    m_shader = new QOpenGLShaderProgram();
    
    if (!m_shader->addShaderFromSourceCode(QOpenGLShader::Vertex, vertexShaderSource)) {
        qCWarning(logGrid3D) << "initialize:vertex-shader-compile-error" << m_shader->log();
        return;
    }
    
    if (!m_shader->addShaderFromSourceCode(QOpenGLShader::Fragment, fragmentShaderSource)) {
        qCWarning(logGrid3D) << "initialize:fragment-shader-compile-error" << m_shader->log();
        return;
    }
    
    if (!m_shader->link()) {
        qCWarning(logGrid3D) << "initialize:shader-link-error" << m_shader->log();
        return;
    }
    qCDebug(logGrid3D) << "initialize:shader-ready";
    
    // Create buffers
    if (!m_vao.create()) {
        qCWarning(logGrid3D) << "initialize:failed-create-vao";
        return;
    }
    
    if (!m_vertexBuffer.create()) {
        qCWarning(logGrid3D) << "initialize:failed-create-vertex-buffer";
        return;
    }
    
    m_vertexBuffer.setUsagePattern(QOpenGLBuffer::DynamicDraw);
    
    m_initialized = true;
    
    // Build initial grid
    buildGrid(10.0f, 50.0f, -500.0f, 500.0f, -500.0f, 500.0f);
    
    qCInfo(logGrid3D) << "initialize:done" << "lineVertices=" << m_lineCount;
}

void Grid3D::cleanup() {
    if (!m_initialized) return;
    
    m_vao.destroy();
    m_vertexBuffer.destroy();
    delete m_shader;
    m_shader = nullptr;
    
    m_initialized = false;
}

float Grid3D::adaptiveSpacing(float pixelScale) {
    if (pixelScale <= 0.0f) {
        return 10.0f;
    }

    const float targetMinor = pixelScale * 50.0f;
    const float exponent = std::floor(std::log10(targetMinor));
    const float base = std::pow(10.0f, exponent);
    const float normalized = targetMinor / base;

    float snapped = 1.0f;
    if (normalized < 1.5f) {
        snapped = 1.0f;
    } else if (normalized < 3.5f) {
        snapped = 2.0f;
    } else if (normalized < 7.5f) {
        snapped = 5.0f;
    } else {
        snapped = 10.0f;
    }

    return snapped * base;
}

void Grid3D::buildGrid(float minorSpacing,
                       float majorSpacing,
                       float startX,
                       float endX,
                       float startY,
                       float endY) {
    m_vertices.clear();
    m_colors.clear();

    if (minorSpacing <= 0.0f) {
        return;
    }

    int majorStep = 5;
    if (majorSpacing > 0.0f) {
        majorStep = qMax(1, static_cast<int>(std::round(majorSpacing / minorSpacing)));
    }

    if (startX > endX) {
        std::swap(startX, endX);
    }
    if (startY > endY) {
        std::swap(startY, endY);
    }

    int lineCountX = static_cast<int>(std::round((endX - startX) / minorSpacing));
    int lineCountY = static_cast<int>(std::round((endY - startY) / minorSpacing));

    float lineExtentX = endX - startX;
    float lineExtentY = endY - startY;
    
    auto addLine = [this](float x1, float y1, float z1, 
                          float x2, float y2, float z2,
                          const QColor& color) {
        // Vertex 1
        m_vertices.push_back(x1);
        m_vertices.push_back(y1);
        m_vertices.push_back(z1);
        m_colors.push_back(color.redF());
        m_colors.push_back(color.greenF());
        m_colors.push_back(color.blueF());
        m_colors.push_back(color.alphaF());
        
        // Vertex 2
        m_vertices.push_back(x2);
        m_vertices.push_back(y2);
        m_vertices.push_back(z2);
        m_colors.push_back(color.redF());
        m_colors.push_back(color.greenF());
        m_colors.push_back(color.blueF());
        m_colors.push_back(color.alphaF());
    };
    
    const bool originInX = (startX <= 0.0f && endX >= 0.0f);
    const bool originInY = (startY <= 0.0f && endY >= 0.0f);

    // Grid lines parallel to X axis (varying Y)
    for (int i = 0; i <= lineCountY; ++i) {
        float y = startY + static_cast<float>(i) * minorSpacing;
        int stepIndex = static_cast<int>(std::round(y / minorSpacing));
        bool isMajor = (stepIndex % majorStep == 0);

        if (qFuzzyIsNull(y) && originInY) {
            // Local +X axis is red, local -X remains major grid color.
            if (startX < 0.0f) {
                addLine(0.0f, 0.0f, 0.0f, startX, 0.0f, 0.0f, m_majorColor);
            }
            if (endX > 0.0f) {
                addLine(0.0f, 0.0f, 0.0f, endX, 0.0f, 0.0f, m_xAxisColor);
            }
        } else {
            QColor color = isMajor ? m_majorColor : m_minorColor;
            addLine(startX, y, 0.0f, endX, y, 0.0f, color);
        }
    }
    
    // Grid lines parallel to Y axis (varying X)
    for (int i = 0; i <= lineCountX; ++i) {
        float x = startX + static_cast<float>(i) * minorSpacing;
        int stepIndex = static_cast<int>(std::round(x / minorSpacing));
        bool isMajor = (stepIndex % majorStep == 0);

        if (qFuzzyIsNull(x) && originInX) {
            // Local +Y axis is green, local -Y remains major grid color.
            if (startY < 0.0f) {
                addLine(0.0f, startY, 0.0f, 0.0f, 0.0f, 0.0f, m_majorColor);
            }
            if (endY > 0.0f) {
                addLine(0.0f, 0.0f, 0.0f, 0.0f, endY, 0.0f, m_yAxisColor);
            }
        } else {
            QColor color = isMajor ? m_majorColor : m_minorColor;
            addLine(x, startY, 0.0f, x, endY, 0.0f, color);
        }
    }
    
    // Z axis (blue) - vertical line at origin
    float zExtent = 0.5f * qMax(lineExtentX, lineExtentY);
    if (originInX && originInY) {
        addLine(0.0f, 0.0f, 0.0f, 0.0f, 0.0f, zExtent, m_zAxisColor);
    }
    
    m_lineCount = static_cast<int>(m_vertices.size() / 3);
    m_lastSpacing = minorSpacing;
    m_lastStart = QVector2D(startX, startY);
    m_lastEnd = QVector2D(endX, endY);
    
    if (m_vertices.empty()) {
        qCWarning(logGrid3D) << "buildGrid:no-vertices";
        return;
    }
    
    // Upload to GPU
    m_vao.bind();
    
    // Interleave position and color data
    std::vector<float> interleavedData;
    interleavedData.reserve(m_vertices.size() + m_colors.size());
    
    for (size_t i = 0; i < m_vertices.size() / 3; ++i) {
        // Position (3 floats)
        interleavedData.push_back(m_vertices[i * 3]);
        interleavedData.push_back(m_vertices[i * 3 + 1]);
        interleavedData.push_back(m_vertices[i * 3 + 2]);
        // Color (4 floats)
        interleavedData.push_back(m_colors[i * 4]);
        interleavedData.push_back(m_colors[i * 4 + 1]);
        interleavedData.push_back(m_colors[i * 4 + 2]);
        interleavedData.push_back(m_colors[i * 4 + 3]);
    }
    
    m_vertexBuffer.bind();
    m_vertexBuffer.allocate(interleavedData.data(), 
                            static_cast<int>(interleavedData.size() * sizeof(float)));
    
    // Position attribute
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 7 * sizeof(float), nullptr);
    
    // Color attribute
    glEnableVertexAttribArray(1);
    glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 7 * sizeof(float), 
                          reinterpret_cast<void*>(3 * sizeof(float)));
    
    m_vertexBuffer.release();
    m_vao.release();
}

void Grid3D::render(const QMatrix4x4& mvp,
                    float pixelScale,
                    const QVector2D& planeMin,
                    const QVector2D& planeMax,
                    const QVector2D& fadeOriginPlane) {
    if (!m_visible || !m_initialized || m_lineCount == 0) return;

    const float minorSpacing = adaptiveSpacing(pixelScale);
    const float majorSpacing = minorSpacing * 5.0f;

    float minX = planeMin.x();
    float maxX = planeMax.x();
    float minY = planeMin.y();
    float maxY = planeMax.y();
    if (minX > maxX) {
        std::swap(minX, maxX);
    }
    if (minY > maxY) {
        std::swap(minY, maxY);
    }

    QVector2D center{(minX + maxX) * 0.5f, (minY + maxY) * 0.5f};
    QVector2D half{(maxX - minX) * 0.5f, (maxY - minY) * 0.5f};
    float viewHalf = qMax(half.x(), half.y());
    viewHalf = qMax(viewHalf, minorSpacing * 2.0f);

    float gridHalf = qMax(viewHalf * 3.5f, minorSpacing * 20.0f);
    float gridMinX = center.x() - gridHalf;
    float gridMaxX = center.x() + gridHalf;
    float gridMinY = center.y() - gridHalf;
    float gridMaxY = center.y() + gridHalf;

    float startX = std::floor(gridMinX / minorSpacing) * minorSpacing;
    float endX = std::ceil(gridMaxX / minorSpacing) * minorSpacing;
    float startY = std::floor(gridMinY / minorSpacing) * minorSpacing;
    float endY = std::ceil(gridMaxY / minorSpacing) * minorSpacing;

    constexpr int kMaxLinesPerAxis = 600;
    auto clampSpan = [&](float& start, float& end) {
        float span = end - start;
        float maxSpan = minorSpacing * static_cast<float>(kMaxLinesPerAxis);
        if (span > maxSpan) {
            float mid = (start + end) * 0.5f;
            start = mid - maxSpan * 0.5f;
            end = mid + maxSpan * 0.5f;
        }
    };
    clampSpan(startX, endX);
    clampSpan(startY, endY);

    if (m_lastSpacing < 0.0f ||
        std::abs(minorSpacing - m_lastSpacing) > 1e-4f ||
        std::abs(startX - m_lastStart.x()) > 1e-3f ||
        std::abs(startY - m_lastStart.y()) > 1e-3f ||
        std::abs(endX - m_lastEnd.x()) > 1e-3f ||
        std::abs(endY - m_lastEnd.y()) > 1e-3f) {
        buildGrid(minorSpacing, majorSpacing, startX, endX, startY, endY);
    }
    
    m_shader->bind();
    m_shader->setUniformValue("uMVP", mvp);
    m_shader->setUniformValue("uFadeOriginPlane", fadeOriginPlane);

    float gridHalfSpan = 0.5f * qMax(endX - startX, endY - startY);
    float fadeStart = qMax(viewHalf * 0.6f, minorSpacing * 12.0f);
    float fadeEnd = qMax(gridHalfSpan * 3.0f, fadeStart + minorSpacing * 120.0f);
    m_shader->setUniformValue("uFadeStart", fadeStart);
    m_shader->setUniformValue("uFadeEnd", fadeEnd);
    
    m_vao.bind();

    // Disable depth writes so grid never occludes bodies
    glDepthMask(GL_FALSE);
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

    glDrawArrays(GL_LINES, 0, m_lineCount);

    glDisable(GL_BLEND);
    glDepthMask(GL_TRUE);  // Restore depth writes

    m_vao.release();
    m_shader->release();
}

} // namespace render
} // namespace onecad
