#ifndef ONECAD_RENDER_GRID3D_H
#define ONECAD_RENDER_GRID3D_H

#include <QVector2D>
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
 * Renders a plane-local grid with:
 * - Auto-spacing based on camera distance (per spec section 7.2)
 * - Major/minor line distinction
 * - Origin axes in RGB colors
 */
class Grid3D : protected QOpenGLFunctions {
public:
    Grid3D();
    ~Grid3D();

    void initialize();
    void render(const QMatrix4x4& mvp,
                float pixelScale,
                const QVector2D& planeMin,
                const QVector2D& planeMax,
                const QVector2D& fadeOriginPlane);
    void cleanup();

    // Shared adaptive spacing used by both rendered grid and snap grid.
    static float adaptiveSpacing(float pixelScale);
    
    // Appearance
    void setMajorColor(const QColor& color) { m_majorColor = color; }
    void setMinorColor(const QColor& color) { m_minorColor = color; }
    void setAxisColors(const QColor& xColor, const QColor& yColor, const QColor& zColor) {
        m_xAxisColor = xColor;
        m_yAxisColor = yColor;
        m_zAxisColor = zColor;
    }
    void setVisible(bool visible) { m_visible = visible; }
    bool isVisible() const { return m_visible; }
    
    // Force grid rebuild (e.g. after color change)
    void forceUpdate() { m_lastSpacing = -1.0f; }

private:
    void buildGrid(float minorSpacing,
                   float majorSpacing,
                   float startX,
                   float endX,
                   float startY,
                   float endY);
    
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
    QVector2D m_lastStart{0.0f, 0.0f};
    QVector2D m_lastEnd{0.0f, 0.0f};
};

} // namespace render
} // namespace onecad

#endif // ONECAD_RENDER_GRID3D_H
