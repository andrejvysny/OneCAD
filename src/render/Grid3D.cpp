#include "Grid3D.h"
#include <QtMath>
#include <iostream>

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
        std::cerr << "Grid3D: Vertex shader compile error: " 
                  << m_shader->log().toStdString() << std::endl;
        return;
    }
    
    if (!m_shader->addShaderFromSourceCode(QOpenGLShader::Fragment, fragmentShaderSource)) {
        std::cerr << "Grid3D: Fragment shader compile error: " 
                  << m_shader->log().toStdString() << std::endl;
        return;
    }
    
    if (!m_shader->link()) {
        std::cerr << "Grid3D: Shader link error: " 
                  << m_shader->log().toStdString() << std::endl;
        return;
    }
    
    std::cout << "Grid3D: Shaders compiled and linked successfully" << std::endl;
    
    // Create buffers
    if (!m_vao.create()) {
        std::cerr << "Grid3D: Failed to create VAO" << std::endl;
        return;
    }
    
    if (!m_vertexBuffer.create()) {
        std::cerr << "Grid3D: Failed to create vertex buffer" << std::endl;
        return;
    }
    
    m_vertexBuffer.setUsagePattern(QOpenGLBuffer::DynamicDraw);
    
    m_initialized = true;
    
    // Build initial grid
    buildGrid(10.0f, 1000.0f);
    
    std::cout << "Grid3D: Initialized successfully with " << m_lineCount << " vertices" << std::endl;
}

void Grid3D::cleanup() {
    if (!m_initialized) return;
    
    m_vao.destroy();
    m_vertexBuffer.destroy();
    delete m_shader;
    m_shader = nullptr;
    
    m_initialized = false;
}

float Grid3D::calculateSpacing(float cameraDistance) const {
    // Per specification section 7.2 - adaptive spacing tiers
    // Target ~30-50 grid lines visible
    
    if (cameraDistance < 20.0f) return 0.1f;
    if (cameraDistance < 100.0f) return 0.5f;
    if (cameraDistance < 200.0f) return 1.0f;
    if (cameraDistance < 1000.0f) return 5.0f;
    if (cameraDistance < 2000.0f) return 10.0f;
    if (cameraDistance < 10000.0f) return 50.0f;
    return 100.0f;
}

void Grid3D::buildGrid(float spacing, float extent) {
    m_vertices.clear();
    m_colors.clear();
    
    int lineCount = static_cast<int>(extent / spacing);
    // Limit line count to avoid excessive geometry
    lineCount = qMin(lineCount, 200);
    
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
        float y = i * spacing;
        bool isMajor = (i % 10 == 0);
        QColor color = (i == 0) ? QColor(100, 255, 100, 255) : // Y axis green
                       (isMajor ? m_majorColor : m_minorColor);
        float lineExtent = lineCount * spacing;
        addLine(-lineExtent, y, 0, lineExtent, y, 0, color);
    }
    
    // Grid lines parallel to Y axis
    for (int i = -lineCount; i <= lineCount; ++i) {
        float x = i * spacing;
        bool isMajor = (i % 10 == 0);
        QColor color = (i == 0) ? QColor(255, 100, 100, 255) : // X axis red
                       (isMajor ? m_majorColor : m_minorColor);
        float lineExtent = lineCount * spacing;
        addLine(x, -lineExtent, 0, x, lineExtent, 0, color);
    }
    
    // Z axis (blue) - vertical line at origin
    float zExtent = lineCount * spacing * 0.5f;
    addLine(0, 0, 0, 0, 0, zExtent, QColor(100, 100, 255, 255));
    
    m_lineCount = static_cast<int>(m_vertices.size() / 3);
    m_lastSpacing = spacing;
    
    if (m_vertices.empty()) {
        std::cerr << "Grid3D: No vertices generated!" << std::endl;
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

void Grid3D::render(const QMatrix4x4& viewProjection, float cameraDistance) {
    if (!m_visible || !m_initialized || m_lineCount == 0) return;
    
    // Rebuild grid if spacing changed
    float newSpacing = calculateSpacing(cameraDistance);
    if (qAbs(newSpacing - m_lastSpacing) > 0.001f) {
        float extent = cameraDistance * 3.0f;
        buildGrid(newSpacing, extent);
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
