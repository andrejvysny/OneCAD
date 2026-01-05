#ifndef ONECAD_RENDER_GRID3D_H
#define ONECAD_RENDER_GRID3D_H

#include <QVector3D>
#include <QMatrix4x4>
#include <QColor>
#include <QOpenGLFunctions>
#include <QOpenGLShaderProgram>
#include <QOpenGLBuffer>
#include <QOpenGLVertexArrayObject>
#include <vector>

namespace onecad {
namespace render {

/**
 * @brief Adaptive 3D grid for CAD viewport.
 * 
 * Renders XY plane grid with:
 * - Auto-spacing based on camera distance (per spec section 7.2)
 * - Major/minor line distinction
 * - Origin axes in RGB colors
 */
class Grid3D : protected QOpenGLFunctions {
public:
    Grid3D();
    ~Grid3D();

    void initialize();
    void render(const QMatrix4x4& viewProjection, float pixelScale, float viewExtent);
    void cleanup();
    
    // Appearance
    void setMajorColor(const QColor& color) { m_majorColor = color; }
    void setMinorColor(const QColor& color) { m_minorColor = color; }
    void setVisible(bool visible) { m_visible = visible; }
    bool isVisible() const { return m_visible; }
    
    // Force grid rebuild (e.g. after color change)
    void forceUpdate() { m_lastSpacing = -1.0f; m_lastExtent = -1.0f; }

private:
    float calculateSpacing(float pixelScale) const;
    void buildGrid(float minorSpacing, float majorSpacing, float extent);
    
    bool m_initialized = false;
    bool m_visible = true;
    
    QOpenGLShaderProgram* m_shader = nullptr;
    QOpenGLBuffer m_vertexBuffer;
    QOpenGLVertexArrayObject m_vao;
    
    QColor m_majorColor;
    QColor m_minorColor;

    QColor m_xAxisColor{255, 100, 100, 255}; // Red
    QColor m_yAxisColor{100, 255, 100, 255}; // Green
    QColor m_zAxisColor{100, 100, 255, 255}; // Blue
    
    std::vector<float> m_vertices;
    std::vector<float> m_colors;
    int m_lineCount = 0;
    
    float m_lastSpacing = 0.0f;
    float m_lastExtent = 0.0f;
};

} // namespace render
} // namespace onecad

#endif // ONECAD_RENDER_GRID3D_H
