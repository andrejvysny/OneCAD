#ifndef ONECAD_UI_VIEWPORT_H
#define ONECAD_UI_VIEWPORT_H

#include <QOpenGLWidget>
#include <QOpenGLFunctions>
#include <QPoint>
#include <QElapsedTimer>
#include <QTimer>
#include <QMatrix4x4>
#include <QVector3D>
#include <QVariantAnimation>
#include <QStringList>
#include <QSize>
#include <QImage>
#include "SnapSettingsPanel.h"
#include "selection/ModelPickerAdapter.h"
#include "../../core/sketch/SketchTypes.h"
#include "../../render/scene/SceneMeshStore.h"
#include "../../app/selection/SelectionTypes.h"
#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace onecad {
namespace app {
    class Document;
    namespace commands {
        class CommandProcessor;
    }
    namespace selection {
        class SelectionManager;
        struct SelectionItem;
        struct ClickModifiers;
        struct PickResult;
    }
}
namespace render {
    class Camera3D;
    class Grid3D;
    class BodyRenderer;
}
namespace core::sketch {
    class Sketch;
    class SketchRenderer;
    struct SketchPlane;
    namespace tools {
        class SketchToolManager;
        enum class ToolType;
    }
}
}

namespace onecad {
namespace ui {
    class ViewCube; // Forward declaration
    class DimensionEditor; // Forward declaration
    namespace selection {
        class DeepSelectPopup;
        class SketchPickerAdapter;
    }
    namespace tools {
        class ModelingToolManager;
    }

    struct CameraState {
        QVector3D position;
        QVector3D target;
        QVector3D up;
        float angle = 0.0f;
        float orthoScale = 1.0f;  // Preserved visual scale for zoom-free transitions
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
    double pixelScale() const { return m_pixelScale; }

    // Plane selection
    bool isPlaneSelectionActive() const { return m_planeSelectionActive; }

    // Sketch mode
    bool isInSketchMode() const { return m_inSketchMode; }
    core::sketch::Sketch* activeSketch() const { return m_activeSketch; }

    // Tool management
    core::sketch::tools::SketchToolManager* toolManager() const;
    core::sketch::SketchRenderer* sketchRenderer() const;
    core::sketch::Vec2d screenToSketch(const QPoint& screenPos) const;
    bool isMouseOverIndicator(const QPoint& screenPos) const;

    // Sketch update notification
    void notifySketchUpdated();

    // Debug state accessors
    bool debugNormalsEnabled() const { return m_debugNormals; }
    bool debugDepthEnabled() const { return m_debugDepth; }
    bool wireframeOnlyEnabled() const { return m_wireframeOnly; }
    bool gammaDisabled() const { return m_disableGamma; }
    bool matcapEnabled() const { return m_useMatcap; }

    // Thumbnail capture
    QImage captureThumbnail(int maxSize = 512);

    // Document access (for rendering all sketches in 3D mode)
    void setDocument(app::Document* document);
    void setCommandProcessor(app::commands::CommandProcessor* processor);
    void setModelPickMeshes(std::vector<selection::ModelPickerAdapter::Mesh>&& meshes);
    void setModelPreviewMeshes(const std::vector<render::SceneMeshStore::Mesh>& meshes);
    void clearModelPreviewMeshes();
    void setPreviewHiddenBody(const std::string& bodyId);
    void clearPreviewHiddenBody();

signals:
    void mousePositionChanged(double x, double y, double z);
    void cameraChanged();
    void sketchModeChanged(bool inSketchMode);
    void sketchPlanePicked(int planeIndex);
    void planeSelectionCancelled();
    void sketchUpdated();  // Emitted when geometry/constraints change
    void openSketchForEditRequested(const QString& sketchId);
    void extrudeToolActiveChanged(bool active);
    void revolveToolActiveChanged(bool active);
    void filletToolActiveChanged(bool active);
    void shellToolActiveChanged(bool active);
    void debugTogglesChanged(bool normals, bool depth, bool wireframe, bool disableGamma, bool matcap);
    void selectionContextChanged(int contextKind);  // 0=Default, 1=Edge, 2=Face, 3=Body
    /** Request a temporary status bar message (e.g. "Point is fixed", solver error). */
    void statusMessageRequested(const QString& message);

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
    void activateArcTool();
    void activateEllipseTool();
    void activateTrimTool();
    void activateMirrorTool();
    void deactivateTool();
    void setReferenceSketch(const QString& sketchId);
    bool activateExtrudeTool();
    bool activateRevolveTool();
    bool activateFilletTool();
    bool activateShellTool();
    void setDebugToggles(bool normals, bool depth, bool wireframe, bool disableGamma, bool matcap);
    void setRenderLightRig(const QVector3D& keyDir,
                           const QVector3D& fillDir,
                           float fillIntensity,
                           float ambientIntensity,
                           const QVector3D& hemiUpDir,
                           const QVector3D& gradientDir,
                           float gradientStrength);

    // Snap settings
    void updateSnapSettings(const SnapSettingsPanel::SnapSettings& settings);

    // Move Sketch mode (dedicated tool for translating sketch via plane origin)
    void setMoveSketchMode(bool active);
    bool isMoveSketchModeActive() const { return m_moveSketchModeActive; }
    void setMoveSketchModeChangedCallback(std::function<void(bool)> callback);

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
    void mouseDoubleClickEvent(QMouseEvent* event) override;
    void mouseMoveEvent(QMouseEvent* event) override;
    void mouseReleaseEvent(QMouseEvent* event) override;
    void wheelEvent(QWheelEvent* event) override;
    void resizeEvent(QResizeEvent* event) override;
    void keyPressEvent(QKeyEvent* event) override;
    void leaveEvent(QEvent* event) override;

