#ifndef ONECAD_UI_VIEWCUBE_H
#define ONECAD_UI_VIEWCUBE_H

#include <QWidget>
#include <QMatrix4x4>
#include <QVector3D>
#include <QVector2D>
#include <QVector>

namespace onecad {
namespace render {
    class Camera3D;
}
}

namespace onecad {
namespace ui {

class ViewCube : public QWidget {
    Q_OBJECT

public:
    explicit ViewCube(QWidget* parent = nullptr);

    void setCamera(render::Camera3D* camera);

public slots:
    void updateRotation();
    void updateTheme();

signals:
    void viewChanged();

protected:
    void paintEvent(QPaintEvent* event) override;
    void mousePressEvent(QMouseEvent* event) override;
    void mouseMoveEvent(QMouseEvent* event) override;
    void mouseReleaseEvent(QMouseEvent* event) override;
    void enterEvent(QEnterEvent* event) override;
    void leaveEvent(QEvent* event) override;

private:
    enum class ElementType { None, Face, Edge, Corner };
    struct Hit {
        ElementType type = ElementType::None;
        int index = -1;
        bool operator==(const Hit& other) const { return type == other.type && index == other.index; }
        bool operator!=(const Hit& other) const { return !(*this == other); }
    };

    struct CubeVertex {
        QVector3D pos; // World space (+/- 1)
        int id;
    };
    
    struct CubeEdge {
        int v1, v2;
        int id;
    };
    
    struct CubeFace {
        int vIndices[4]; // Indices into m_vertices
        QVector3D normal;
        QString text;
        int id;
    };

    void initGeometry();
    Hit hitTest(const QPoint& pos);
    QPointF project(const QVector3D& point, const QMatrix4x4& viewRot, float scale);
    void snapToView(const Hit& hit);

    render::Camera3D* m_camera = nullptr;
    
    QVector<CubeVertex> m_vertices;
    QVector<CubeEdge> m_edges;
    QVector<CubeFace> m_faces;
    
    QMatrix4x4 m_cubeRotation;
    float m_cubeSize = 120.0f;
    float m_scale = 0.6f;

    Hit m_hoveredHit;
    bool m_isDragging = false;
    QPoint m_lastMousePos;

    // Theme Colors
    QColor m_faceColor;
    QColor m_faceHoverColor;
    QColor m_textColor;
    QColor m_textHoverColor;
    QColor m_edgeHoverColor;
    QColor m_cornerHoverColor;

    // Signal connection management
    QMetaObject::Connection m_themeConnection;
};

} // namespace ui
} // namespace onecad

#endif // ONECAD_UI_VIEWCUBE_H
