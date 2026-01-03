#ifndef ONECAD_UI_VIEWPORT_H
#define ONECAD_UI_VIEWPORT_H

#include <QOpenGLWidget>
#include <QOpenGLFunctions>
#include <QPoint>
#include <QElapsedTimer>
#include <memory>

namespace onecad {
namespace render {
    class Camera3D;
    class Grid3D;
}
}

namespace onecad {
namespace ui {
    class ViewCube; // Forward declaration

/**
 * @brief OpenGL 3D viewport with Shapr3D-style navigation.
 *
 * Navigation controls:
 * - Mouse: Right-drag=orbit, Middle-drag=pan, Scroll=zoom
 * - Trackpad: Two-finger=pan, Shift+two-finger=orbit, Pinch=zoom
 */
class Viewport : public QOpenGLWidget, protected QOpenGLFunctions {
    Q_OBJECT

public:
    explicit Viewport(QWidget* parent = nullptr);
    ~Viewport() override;

    render::Camera3D* camera() const { return m_camera.get(); }

signals:
    void mousePositionChanged(double x, double y, double z);
    void cameraChanged();

public slots:
    void setFrontView();
    void setBackView();
    void setLeftView();
    void setRightView();
    void setTopView();
    void setBottomView();
    void setIsometricView();
    void resetView();
    void toggleGrid();
    void updateTheme();

protected:
    // OpenGL
    void initializeGL() override;
    void resizeGL(int w, int h) override;
    void paintGL() override;
    
    // Mouse events
    void mousePressEvent(QMouseEvent* event) override;
    void mouseMoveEvent(QMouseEvent* event) override;
    void mouseReleaseEvent(QMouseEvent* event) override;
    void wheelEvent(QWheelEvent* event) override;
    void resizeEvent(QResizeEvent* event) override;
    
    // Touch/gesture events (for trackpad)
    bool event(QEvent* event) override;

private:
    void handlePan(float dx, float dy);
    void handleOrbit(float dx, float dy);
    void handleZoom(float delta);
    bool isNativeZoomActive() const;

    std::unique_ptr<render::Camera3D> m_camera;
    std::unique_ptr<render::Grid3D> m_grid;
    ViewCube* m_viewCube = nullptr;
    
    // Appearance
    QColor m_backgroundColor;
    
    // Mouse state
    QPoint m_lastMousePos;
    bool m_isPanning = false;
    bool m_isOrbiting = false;
    
    // Gesture state
    qreal m_lastPinchScale = 1.0;
    bool m_pinchActive = false;
    QElapsedTimer m_nativeZoomTimer;
    qint64 m_lastNativeZoomMs = -1;
    
    // Viewport size
    int m_width = 1;
    int m_height = 1;

    // Signal connection management
    QMetaObject::Connection m_themeConnection;
};

} // namespace ui
} // namespace onecad

// Backward compatibility
using MainWindowViewport = onecad::ui::Viewport;

#endif // ONECAD_UI_VIEWPORT_H