    // Touch/gesture events (for trackpad)
    bool event(QEvent* event) override;

private:
    struct RenderTuning {
        QVector3D keyLightDir{ -0.4f, 0.5f, 0.75f };
        QVector3D fillLightDir{ 0.6f, -0.2f, 0.55f };
        float fillLightIntensity = 0.35f;
        float ambientIntensity = 0.25f;
        QVector3D hemiUpDir{ 0.0f, 1.0f, 0.0f };
        QVector3D gradientDir{ 0.0f, 1.0f, 0.0f };
        float gradientStrength = 0.08f;
    };

    void updateModelSelectionFilter();
    void handleModelSelectionChanged();
    core::sketch::Vec2d screenToSketchPlane(const QPoint& screenPos,
                                            const core::sketch::SketchPlane& plane) const;
    app::selection::PickResult buildReferenceSketchPickResult(const QPoint& screenPos);
    app::selection::PickResult buildModelPickResult(const QPoint& screenPos);
    void updateSketchRenderingState();
    void handlePan(float dx, float dy);
    void handleOrbit(float dx, float dy);
    void handleZoom(float delta);
    bool isNativeZoomActive() const;
    void updatePlaneSelectionHover(const QPoint& screenPos);
    bool pickPlaneSelection(const QPoint& screenPos, int* outIndex) const;
    void drawPlaneSelectionOverlay(const QMatrix4x4& viewProjection);
    void drawModelSelectionOverlay(const QMatrix4x4& viewProjection);
    void drawModelToolOverlay(const QMatrix4x4& viewProjection);
    QMatrix4x4 buildViewProjection() const;
    QSize viewportSize() const;
    void syncModelMeshes();
    std::string resolveActiveSketchId() const;
    void updateSketchSelectionFromManager();
    void updateSketchHoverFromManager();
    app::selection::PickResult buildSketchPickResult(const QPoint& screenPos) const;
    QStringList buildDeepSelectLabels(const std::vector<app::selection::SelectionItem>& candidates) const;
    
    // Snap integration
    void updateSnapGeometry();

    // Animation
    void animateCamera(const CameraState& targetState);
    void setExtrudeToolActive(bool active);
    void setRevolveToolActive(bool active);
    void setFilletToolActive(bool active);
    void setPushPullToolActive(bool active);
    void setShellToolActive(bool active);

    std::unique_ptr<render::Camera3D> m_camera;
    std::unique_ptr<render::Grid3D> m_grid;
    std::unique_ptr<render::BodyRenderer> m_bodyRenderer;
    std::unique_ptr<core::sketch::SketchRenderer> m_sketchRenderer;
    std::unique_ptr<core::sketch::tools::SketchToolManager> m_toolManager;
    std::unique_ptr<ui::tools::ModelingToolManager> m_modelingToolManager;
    app::commands::CommandProcessor* m_commandProcessor = nullptr;
    bool m_extrudeToolActive = false;
    bool m_revolveToolActive = false;
    bool m_filletToolActive = false;
    bool m_shellToolActive = false;
    ViewCube* m_viewCube = nullptr;
    DimensionEditor* m_dimensionEditor = nullptr;
    QVariantAnimation* m_cameraAnimation = nullptr;
    app::selection::SelectionManager* m_selectionManager = nullptr;
    selection::DeepSelectPopup* m_deepSelectPopup = nullptr;
    std::unique_ptr<selection::SketchPickerAdapter> m_sketchPicker;
    std::unique_ptr<selection::ModelPickerAdapter> m_modelPicker;
    std::string m_previewHiddenBodyId;
    std::vector<app::selection::SelectionItem> m_pendingCandidates;
    app::selection::ClickModifiers m_pendingModifiers;
    QPoint m_pendingClickPos;
    bool m_pendingShellFaceToggle = false;

    // Sketch mode
    core::sketch::Sketch* m_activeSketch = nullptr;
    std::string m_activeSketchId;
    core::sketch::Sketch* m_referenceSketch = nullptr;
    std::string m_referenceSketchId;
    bool m_inSketchMode = false;
    bool m_indicatorHovered = false;
    bool m_planeSelectionActive = false;
    int m_planeHoverIndex = -1;

    // Sketch interaction state (point drag / move sketch / region move)
    enum class SketchInteractionState {
        Idle,
        PendingPointDrag,
        PointDragging,
        PendingSketchMove,
        SketchMoving,
        PendingRegionMove,
        RegionMoving
    };
    static constexpr int kPointDragThresholdPixels = 4;
    SketchInteractionState m_sketchInteractionState = SketchInteractionState::Idle;
    QPoint m_sketchPressPos;
    std::string m_pointDragCandidateId;
    std::string m_selectedRegionId;
    std::string m_regionMoveCandidateId;
    bool m_moveSketchModeActive = false;
    core::sketch::Vec2d m_moveSketchLastSketchPos{0.0, 0.0};
    bool m_pointDragFailureFeedbackShown = false;
    std::function<void(bool)> m_moveSketchModeChangedCallback;

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
    double m_pixelScale = 1.0;

    // Debug rendering modes (F1-F4 toggles)
    bool m_debugNormals = false;
    bool m_debugDepth = false;
    bool m_wireframeOnly = false;
    bool m_disableGamma = false;
    bool m_useMatcap = false;  // F5: MatCap shading mode
    RenderTuning m_renderTuning;

    // Dynamic quality during navigation
    bool m_isNavigating = false;
    QTimer* m_navigationTimer = nullptr;

    // Signal connection management
    QMetaObject::Connection m_themeConnection;
};

} // namespace ui
} // namespace onecad

// Backward compatibility
using MainWindowViewport = onecad::ui::Viewport;

#endif // ONECAD_UI_VIEWPORT_H
