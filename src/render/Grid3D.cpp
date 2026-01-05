#include "Grid3D.h"
#include <QtMath>
#include <QDebug>
#include <cmath>

namespace onecad {
namespace render {

// Use GLSL 410 core for macOS compatibility (Metal backend)
static const char* vertexShaderSource = R"(
#version 410 core
layout(location = 0) in vec3 aPos;
layout(location = 1) in vec4 aColor;

uniform mat4 uMVP;

out vec4 vColor;

void main() {
    gl_Position = uMVP * vec4(aPos, 1.0);
    vColor = aColor;
}
)";

static const char* fragmentShaderSource = R"(
#version 410 core
in vec4 vColor;
out vec4 FragColor;

void main() {
    FragColor = vColor;
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
        qWarning() << "Grid3D: Vertex shader compile error:" << m_shader->log();
        return;
    }
    
    if (!m_shader->addShaderFromSourceCode(QOpenGLShader::Fragment, fragmentShaderSource)) {
        qWarning() << "Grid3D: Fragment shader compile error:" << m_shader->log();
        return;
    }
    
    if (!m_shader->link()) {
        qWarning() << "Grid3D: Shader link error:" << m_shader->log();
        return;
    }
    
    qDebug() << "Grid3D: Shaders compiled and linked successfully";
    
    // Create buffers
    if (!m_vao.create()) {
        qWarning() << "Grid3D: Failed to create VAO";
        return;
    }
    
    if (!m_vertexBuffer.create()) {
        qWarning() << "Grid3D: Failed to create vertex buffer";
        return;
    }
    
    m_vertexBuffer.setUsagePattern(QOpenGLBuffer::DynamicDraw);
    
    m_initialized = true;
    
    // Build initial grid
    buildGrid(10.0f, 50.0f, 1000.0f);
    
    qDebug() << "Grid3D: Initialized successfully with" << m_lineCount << "vertices";
}

void Grid3D::cleanup() {
    if (!m_initialized) return;
    
    m_vao.destroy();
    m_vertexBuffer.destroy();
    delete m_shader;
    m_shader = nullptr;
    
    m_initialized = false;
}

float Grid3D::calculateSpacing(float pixelScale) const {
    if (pixelScale <= 0.0f) {
        return 10.0f;
    }

    const float targetMinor = pixelScale * 10.0f;
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

void Grid3D::buildGrid(float minorSpacing, float majorSpacing, float extent) {
    m_vertices.clear();
    m_colors.clear();
    m_lastExtent = extent;

    if (minorSpacing <= 0.0f) {
        return;
    }

    int lineCount = static_cast<int>(extent / minorSpacing);
    // Limit line count to avoid excessive geometry
    lineCount = qMin(lineCount, 200);

    int majorStep = 5;
    if (majorSpacing > 0.0f) {
        majorStep = qMax(1, static_cast<int>(std::round(majorSpacing / minorSpacing)));
    }
    
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
    
    // Grid lines parallel to X axis
    for (int i = -lineCount; i <= lineCount; ++i) {
        float y = i * minorSpacing;
        bool isMajor = (i % majorStep == 0);
        float lineExtent = lineCount * minorSpacing;

        if (i == 0) {
            // Coordinate Mapping: Geom X- aligns with User Y+
            // Draw User Y Axis (Green) along negative Geom X
            addLine(0, 0, 0, lineExtent, 0, 0, m_majorColor); // Geom X+ (Gray)
            addLine(0, 0, 0, -lineExtent, 0, 0, m_yAxisColor); // Geom X- (Green)
        } else {
            QColor color = isMajor ? m_majorColor : m_minorColor;
            addLine(-lineExtent, y, 0, lineExtent, y, 0, color);
        }
    }
    
    // Grid lines parallel to Y axis
    for (int i = -lineCount; i <= lineCount; ++i) {
        float x = i * minorSpacing;
        bool isMajor = (i % majorStep == 0);
        float lineExtent = lineCount * minorSpacing;

        if (i == 0) {
            // Coordinate Mapping: Geom Y+ aligns with User X+
            // Draw User X Axis (Red) along positive Geom Y
            addLine(0, -lineExtent, 0, 0, 0, 0, m_majorColor); // Geom Y- (Gray)
            addLine(0, 0, 0, 0, lineExtent, 0, m_xAxisColor); // Geom Y+ (Red)
        } else {
            QColor color = isMajor ? m_majorColor : m_minorColor;
            addLine(x, -lineExtent, 0, x, lineExtent, 0, color);
        }
    }
    
    // Z axis (blue) - vertical line at origin
    float zExtent = lineCount * minorSpacing * 0.5f;
    addLine(0, 0, 0, 0, 0, zExtent, m_zAxisColor);
    
    m_lineCount = static_cast<int>(m_vertices.size() / 3);
    m_lastSpacing = minorSpacing;
    
    if (m_vertices.empty()) {
        qWarning() << "Grid3D: No vertices generated!";
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

void Grid3D::render(const QMatrix4x4& viewProjection, float pixelScale, float viewExtent) {
    if (!m_visible || !m_initialized || m_lineCount == 0) return;

    const float minorSpacing = calculateSpacing(pixelScale);
    const float majorSpacing = minorSpacing * 5.0f;
    const float extent = qMax(viewExtent, minorSpacing * 20.0f);

    if (m_lastSpacing < 0.0f ||
        std::abs(minorSpacing - m_lastSpacing) > 1e-4f ||
        std::abs(extent - m_lastExtent) > 1e-3f) {
        buildGrid(minorSpacing, majorSpacing, extent);
    }
    
    m_shader->bind();
    m_shader->setUniformValue("uMVP", viewProjection);
    
    m_vao.bind();
    
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    
    glDrawArrays(GL_LINES, 0, m_lineCount);
    
    glDisable(GL_BLEND);
    
    m_vao.release();
    m_shader->release();
}

} // namespace render
} // namespace onecad
