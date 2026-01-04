#ifndef ONECAD_UI_VIEWPORT_H
#define ONECAD_UI_VIEWPORT_H

#include <QOpenGLWidget>
#include <QOpenGLFunctions>
#include <QPoint>
#include <QElapsedTimer>
#include <QMatrix4x4>
#include <QVector3D>
#include <QVariantAnimation>
#include <memory>

namespace onecad {
namespace app {
    class Document;
}
namespace render {
    class Camera3D;
    class Grid3D;
}
namespace core::sketch {
    class Sketch;
    class SketchRenderer;
    struct Vec2d;
    namespace tools {
        class SketchToolManager;
        enum class ToolType;
    }
}
}

namespace onecad {
namespace ui {
    class ViewCube; // Forward declaration

    struct CameraState {
        QVector3D position;
        QVector3D target;
        QVector3D up;
        float angle = 0.0f;
    };

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

    // Plane selection
    bool isPlaneSelectionActive() const { return m_planeSelectionActive; }

    // Sketch mode
    bool isInSketchMode() const { return m_inSketchMode; }
    core::sketch::Sketch* activeSketch() const { return m_activeSketch; }

    // Tool management
    core::sketch::tools::SketchToolManager* toolManager() const;
    core::sketch::Vec2d screenToSketch(const QPoint& screenPos) const;

    // Document access (for rendering all sketches in 3D mode)
    void setDocument(app::Document* document);

signals:
    void mousePositionChanged(double x, double y, double z);
    void cameraChanged();
    void sketchModeChanged(bool inSketchMode);
    void sketchPlanePicked(int planeIndex);
    void planeSelectionCancelled();
    void sketchUpdated();  // Emitted when geometry/constraints change

public slots:
    void beginPlaneSelection();
    void cancelPlaneSelection();

    // Sketch mode
    void enterSketchMode(core::sketch::Sketch* sketch);
    void exitSketchMode();

    // Tool activation
    void activateLineTool();
    void activateCircleTool();
    void activateRectangleTool();
    void deactivateTool();

    // Views
    void setFrontView();
    void setBackView();
    void setLeftView();
    void setRightView();
    void setTopView();
    void setBottomView();
    void setIsometricView();
    void resetView();
    void setCameraAngle(float degrees);
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
    void keyPressEvent(QKeyEvent* event) override;

    // Touch/gesture events (for trackpad)
    bool event(QEvent* event) override;

private:
    void updateSketchRenderingState();
    void handlePan(float dx, float dy);
    void handleOrbit(float dx, float dy);
    void handleZoom(float delta);
    bool isNativeZoomActive() const;
    void updatePlaneSelectionHover(const QPoint& screenPos);
    bool pickPlaneSelection(const QPoint& screenPos, int* outIndex) const;
    void drawPlaneSelectionOverlay(const QMatrix4x4& viewProjection);
    
    // Animation
    void animateCamera(const CameraState& targetState);

    std::unique_ptr<render::Camera3D> m_camera;
    std::unique_ptr<render::Grid3D> m_grid;
    std::unique_ptr<core::sketch::SketchRenderer> m_sketchRenderer;
    std::unique_ptr<core::sketch::tools::SketchToolManager> m_toolManager;
    ViewCube* m_viewCube = nullptr;
    QVariantAnimation* m_cameraAnimation = nullptr;

    // Sketch mode
    core::sketch::Sketch* m_activeSketch = nullptr;
    bool m_inSketchMode = false;
    bool m_planeSelectionActive = false;
    int m_planeHoverIndex = -1;

    // Document for rendering all sketches
    app::Document* m_document = nullptr;
    bool m_documentSketchesDirty = true;  // Track when geometry needs rebuild
    float m_savedCameraAngle = 45.0f;  // Store camera angle before sketch mode
    QVector3D m_savedCameraPosition;
    QVector3D m_savedCameraTarget;
    QVector3D m_savedCameraUp;
    
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
