#include "Viewport.h"
#include "../../render/BodyRenderer.h"
#include "../../render/Camera3D.h"
#include "../../render/Grid3D.h"
#include "../../core/sketch/SketchRenderer.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/SketchConstraint.h"
#include "../../core/sketch/constraints/Constraints.h"
#include "../../core/sketch/tools/SketchToolManager.h"
#include "../../core/sketch/tools/SnapPreviewResolver.h"
#include "../../core/loop/RegionUtils.h"
#include "../../app/document/Document.h"
#include "../../app/selection/SelectionManager.h"
#include "../../app/selection/SelectionTypes.h"
#include "../viewcube/ViewCube.h"
#include "../theme/ThemeManager.h"
#include "../sketch/DimensionEditor.h"
#include "../viewport/SnapSettingsPanel.h"
#include "../selection/DeepSelectPopup.h"
#include "../selection/SketchPickerAdapter.h"
#include "../selection/ModelPickerAdapter.h"
#include "../tools/ModelingToolManager.h"

#include <QKeyEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPainterPathStroker>
#include <QPolygonF>
#include <QRectF>
#include <QString>
#include <QMouseEvent>
#include <QWheelEvent>
#include <QGestureEvent>
#include <QPinchGesture>
#include <QPanGesture>
#include <QNativeGestureEvent>
#include <QApplication>
#include <QOpenGLContext>
#include <QSizePolicy>
#include <QVector2D>
#include <QEasingCurve>
#include <QtMath>
#include <QDebug>
#include <algorithm>
#include <array>
#include <cmath>
#include <limits>
#include <optional>
#include <unordered_set>

#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Vertex.hxx>
#include <TopoDS_Edge.hxx>
#include <BRep_Tool.hxx>
#include <gp_Pnt.hxx>

namespace onecad {
namespace ui {

namespace sketch = core::sketch;
namespace sketchTools = core::sketch::tools;

namespace {
std::vector<sketch::SketchRenderer::GuideLineInfo> toRendererGuides(
    const std::vector<sketchTools::GuideSegment>& guides) {
    std::vector<sketch::SketchRenderer::GuideLineInfo> rendererGuides;
    rendererGuides.reserve(guides.size());
    for (const auto& guide : guides) {
        rendererGuides.push_back({guide.origin, guide.target});
    }
    return rendererGuides;
}

constexpr float kOrbitSensitivity = 0.3f;
constexpr float kTrackpadPanScale = 1.0f;
constexpr float kTrackpadOrbitScale = 0.35f;
constexpr float kPinchZoomScale = 1000.0f;
constexpr float kWheelZoomShiftScale = 0.2f;
constexpr float kAngleDeltaToPixels = 1.0f / 8.0f;
constexpr qint64 kNativeZoomPanSuppressMs = 120;
constexpr float kPlaneSelectSize = 120.0f;
constexpr float kPlaneSelectHalf = kPlaneSelectSize * 0.5f;
constexpr float kThumbnailCameraAngle = 45.0f;
constexpr float kGridDepthScaleMin = 1.0f;
constexpr float kGridDepthScaleMax = 8.0f;
constexpr float kGridBoundsMaxScale = 6.0f;
} // namespace

namespace {
struct PlaneSelectionVisual {
    core::sketch::SketchPlane plane;
    QString label;
    QColor color;
};

std::array<PlaneSelectionVisual, 3> planeSelections(const ThemeViewportPlaneColors& colors) {
    return {{
        {core::sketch::SketchPlane::XY(), QStringLiteral("XY"), colors.xy},
        {core::sketch::SketchPlane::XZ(), QStringLiteral("XZ"), colors.xz},
        {core::sketch::SketchPlane::YZ(), QStringLiteral("YZ"), colors.yz}
    }};
}

struct PlaneAxes {
    QVector3D normal;
    QVector3D xAxis;
    QVector3D yAxis;
};

PlaneAxes buildPlaneAxes(const core::sketch::SketchPlane& plane) {
    PlaneAxes axes;
    axes.normal = QVector3D(plane.normal.x, plane.normal.y, plane.normal.z);
    axes.xAxis = QVector3D(plane.xAxis.x, plane.xAxis.y, plane.xAxis.z);
    axes.yAxis = QVector3D(plane.yAxis.x, plane.yAxis.y, plane.yAxis.z);

    if (axes.normal.lengthSquared() < 1e-8f) {
        axes.normal = QVector3D::crossProduct(axes.xAxis, axes.yAxis);
    }
    if (axes.normal.lengthSquared() < 1e-8f) {
        axes.normal = QVector3D(0.0f, 0.0f, 1.0f);
    }
    axes.normal.normalize();

    if (axes.xAxis.lengthSquared() < 1e-8f) {
        axes.xAxis = QVector3D::crossProduct(axes.yAxis, axes.normal);
    }
    if (axes.xAxis.lengthSquared() < 1e-8f) {
        axes.xAxis = (std::abs(axes.normal.z()) < 0.9f)
            ? QVector3D::crossProduct(axes.normal, QVector3D(0, 0, 1))
            : QVector3D::crossProduct(axes.normal, QVector3D(0, 1, 0));
    }

    axes.xAxis -= axes.normal * QVector3D::dotProduct(axes.normal, axes.xAxis);
    if (axes.xAxis.lengthSquared() < 1e-8f) {
        axes.xAxis = (std::abs(axes.normal.z()) < 0.9f)
            ? QVector3D::crossProduct(axes.normal, QVector3D(0, 0, 1))
            : QVector3D::crossProduct(axes.normal, QVector3D(0, 1, 0));
    }
    axes.xAxis.normalize();

    axes.yAxis = QVector3D::crossProduct(axes.normal, axes.xAxis).normalized();
    return axes;
}

QMatrix4x4 buildPlaneModelMatrix(const core::sketch::SketchPlane& plane) {
    PlaneAxes axes = buildPlaneAxes(plane);
    QVector3D origin(plane.origin.x, plane.origin.y, plane.origin.z);

    QMatrix4x4 matrix;
    matrix.setToIdentity();
    matrix.setColumn(0, QVector4D(axes.xAxis, 0.0f));
    matrix.setColumn(1, QVector4D(axes.yAxis, 0.0f));
    matrix.setColumn(2, QVector4D(axes.normal, 0.0f));
    matrix.setColumn(3, QVector4D(origin, 1.0f));
    return matrix;
}

sketch::Vec3d toVec3d(const QColor& color) {
    return {color.redF(), color.greenF(), color.blueF()};
}

void applySketchColors(const ThemeSketchColors& colors, sketch::SketchRenderStyle* style) {
    if (!style) {
        return;
    }
    style->colors.normalGeometry = toVec3d(colors.normalGeometry);
    style->colors.constructionGeometry = toVec3d(colors.constructionGeometry);
    style->colors.selectedGeometry = toVec3d(colors.selectedGeometry);
    style->colors.previewGeometry = toVec3d(colors.previewGeometry);
    style->colors.errorGeometry = toVec3d(colors.errorGeometry);
    style->colors.constraintIcon = toVec3d(colors.constraintIcon);
    style->colors.dimensionText = toVec3d(colors.dimensionText);
    style->colors.conflictHighlight = toVec3d(colors.conflictHighlight);
    style->colors.fullyConstrained = toVec3d(colors.fullyConstrained);
    style->colors.underConstrained = toVec3d(colors.underConstrained);
    style->colors.overConstrained = toVec3d(colors.overConstrained);
    style->colors.gridMajor = toVec3d(colors.gridMajor);
    style->colors.gridMinor = toVec3d(colors.gridMinor);
    style->colors.regionFill = toVec3d(colors.regionFill);
}

bool projectToScreen(const QMatrix4x4& viewProjection,
                     const QVector3D& worldPos,
                     float width,
                     float height,
                     QPointF* outPos) {
    QVector4D clip = viewProjection * QVector4D(worldPos, 1.0f);
    if (clip.w() <= 1e-6f) {
        return false;
    }

    QVector3D ndc = clip.toVector3D() / clip.w();
    float x = (ndc.x() * 0.5f + 0.5f) * width;
    float y = (1.0f - (ndc.y() * 0.5f + 0.5f)) * height;
    *outPos = QPointF(x, y);
    return true;
}

bool intersectRayWithPlane(const QVector3D& origin,
                           const QVector3D& direction,
                           const QVector3D& planeOrigin,
                           const QVector3D& planeNormal,
                           QVector3D* outPoint,
                           float* outDistance) {
    constexpr float kEpsilon = 1e-6f;
    float denom = QVector3D::dotProduct(direction, planeNormal);
    if (std::abs(denom) < kEpsilon) {
        return false;
    }

    float t = QVector3D::dotProduct(planeOrigin - origin, planeNormal) / denom;
    if (t < 0.0f) {
        return false;
    }

    if (outPoint) {
        *outPoint = origin + direction * t;
    }
    if (outDistance) {
        *outDistance = t;
    }
    return true;
}

bool intersectRayWithPlaneZ0(const QVector3D& origin,
                             const QVector3D& direction,
                             QVector3D* outPoint,
                             float* outDistance) {
    constexpr float kEpsilon = 1e-6f;
    if (std::abs(direction.z()) < kEpsilon) {
        return false;
    }

    float t = (0.0f - origin.z()) / direction.z();
    if (t < 0.0f) {
        return false;
    }

    if (outPoint) {
        *outPoint = origin + direction * t;
    }
    if (outDistance) {
        *outDistance = t;
    }
    return true;
}

bool computePlaneBoundsUV(const QMatrix4x4& viewProjection,
                         const core::sketch::SketchPlane& plane,
                         QRectF* outBounds) {
    bool invertible = false;
    QMatrix4x4 inverse = viewProjection.inverted(&invertible);
    if (!invertible) {
        return false;
    }

    PlaneAxes axes = buildPlaneAxes(plane);
    QVector3D planeOrigin(plane.origin.x, plane.origin.y, plane.origin.z);

    QVector2D minPoint(std::numeric_limits<float>::max(), std::numeric_limits<float>::max());
    QVector2D maxPoint(-std::numeric_limits<float>::max(), -std::numeric_limits<float>::max());
    int hitCount = 0;

    const float ndc[2] = {-1.0f, 1.0f};
    for (float x : ndc) {
        for (float y : ndc) {
            QVector4D nearPoint = inverse * QVector4D(x, y, -1.0f, 1.0f);
            QVector4D farPoint = inverse * QVector4D(x, y, 1.0f, 1.0f);
            if (qFuzzyIsNull(nearPoint.w()) || qFuzzyIsNull(farPoint.w())) {
                continue;
            }

            QVector3D p0 = nearPoint.toVector3D() / nearPoint.w();
            QVector3D p1 = farPoint.toVector3D() / farPoint.w();
            QVector3D dir = p1 - p0;

            QVector3D hit;
            if (!intersectRayWithPlane(p0, dir, planeOrigin, axes.normal, &hit, nullptr)) {
                continue;
            }

            QVector3D rel = hit - planeOrigin;
            float u = QVector3D::dotProduct(rel, axes.xAxis);
            float v = QVector3D::dotProduct(rel, axes.yAxis);
            if (!std::isfinite(u) || !std::isfinite(v)) {
                continue;
            }

            minPoint.setX(std::min(minPoint.x(), u));
            minPoint.setY(std::min(minPoint.y(), v));
            maxPoint.setX(std::max(maxPoint.x(), u));
            maxPoint.setY(std::max(maxPoint.y(), v));
            ++hitCount;
        }
    }

    if (hitCount < 4) {
        return false;
    }

    *outBounds = QRectF(QPointF(minPoint.x(), minPoint.y()),
                        QPointF(maxPoint.x(), maxPoint.y())).normalized();
    return true;
}

bool computePlaneBoundsXY(const QMatrix4x4& viewProjection, QRectF* outBounds) {
    bool invertible = false;
    QMatrix4x4 inverse = viewProjection.inverted(&invertible);
    if (!invertible) {
        return false;
    }

    QVector2D minPoint(std::numeric_limits<float>::max(), std::numeric_limits<float>::max());
    QVector2D maxPoint(-std::numeric_limits<float>::max(), -std::numeric_limits<float>::max());
    int hitCount = 0;

    constexpr float kPlaneZ = 0.0f;
    constexpr float kEpsilon = 1e-6f;
    const float ndc[2] = { -1.0f, 1.0f };

    for (float x : ndc) {
        for (float y : ndc) {
            QVector4D nearPoint = inverse * QVector4D(x, y, -1.0f, 1.0f);
            QVector4D farPoint = inverse * QVector4D(x, y, 1.0f, 1.0f);

            if (qFuzzyIsNull(nearPoint.w()) || qFuzzyIsNull(farPoint.w())) {
                continue;
            }

            QVector3D p0 = nearPoint.toVector3D() / nearPoint.w();
            QVector3D p1 = farPoint.toVector3D() / farPoint.w();
            QVector3D dir = p1 - p0;

            if (std::abs(dir.z()) < kEpsilon) {
                continue;
            }

            float t = (kPlaneZ - p0.z()) / dir.z();
            if (t < 0.0f) {
                continue;
            }

            QVector3D hit = p0 + dir * t;
            if (!std::isfinite(hit.x()) || !std::isfinite(hit.y()) || !std::isfinite(hit.z())) {
                continue;
            }
            minPoint.setX(std::min(minPoint.x(), hit.x()));
            minPoint.setY(std::min(minPoint.y(), hit.y()));
            maxPoint.setX(std::max(maxPoint.x(), hit.x()));
            maxPoint.setY(std::max(maxPoint.y(), hit.y()));
            ++hitCount;
        }
    }

    if (hitCount < 4) {
        return false;
    }

    *outBounds = QRectF(QPointF(minPoint.x(), minPoint.y()),
                        QPointF(maxPoint.x(), maxPoint.y())).normalized();
    return true;
}
} // namespace

Viewport::Viewport(QWidget* parent)
    : QOpenGLWidget(parent) {

    m_camera = std::make_unique<render::Camera3D>();
    m_grid = std::make_unique<render::Grid3D>();
    m_sketchPicker = std::make_unique<selection::SketchPickerAdapter>();
    m_modelPicker = std::make_unique<selection::ModelPickerAdapter>();
    // Note: SketchRenderer is created in initializeGL() when OpenGL context is ready

    setMouseTracking(true);
    setFocusPolicy(Qt::StrongFocus);
    
    // CRITICAL: Allow viewport to expand and fill available space
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
    
    // Prevent partial updates which can cause compositing artifacts on macOS
    setUpdateBehavior(QOpenGLWidget::NoPartialUpdate);
    
    // Enable gesture recognition for trackpad
    grabGesture(Qt::PinchGesture);
    grabGesture(Qt::PanGesture);

    m_nativeZoomTimer.start();

    // Setup navigation quality timer (debounce end of navigation)
    m_navigationTimer = new QTimer(this);
    m_navigationTimer->setSingleShot(true);
    connect(m_navigationTimer, &QTimer::timeout, this, [this]() {
        m_isNavigating = false;
        update();  // Final high-quality redraw
    });

    // Setup ViewCube
    m_viewCube = new ViewCube(this);
    m_viewCube->setCamera(m_camera.get());

    // Connect ViewCube -> Viewport
    connect(m_viewCube, &ViewCube::viewChanged, this, [this]() {
        update();
        emit cameraChanged();
    });

    // Connect Viewport -> ViewCube
    connect(this, &Viewport::cameraChanged, m_viewCube, &ViewCube::updateRotation);

    // Setup DimensionEditor for inline constraint editing
    m_dimensionEditor = new DimensionEditor(this);
    m_dimensionEditor->hide();
    connect(m_dimensionEditor, &DimensionEditor::valueConfirmed,
            this, [this](const QString& constraintId, double newValue) {
        if (!m_activeSketch) return;

        auto* constraint = m_activeSketch->getConstraint(constraintId.toStdString());
        if (!constraint) return;

        // Check if it's a dimensional constraint
        auto* dimConstraint = dynamic_cast<core::sketch::DimensionalConstraint*>(constraint);
        if (dimConstraint) {
            dimConstraint->setValue(newValue);
            m_activeSketch->solve();
            if (m_sketchRenderer) {
                m_sketchRenderer->updateGeometry();
                m_sketchRenderer->updateConstraints();
            }
            update();
            emit sketchUpdated();
        }
    });

    // Theme integration
    m_themeConnection = connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
                                this, &Viewport::updateTheme, Qt::UniqueConnection);
    updateTheme();

    // Selection manager
    m_selectionManager = new app::selection::SelectionManager(this);
    m_selectionManager->setMode(app::selection::SelectionMode::Model);
    {
        app::selection::SelectionFilter filter;
        filter.allowedKinds = {
            app::selection::SelectionKind::Vertex,
            app::selection::SelectionKind::Edge,
            app::selection::SelectionKind::Face,
            app::selection::SelectionKind::Body
        };
        m_selectionManager->setFilter(filter);
    }
    connect(m_selectionManager, &app::selection::SelectionManager::selectionChanged,
            this, &Viewport::updateSketchSelectionFromManager);
    connect(m_selectionManager, &app::selection::SelectionManager::hoverChanged,
            this, &Viewport::updateSketchHoverFromManager);
    connect(m_selectionManager, &app::selection::SelectionManager::selectionChanged,
            this, &Viewport::handleModelSelectionChanged);

    m_modelingToolManager = std::make_unique<tools::ModelingToolManager>(this);

    // Deep select popup
    m_deepSelectPopup = new selection::DeepSelectPopup(this);
    m_deepSelectPopup->hide();
    connect(m_deepSelectPopup, &selection::DeepSelectPopup::candidateHovered,
            this, [this](int index) {
        if (index < 0 || index >= static_cast<int>(m_pendingCandidates.size())) {
            return;
        }
        m_selectionManager->setHoverItem(m_pendingCandidates[static_cast<size_t>(index)]);
        update();
    });
    connect(m_deepSelectPopup, &selection::DeepSelectPopup::candidateSelected,
            this, [this](int index) {
        if (index < 0 || index >= static_cast<int>(m_pendingCandidates.size())) {
            return;
        }
        const auto candidate = m_pendingCandidates[static_cast<size_t>(index)];
        m_selectionManager->applySelectionCandidate(
            candidate,
            m_pendingModifiers,
            m_pendingClickPos
        );
        if (m_pendingShellFaceToggle && m_modelingToolManager) {
            m_modelingToolManager->toggleShellOpenFace(candidate);
        }
        m_pendingShellFaceToggle = false;
        update();
    });
    connect(m_deepSelectPopup, &selection::DeepSelectPopup::popupClosed,
            this, [this]() {
        m_pendingCandidates.clear();
        m_pendingClickPos = QPoint();
        m_pendingShellFaceToggle = false;
        if (m_selectionManager) {
            m_selectionManager->setHoverItem(std::nullopt);
        }
        update();
    });

    // Initialize camera animation
    m_cameraAnimation = new QVariantAnimation(this);
    m_cameraAnimation->setDuration(500); // 500ms transition
    m_cameraAnimation->setEasingCurve(QEasingCurve::OutCubic);
}

Viewport::~Viewport() {
    makeCurrent();
    if (m_sketchRenderer) {
        m_sketchRenderer->cleanup();
    }
    m_grid->cleanup();
    doneCurrent();
}

void Viewport::initializeGL() {
    initializeOpenGLFunctions();
    
    // Debug: Print OpenGL info
    const char* version = reinterpret_cast<const char*>(glGetString(GL_VERSION));
    const char* renderer = reinterpret_cast<const char*>(glGetString(GL_RENDERER));
    qDebug() << "OpenGL Version:" << (version ? version : "unknown");
    qDebug() << "OpenGL Renderer:" << (renderer ? renderer : "unknown");
    
    // Background color set via updateTheme
    glClearColor(m_backgroundColor.redF(), m_backgroundColor.greenF(), m_backgroundColor.blueF(), m_backgroundColor.alphaF());
    
    glEnable(GL_DEPTH_TEST);
    glEnable(GL_MULTISAMPLE);
    glEnable(GL_LINE_SMOOTH);
    glHint(GL_LINE_SMOOTH_HINT, GL_NICEST);
    
    // Disable states we don't want by default
    glDisable(GL_CULL_FACE);
    
    m_grid->initialize();

    m_bodyRenderer = std::make_unique<render::BodyRenderer>();
    m_bodyRenderer->initialize();

    // Create and initialize sketch renderer (requires OpenGL context)
    m_sketchRenderer = std::make_unique<sketch::SketchRenderer>();
    if (!m_sketchRenderer->initialize()) {
        qWarning() << "Failed to initialize SketchRenderer";
    }
    updateTheme();
}

void Viewport::updateTheme() {
    const ThemeDefinition& theme = ThemeManager::instance().currentTheme();
    m_backgroundColor = theme.viewport.background;
    if (m_grid) {
        m_grid->setMajorColor(theme.viewport.grid.major);
        m_grid->setMinorColor(theme.viewport.grid.minor);
        m_grid->setAxisColors(theme.viewport.grid.axisX,
                              theme.viewport.grid.axisY,
                              theme.viewport.grid.axisZ);
        m_grid->forceUpdate();
    }
    if (m_sketchRenderer) {
        sketch::SketchRenderStyle style = m_sketchRenderer->getStyle();
        applySketchColors(theme.sketch, &style);
        m_sketchRenderer->setStyle(style);
    }
    const auto& body = theme.viewport.body;
    m_renderTuning.keyLightDir = body.keyLightDir;
    m_renderTuning.fillLightDir = body.fillLightDir;
    m_renderTuning.fillLightIntensity = body.fillLightIntensity;
    m_renderTuning.ambientIntensity = body.ambientIntensity;
    m_renderTuning.hemiUpDir = body.hemiUpDir;
    m_renderTuning.gradientDir = body.ambientGradientDir;
    m_renderTuning.gradientStrength = body.ambientGradientStrength;
    update();
}

void Viewport::resizeGL(int w, int h) {
    m_width = w > 0 ? w : 1;
    m_height = h > 0 ? h : 1;
    
    // Handle Retina/High-DPI displays
    const qreal ratio = devicePixelRatio();
    glViewport(0, 0, static_cast<GLsizei>(m_width * ratio), static_cast<GLsizei>(m_height * ratio));
}

void Viewport::resizeEvent(QResizeEvent* event) {
    QOpenGLWidget::resizeEvent(event);
    if (m_viewCube) {
        // Position top-right with margin
        m_viewCube->move(width() - m_viewCube->width() - 20, 20);
    }
}

void Viewport::paintGL() {
    // Ensure viewport is set correctly with correct device pixel ratio
    const qreal ratio = devicePixelRatio();
    glViewport(0, 0, static_cast<GLsizei>(m_width * ratio), static_cast<GLsizei>(m_height * ratio));

    // Clear to background color
    glClearColor(m_backgroundColor.redF(), m_backgroundColor.greenF(), m_backgroundColor.blueF(), m_backgroundColor.alphaF());
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);

    // Reset depth test state
    glEnable(GL_DEPTH_TEST);
    glDepthFunc(GL_LESS);

    float aspectRatio = static_cast<float>(m_width) / static_cast<float>(m_height);
    QMatrix4x4 projection = m_camera->projectionMatrix(aspectRatio);
    QMatrix4x4 view = m_camera->viewMatrix();
    QMatrix4x4 viewProjection = projection * view;

    // Calculate pixel scale for sketch rendering and adaptive grid
    double pixelScale = 1.0;
    float dist = m_camera->distance();
    float fov = m_camera->fov();

    if (m_camera->projectionType() == render::Camera3D::ProjectionType::Orthographic) {
        double worldHeight = static_cast<double>(m_camera->orthoScale());
        pixelScale = worldHeight / (static_cast<double>(m_height) * ratio);
    } else {
        double halfFov = qDegreesToRadians(fov * 0.5f);
        double worldHeight = 2.0 * static_cast<double>(dist) * std::tan(halfFov);
        pixelScale = worldHeight / (static_cast<double>(m_height) * ratio);
    }

    if (pixelScale <= 0.0) {
        pixelScale = 1.0;
    }
    m_pixelScale = pixelScale;

    const double maxDimension = static_cast<double>(qMax(m_width, m_height));
    const double viewExtent = 0.5 * maxDimension * ratio * pixelScale;

    // Render grid
    QVector3D target = m_camera->target();
    QVector3D forward = m_camera->forward();
    QVector3D position = m_camera->position();

    if (m_inSketchMode && m_activeSketch) {
        const auto& sketchPlane = m_activeSketch->getPlane();
        PlaneAxes axes = buildPlaneAxes(sketchPlane);
        QVector3D planeOrigin(sketchPlane.origin.x, sketchPlane.origin.y, sketchPlane.origin.z);

        float planeDistance = dist;
        QVector3D planeAnchor = planeOrigin;
        intersectRayWithPlane(position, forward, planeOrigin, axes.normal, &planeAnchor, &planeDistance);

        float depthScale = 1.0f;
        if (m_camera->projectionType() == render::Camera3D::ProjectionType::Perspective && dist > 1e-4f) {
            depthScale = planeDistance / dist;
        }
        depthScale = std::clamp(depthScale, kGridDepthScaleMin, kGridDepthScaleMax);

        float viewHalf = static_cast<float>(viewExtent) * depthScale;
        QVector3D anchorDelta = planeAnchor - planeOrigin;
        QVector2D anchorUv(QVector3D::dotProduct(anchorDelta, axes.xAxis),
                           QVector3D::dotProduct(anchorDelta, axes.yAxis));

        QRectF fallbackBounds(QPointF(anchorUv.x() - viewHalf, anchorUv.y() - viewHalf),
                              QPointF(anchorUv.x() + viewHalf, anchorUv.y() + viewHalf));

        QRectF gridBounds = fallbackBounds;
        QRectF frustumBounds;
        bool hasFrustumBounds = computePlaneBoundsUV(viewProjection, sketchPlane, &frustumBounds);
        if (hasFrustumBounds) {
            float maxHalf = 0.5f * static_cast<float>(qMax(frustumBounds.width(),
                                                           frustumBounds.height()));
            QVector2D frustumCenter(static_cast<float>(frustumBounds.center().x()),
                                    static_cast<float>(frustumBounds.center().y()));
            float centerDistance = (frustumCenter - anchorUv).length();
            float maxAllowed = viewHalf * kGridBoundsMaxScale;

            if (maxHalf > maxAllowed || centerDistance > maxAllowed) {
                hasFrustumBounds = false;
            }
        }
        if (hasFrustumBounds) {
            gridBounds = frustumBounds;
        }

        QVector3D cameraDelta = position - planeOrigin;
        QVector2D fadeOrigin(QVector3D::dotProduct(cameraDelta, axes.xAxis),
                             QVector3D::dotProduct(cameraDelta, axes.yAxis));

        m_grid->render(viewProjection,
                       static_cast<float>(pixelScale),
                       QVector2D(static_cast<float>(gridBounds.left()), static_cast<float>(gridBounds.top())),
                       QVector2D(static_cast<float>(gridBounds.right()), static_cast<float>(gridBounds.bottom())),
                       fadeOrigin,
                       buildPlaneModelMatrix(sketchPlane));
    } else {
        float planeDistance = dist;
        QVector3D planeAnchor(target.x(), target.y(), 0.0f);
        intersectRayWithPlaneZ0(position, forward, &planeAnchor, &planeDistance);

        float depthScale = 1.0f;
        if (m_camera->projectionType() == render::Camera3D::ProjectionType::Perspective &&
            dist > 1e-4f) {
            depthScale = planeDistance / dist;
        }
        depthScale = std::clamp(depthScale, kGridDepthScaleMin, kGridDepthScaleMax);

        float viewHalf = static_cast<float>(viewExtent) * depthScale;
        QRectF fallbackBounds(QPointF(planeAnchor.x() - viewHalf, planeAnchor.y() - viewHalf),
                              QPointF(planeAnchor.x() + viewHalf, planeAnchor.y() + viewHalf));

        QRectF gridBounds = fallbackBounds;
        QRectF frustumBounds;
        bool hasFrustumBounds = computePlaneBoundsXY(viewProjection, &frustumBounds);
        if (hasFrustumBounds) {
            float maxHalf = 0.5f * static_cast<float>(qMax(frustumBounds.width(),
                                                           frustumBounds.height()));
            QVector2D anchor2d(planeAnchor.x(), planeAnchor.y());
            QVector2D frustumCenter(static_cast<float>(frustumBounds.center().x()),
                                    static_cast<float>(frustumBounds.center().y()));
            float centerDistance = (frustumCenter - anchor2d).length();
            float maxAllowed = viewHalf * kGridBoundsMaxScale;

            if (maxHalf > maxAllowed || centerDistance > maxAllowed) {
                hasFrustumBounds = false;
            }
        }
        if (hasFrustumBounds) {
            gridBounds = frustumBounds;
        }

        float minX = static_cast<float>(gridBounds.left());
        float maxX = static_cast<float>(gridBounds.right());
        float minY = static_cast<float>(gridBounds.top());
        float maxY = static_cast<float>(gridBounds.bottom());

        QVector2D fadeOrigin(position.x(), position.y());

        m_grid->render(viewProjection,
                       static_cast<float>(pixelScale),
                       QVector2D(minX, minY),
                       QVector2D(maxX, maxY),
                       fadeOrigin);
    }

    if (m_bodyRenderer) {
        render::BodyRenderer::RenderStyle style;
        const ThemeDefinition& theme = ThemeManager::instance().currentTheme();
        style.baseColor = theme.viewport.body.base;
        style.edgeColor = theme.viewport.body.edge;
        style.specularColor = theme.viewport.body.specular;
        style.rimColor = theme.viewport.body.rim;
        style.glowColor = theme.viewport.body.glow;
        style.highlightColor = theme.viewport.body.highlight;
        style.hemiSkyColor = theme.viewport.body.hemiSky;
        style.hemiGroundColor = theme.viewport.body.hemiGround;
        style.highlightStrength = 0.08f;
        style.drawGlow = true;
        style.glowAlpha = 0.18f;
        style.drawEdges = true;
        style.previewAlpha = 0.35f;
        if (!m_previewHiddenBodyId.empty()) {
            style.previewAlpha = 0.8f;
        }
        style.keyLightDir = m_renderTuning.keyLightDir;
        style.fillLightDir = m_renderTuning.fillLightDir;
        style.fillLightIntensity = m_renderTuning.fillLightIntensity;
        style.ambientIntensity = m_renderTuning.ambientIntensity;
        style.hemiUpDir = m_renderTuning.hemiUpDir;
        style.ambientGradientStrength = m_renderTuning.gradientStrength;
        style.ambientGradientDir = m_renderTuning.gradientDir;
        if (m_inSketchMode) {
            style.ghosted = true;
            style.ghostFactor = 0.6f;
            style.baseAlpha = 0.25f;
            style.edgeAlpha = 0.5f;
            style.glowAlpha = 0.12f;
            style.highlightStrength = 0.04f;
        }

        // Debug visualization modes (F1-F5)
        style.debugNormals = m_debugNormals;
        style.debugDepth = m_debugDepth;
        style.wireframeOnly = m_wireframeOnly;
        style.disableGamma = m_disableGamma;
        style.useMatcap = m_useMatcap;
        style.nearPlane = m_camera->nearPlane();
        style.farPlane = m_camera->farPlane();
        style.isOrtho = (m_camera->projectionType() == render::Camera3D::ProjectionType::Orthographic);

        // Dynamic quality: reduce during navigation for better responsiveness
        if (m_isNavigating) {
            style.drawEdges = false;
            style.drawGlow = false;
        }

        m_bodyRenderer->render(viewProjection, view, style);
    }

    // Render sketch(es)
    if (m_inSketchMode && m_activeSketch && m_sketchRenderer) {
        // In sketch mode: render only the active sketch with tool preview
        const auto& plane = m_activeSketch->getPlane();
        QVector3D target = m_camera->target();
        sketch::Vec3d target3d{target.x(), target.y(), target.z()};
        sketch::Vec2d center = plane.toSketch(target3d);

        sketch::Viewport sketchViewport;
        sketchViewport.center = center;
        sketchViewport.size = {
            static_cast<double>(m_width) * ratio * pixelScale,
            static_cast<double>(m_height) * ratio * pixelScale
        };
        sketchViewport.zoom = pixelScale > 0.0 ? 1.0 / pixelScale : 1.0;
        m_sketchRenderer->setViewport(sketchViewport);
        m_sketchRenderer->setPixelScale(pixelScale);

        // Render tool preview only when a sketch tool is active.
        // Otherwise no-tool interactions (e.g. point drag) own snap/guide preview state.
        if (m_toolManager && m_toolManager->hasActiveTool()) {
            m_toolManager->renderPreview();
        }

        m_sketchRenderer->render(view, projection);

        // Render preview dimensions overlay
        const auto& dims = m_sketchRenderer->getPreviewDimensions();
        if (!dims.empty()) {
            // Unbind GL context resources before QPainter to be safe
            // QPainter painter(this) automatically handles GL state for the widget
            QPainter painter(this);
            painter.setRenderHint(QPainter::Antialiasing);
            
            const ThemeDefinition& theme = ThemeManager::instance().currentTheme();
            QColor textColor = theme.viewport.overlay.previewDimensionText;
            QColor bgColor = theme.viewport.overlay.previewDimensionBackground;
            
            QFont font = painter.font();
            font.setPointSize(10);
            font.setBold(true);
            painter.setFont(font);

            PlaneAxes axes = buildPlaneAxes(plane);
            QVector3D origin(plane.origin.x, plane.origin.y, plane.origin.z);

            for (const auto& dim : dims) {
                // Calculate world position: origin + xAxis * x + yAxis * y
                QVector3D worldPos = origin + 
                                   axes.xAxis * dim.position.x + 
                                   axes.yAxis * dim.position.y;
                
                QPointF screenPos;
                if (projectToScreen(viewProjection, worldPos, 
                                  static_cast<float>(width()), 
                                  static_cast<float>(height()), 
                                  &screenPos)) {
                    
                    QString text = QString::fromStdString(dim.text);
                    QFontMetrics fm(font);
                    int textWidth = fm.horizontalAdvance(text);
                    int textHeight = fm.height();
                    int padding = 4;
                    
                    QRectF bgRect(screenPos.x() - textWidth / 2 - padding, 
                                screenPos.y() - textHeight / 2 - padding, 
                                textWidth + 2 * padding, 
                                textHeight + 2 * padding);
                    
                    painter.setPen(Qt::NoPen);
                    painter.setBrush(bgColor);
                    painter.drawRoundedRect(bgRect, 4, 4);
                    
                    painter.setPen(textColor);
                    painter.drawText(bgRect, Qt::AlignCenter, text);
                }
            }
        }

        // Render snap hint overlay
        const auto& snap = m_sketchRenderer->getSnapIndicator();
        if (snap.active && !snap.hintText.empty()) {
            QPainter painter(this);
            painter.setRenderHint(QPainter::Antialiasing);
            
            const ThemeDefinition& theme = ThemeManager::instance().currentTheme();
            QColor textColor = theme.viewport.overlay.previewDimensionText;
            QColor bgColor = theme.viewport.overlay.previewDimensionBackground;
            
            QFont font = painter.font();
            font.setPointSize(10);
            font.setBold(true);
            painter.setFont(font);

            PlaneAxes axes = buildPlaneAxes(plane);
            QVector3D origin(plane.origin.x, plane.origin.y, plane.origin.z);
            
            QVector3D worldPos = origin + 
                               axes.xAxis * snap.position.x + 
                               axes.yAxis * snap.position.y;
            
            QPointF screenPos;
            if (projectToScreen(viewProjection, worldPos, 
                              static_cast<float>(width()), 
                              static_cast<float>(height()), 
                              &screenPos)) {
                
                QString text = QString::fromStdString(snap.hintText);
                QFontMetrics fm(font);
                int textWidth = fm.horizontalAdvance(text);
                int textHeight = fm.height();
                int padding = 4;
                
                // Offset ~20px above snap point
                QRectF bgRect(screenPos.x() - textWidth / 2 - padding, 
                            screenPos.y() - textHeight / 2 - padding - 20, 
                            textWidth + 2 * padding, 
                            textHeight + 2 * padding);
                
                painter.setPen(Qt::NoPen);
                painter.setBrush(bgColor);
                painter.drawRoundedRect(bgRect, 4, 4);
                
                painter.setPen(textColor);
                painter.drawText(bgRect, Qt::AlignCenter, text);
            }
        }
    } else if (m_document && m_sketchRenderer) {
        // Not in sketch mode: render only the navigator-selected sketch (if any).
        if (m_referenceSketch) {
            if (m_documentSketchesDirty) {
                m_sketchRenderer->updateGeometry();
            }

            const auto& plane = m_referenceSketch->getPlane();
            QVector3D target = m_camera->target();
            sketch::Vec3d target3d{target.x(), target.y(), target.z()};
            sketch::Vec2d center = plane.toSketch(target3d);

            sketch::Viewport sketchViewport;
            sketchViewport.center = center;
            sketchViewport.size = {
                static_cast<double>(m_width) * ratio * pixelScale,
                static_cast<double>(m_height) * ratio * pixelScale
            };
            sketchViewport.zoom = pixelScale > 0.0 ? 1.0 / pixelScale : 1.0;
            m_sketchRenderer->setViewport(sketchViewport);
            m_sketchRenderer->setPixelScale(pixelScale);

            const sketch::SketchRenderStyle previousStyle = m_sketchRenderer->getStyle();
            sketch::SketchRenderStyle ghostStyle = previousStyle;
            ghostStyle.colors.normalGeometry.x *= 0.6;
            ghostStyle.colors.normalGeometry.y *= 0.6;
            ghostStyle.colors.normalGeometry.z *= 0.6;
            ghostStyle.colors.constructionGeometry.x *= 0.6;
            ghostStyle.colors.constructionGeometry.y *= 0.6;
            ghostStyle.colors.constructionGeometry.z *= 0.6;
            ghostStyle.colors.selectedGeometry.x *= 0.7;
            ghostStyle.colors.selectedGeometry.y *= 0.7;
            ghostStyle.colors.selectedGeometry.z *= 0.7;
            ghostStyle.regionOpacity *= 0.4f;
            ghostStyle.regionHoverOpacity *= 0.4f;
            ghostStyle.regionSelectedOpacity *= 0.6f;
            m_sketchRenderer->setStyle(ghostStyle);

            m_sketchRenderer->render(view, projection);

            m_sketchRenderer->setStyle(previousStyle);
            m_documentSketchesDirty = false;
        }
    }

    if (m_planeSelectionActive) {
        glDisable(GL_DEPTH_TEST);
        drawPlaneSelectionOverlay(viewProjection);
    }

    drawModelSelectionOverlay(viewProjection);
    drawModelToolOverlay(viewProjection);
}

void Viewport::mousePressEvent(QMouseEvent* event) {
    m_lastMousePos = event->pos();

    if (event->button() == Qt::LeftButton && m_deepSelectPopup &&
        m_deepSelectPopup->isVisible()) {
        m_deepSelectPopup->hide();
        m_pendingCandidates.clear();
        m_pendingClickPos = QPoint();
        m_pendingModifiers = {};
        m_pendingShellFaceToggle = false;
    }

    // Move Sketch mode: left press starts sketch move gesture
    if (m_inSketchMode && m_moveSketchModeActive && event->button() == Qt::LeftButton &&
        m_activeSketch && (!m_toolManager || !m_toolManager->hasActiveTool())) {
        m_sketchInteractionState = SketchInteractionState::SketchMoving;
        m_moveSketchLastSketchPos = screenToSketch(event->pos());
        setCursor(Qt::SizeAllCursor);
        update();
        return;
    }

    if (m_inSketchMode && m_sketchRenderer && event->button() == Qt::LeftButton &&
        (!m_toolManager || !m_toolManager->hasActiveTool())) {
        if (!m_selectionManager) {
            return;
        }
        app::selection::ClickModifiers modifiers;
        modifiers.shift = event->modifiers() & Qt::ShiftModifier;
        modifiers.toggle = event->modifiers() & (Qt::MetaModifier | Qt::ControlModifier);

        auto pickResult = buildSketchPickResult(event->pos());

        std::optional<app::selection::SelectionItem> bestPointForDrag;
        for (const auto& hit : pickResult.hits) {
            if (hit.kind == app::selection::SelectionKind::SketchPoint) {
                if (!bestPointForDrag.has_value() ||
                    hit.screenDistance < bestPointForDrag->screenDistance) {
                    bestPointForDrag = hit;
                }
            }
        }

        // Point drag must win over deep-select ambiguity at line endpoints.
        if (!m_moveSketchModeActive && bestPointForDrag.has_value()) {
            m_selectionManager->applySelectionCandidate(*bestPointForDrag, modifiers, event->pos());
            m_sketchInteractionState = SketchInteractionState::PendingPointDrag;
            m_sketchPressPos = event->pos();
            m_pointDragCandidateId = bestPointForDrag->id.elementId;
            m_pointDragFailureFeedbackShown = false;
            m_selectedRegionId.clear();
            update();
            return;
        }

        auto action = m_selectionManager->handleClick(pickResult, modifiers, event->pos());

        if (action.needsDeepSelect) {
            m_pendingCandidates = action.candidates;
            m_pendingModifiers = modifiers;
            m_pendingClickPos = event->pos();
            QStringList labels = buildDeepSelectLabels(m_pendingCandidates);
            m_deepSelectPopup->setCandidateLabels(labels);
            QPoint popupPos = mapToGlobal(event->pos() + QPoint(12, 12));
            m_deepSelectPopup->showAt(popupPos);
            if (!m_pendingCandidates.empty()) {
                m_selectionManager->setHoverItem(m_pendingCandidates.front());
            }
            m_sketchInteractionState = SketchInteractionState::Idle;
            m_pointDragCandidateId.clear();
            return;
        }

        // Candidate for point drag, region move, or sketch move
        if (!m_moveSketchModeActive) {
            auto top = m_selectionManager->topCandidate(pickResult);
            bool hitInSelectedRegion = false;
            bool hitSelectedRegionFill = false;
            if (!m_selectedRegionId.empty() && top.has_value()) {
                if (top->kind == app::selection::SelectionKind::SketchEdge) {
                    std::vector<sketch::EntityID> regionEntityIds =
                        core::loop::getEntityIdsInRegion(*m_activeSketch, m_selectedRegionId);
                    hitInSelectedRegion =
                        std::find(regionEntityIds.begin(), regionEntityIds.end(), top->id.elementId) !=
                        regionEntityIds.end();
                } else if (top->kind == app::selection::SelectionKind::SketchRegion &&
                           top->id.elementId == m_selectedRegionId) {
                    hitSelectedRegionFill = true;
                }
            }

            if (!m_selectedRegionId.empty() &&
                (!top.has_value() || hitInSelectedRegion || hitSelectedRegionFill)) {
                m_sketchInteractionState = SketchInteractionState::PendingRegionMove;
                m_sketchPressPos = event->pos();
                m_regionMoveCandidateId = m_selectedRegionId;
                m_pointDragCandidateId.clear();
            } else if (!top.has_value()) {
                m_sketchInteractionState = SketchInteractionState::PendingSketchMove;
                m_sketchPressPos = event->pos();
                m_pointDragCandidateId.clear();
                m_selectedRegionId.clear();
            } else {
                m_sketchInteractionState = SketchInteractionState::Idle;
                m_pointDragCandidateId.clear();
                m_selectedRegionId.clear();
            }
        } else {
            m_sketchInteractionState = SketchInteractionState::Idle;
            m_pointDragCandidateId.clear();
        }

        update();
        return;
    }

    // Check for indicator drag FIRST, before generic model picking.
    // This ensures that clicking "near" the arrow (tolerance zone) works even if
    // hovering over empty space or unrelated faces.
    if (!m_inSketchMode && !m_planeSelectionActive &&
        m_modelingToolManager && m_modelingToolManager->hasActiveTool()) {
        
        if (isMouseOverIndicator(event->pos())) {
            // Force start dragging
            if (m_modelingToolManager->handleMousePress(event->pos(), event->button())) {
                event->accept();
                return;
            }
        }
    }

    if (!m_inSketchMode && event->button() == Qt::LeftButton && !m_planeSelectionActive) {
        if (m_selectionManager && m_modelPicker) {
            app::selection::ClickModifiers modifiers;
            modifiers.shift = event->modifiers() & Qt::ShiftModifier;
            modifiers.toggle = event->modifiers() & (Qt::MetaModifier | Qt::ControlModifier);

            auto pickResult = m_referenceSketch
                ? buildModelPickResult(event->pos())
                : m_modelPicker->pick(event->pos(),
                                      static_cast<double>(sketch::constants::PICK_TOLERANCE_PIXELS),
                                      buildViewProjection(),
                                      viewportSize());

            auto topCandidate = m_selectionManager->topCandidate(pickResult);

            bool allowTool = false;
            if (!modifiers.shift && !modifiers.toggle &&
                m_modelingToolManager && m_modelingToolManager->hasActiveTool()) {
                const auto& selection = m_selectionManager->selection();
                if (topCandidate.has_value() && selection.size() == 1) {
                    app::selection::SelectionKey topKey{topCandidate->kind, topCandidate->id};
                    app::selection::SelectionKey selKey{selection.front().kind, selection.front().id};
                    if (topKey == selKey) {
                        allowTool = true;
                    }
                }
                if (!allowTool && topCandidate.has_value()) {
                    auto shellBodyId = m_modelingToolManager->activeShellBodyId();
                    if (shellBodyId.has_value() &&
                        (topCandidate->kind == app::selection::SelectionKind::Face ||
                         topCandidate->kind == app::selection::SelectionKind::Body) &&
                        topCandidate->id.ownerId == shellBodyId.value()) {
                        allowTool = true;
                    }
                }
            }

            if (allowTool && m_modelingToolManager->handleMousePress(event->pos(), event->button())) {
                event->accept();
                return;
            }

            bool shellTogglePending = false;
            if (modifiers.shift && m_modelingToolManager && topCandidate.has_value()) {
                auto shellBodyId = m_modelingToolManager->activeShellBodyId();
                if (shellBodyId.has_value() &&
                    topCandidate->kind == app::selection::SelectionKind::Face) {
                    shellTogglePending = true;
                    modifiers.toggle = true;
                    modifiers.shift = false;
                }
            }

            auto action = m_selectionManager->handleClick(pickResult, modifiers, event->pos());

            if (action.needsDeepSelect) {
                m_pendingCandidates = action.candidates;
                m_pendingModifiers = modifiers;
                m_pendingClickPos = event->pos();
                m_pendingShellFaceToggle = shellTogglePending;
                QStringList labels = buildDeepSelectLabels(m_pendingCandidates);
                m_deepSelectPopup->setCandidateLabels(labels);
                QPoint popupPos = mapToGlobal(event->pos() + QPoint(12, 12));
                m_deepSelectPopup->showAt(popupPos);
                if (!m_pendingCandidates.empty()) {
                    m_selectionManager->setHoverItem(m_pendingCandidates.front());
                }
                return;
            }

            if (shellTogglePending && topCandidate.has_value() && m_modelingToolManager) {
                m_modelingToolManager->toggleShellOpenFace(*topCandidate);
            }
            m_pendingShellFaceToggle = false;
            update();
            return;
        }
    }

    // Forward to sketch tool if active and left-click (or right-click for cancel)
    if (m_inSketchMode && m_toolManager && m_toolManager->hasActiveTool()) {
        if (event->button() == Qt::LeftButton || event->button() == Qt::RightButton) {
            sketch::Vec2d sketchPos = screenToSketch(event->pos());
            m_toolManager->handleMousePress(sketchPos, event->button());
            // Still allow right-click to orbit if tool is in Idle state
            if (event->button() == Qt::RightButton && !m_toolManager->activeTool()->isActive()) {
                m_isOrbiting = true;
                setCursor(Qt::ClosedHandCursor);
            }
            return;
        }
    }

    if (!m_inSketchMode && m_planeSelectionActive && event->button() == Qt::LeftButton) {
        int hitIndex = -1;
        if (pickPlaneSelection(event->pos(), &hitIndex)) {
            m_planeSelectionActive = false;
            m_planeHoverIndex = -1;
            setCursor(Qt::ArrowCursor);
            update();
            emit sketchPlanePicked(hitIndex);
            return;
        }
    }

    if (event->button() == Qt::RightButton) {
        m_isOrbiting = true;
        setCursor(Qt::ClosedHandCursor);
    } else if (event->button() == Qt::MiddleButton) {
        m_isPanning = true;
        setCursor(Qt::SizeAllCursor);
    }

    QOpenGLWidget::mousePressEvent(event);
}

void Viewport::mouseDoubleClickEvent(QMouseEvent* event) {
    if (!m_inSketchMode) {
        if (!m_selectionManager || !m_modelPicker) {
            QOpenGLWidget::mouseDoubleClickEvent(event);
            return;
        }

        // Handle double-click: sketch part → open for edit; otherwise body selection
        if (event->button() == Qt::LeftButton) {
            auto pickResult = m_referenceSketch
                ? buildModelPickResult(event->pos())
                : m_modelPicker->pick(event->pos(),
                                      static_cast<double>(sketch::constants::PICK_TOLERANCE_PIXELS),
                                      buildViewProjection(),
                                      viewportSize());

            auto topCandidate = m_selectionManager->topCandidate(pickResult);
            if (topCandidate.has_value()) {
                const auto kind = topCandidate->kind;
                if (kind == app::selection::SelectionKind::SketchPoint ||
                    kind == app::selection::SelectionKind::SketchEdge ||
                    kind == app::selection::SelectionKind::SketchRegion) {
                    const std::string sketchId = topCandidate->id.ownerId;
                    if (m_document && !sketchId.empty()) {
                        core::sketch::Sketch* sketch = m_document->getSketch(sketchId);
                        if (sketch) {
                            emit openSketchForEditRequested(QString::fromStdString(sketchId));
                            return;
                        }
                    }
                }

                // Construct a Body selection item from the hit
                app::selection::SelectionItem bodyItem;
                bodyItem.kind = app::selection::SelectionKind::Body;
                bodyItem.id = {topCandidate->id.ownerId, topCandidate->id.ownerId};
                bodyItem.priority = topCandidate->priority; // Keep priority or set custom
                bodyItem.depth = topCandidate->depth;
                bodyItem.worldPos = topCandidate->worldPos;
                
                // Replace current selection with this body
                app::selection::ClickModifiers modifiers;
                m_selectionManager->applySelectionCandidate(bodyItem, modifiers, event->pos());
                update();
                return;
            }
        }
    }

    if (!m_inSketchMode || !m_sketchRenderer || !m_activeSketch) {
        QOpenGLWidget::mouseDoubleClickEvent(event);
        return;
    }

    if (event->button() != Qt::LeftButton) {
        QOpenGLWidget::mouseDoubleClickEvent(event);
        return;
    }

    // Check if double-clicked on a constraint
    sketch::Vec2d sketchPos = screenToSketch(event->pos());
    sketch::ConstraintID constraintId = m_sketchRenderer->pickConstraint(sketchPos, 5.0);

    if (!constraintId.empty()) {
        auto* constraint = m_activeSketch->getConstraint(constraintId);
        if (constraint) {
            // Check if it's a dimensional constraint
            auto* dimConstraint = dynamic_cast<sketch::DimensionalConstraint*>(constraint);
            if (dimConstraint) {
                // Get the units based on constraint type
                QString units;
                switch (constraint->type()) {
                    case sketch::ConstraintType::Distance:
                    case sketch::ConstraintType::Radius:
                    case sketch::ConstraintType::Diameter:
                        units = "mm";
                        break;
                    case sketch::ConstraintType::Angle:
                        units = QString::fromUtf8("°");
                        break;
                    default:
                        units = "";
                        break;
                }

                // Show the dimension editor at the click position
                m_dimensionEditor->showForConstraint(
                    QString::fromStdString(constraintId),
                    dimConstraint->value(),
                    units,
                    event->pos()
                );
                return;
            }
        }
    }

    // Double-click on region fill, edge, or point: select whole region (all connected edges and vertices)
    auto pickResult = buildSketchPickResult(event->pos());
    auto top = m_selectionManager->topCandidate(pickResult);
    std::string regionIdToSelect;
    if (top.has_value() && top->kind == app::selection::SelectionKind::SketchRegion) {
        regionIdToSelect = top->id.elementId;
    } else if (top.has_value() &&
               (top->kind == app::selection::SelectionKind::SketchEdge ||
                top->kind == app::selection::SelectionKind::SketchPoint)) {
        auto regionIdOpt = core::loop::getRegionIdContainingEntity(*m_activeSketch, top->id.elementId);
        if (regionIdOpt.has_value()) {
            regionIdToSelect = *regionIdOpt;
        }
    }
    if (!regionIdToSelect.empty()) {
        std::vector<sketch::EntityID> entityIds =
            core::loop::getEntityIdsInRegion(*m_activeSketch, regionIdToSelect);
        std::string sketchIdStr = resolveActiveSketchId();
        std::vector<app::selection::SelectionItem> regionItems;
        regionItems.reserve(entityIds.size());
        for (const auto& eid : entityIds) {
            const auto* entity = m_activeSketch->getEntity(eid);
            if (!entity) {
                continue;
            }
            app::selection::SelectionItem item;
            item.id = {sketchIdStr, eid};
            item.priority = (entity->type() == sketch::EntityType::Point) ? 0 : 1;
            item.screenDistance = 0.0;
            if (entity->type() == sketch::EntityType::Point) {
                item.kind = app::selection::SelectionKind::SketchPoint;
            } else {
                item.kind = app::selection::SelectionKind::SketchEdge;
            }
            regionItems.push_back(item);
        }
        if (!regionItems.empty()) {
            m_selectionManager->replaceSelection(regionItems);
            m_selectedRegionId = regionIdToSelect;
            if (m_sketchRenderer) {
                m_sketchRenderer->clearRegionSelection();
                m_sketchRenderer->toggleRegionSelection(regionIdToSelect);
            }
            updateSketchSelectionFromManager();
            update();
            return;
        }
    }

    QOpenGLWidget::mouseDoubleClickEvent(event);
}

void Viewport::mouseMoveEvent(QMouseEvent* event) {
    QPoint delta = event->pos() - m_lastMousePos;
    m_lastMousePos = event->pos();

    if (!m_inSketchMode && m_modelingToolManager && m_modelingToolManager->isDragging() &&
        (event->buttons() & Qt::LeftButton)) {
        setCursor(Qt::ClosedHandCursor);
        if (m_modelingToolManager->handleMouseMove(event->pos())) {
            update();
            return;
        }
    }

    // Move Sketch gesture: translate all geometry by delta in sketch coordinates
    if (m_inSketchMode && m_sketchInteractionState == SketchInteractionState::SketchMoving &&
        m_activeSketch && (event->buttons() & Qt::LeftButton)) {
        sketch::Vec2d currentSketch = screenToSketch(event->pos());
        sketch::Vec2d delta;
        delta.x = currentSketch.x - m_moveSketchLastSketchPos.x;
        delta.y = currentSketch.y - m_moveSketchLastSketchPos.y;
        m_activeSketch->translateSketch(delta.x, delta.y);
        m_moveSketchLastSketchPos = currentSketch;
        if (m_sketchRenderer) {
            m_sketchRenderer->updateGeometry();
            update();
        }
        notifySketchUpdated();
        return;
    }

    // Move Region gesture: translate only the selected region
    if (m_inSketchMode && m_sketchInteractionState == SketchInteractionState::RegionMoving &&
        m_activeSketch && !m_regionMoveCandidateId.empty() && (event->buttons() & Qt::LeftButton)) {
        sketch::Vec2d currentSketch = screenToSketch(event->pos());
        sketch::Vec2d delta;
        delta.x = currentSketch.x - m_moveSketchLastSketchPos.x;
        delta.y = currentSketch.y - m_moveSketchLastSketchPos.y;
        m_activeSketch->translateSketchRegion(m_regionMoveCandidateId, delta.x, delta.y);
        m_moveSketchLastSketchPos = currentSketch;
        if (m_sketchRenderer) {
            m_sketchRenderer->updateGeometry();
            update();
        }
        notifySketchUpdated();
        return;
    }

    // Forward to sketch tool if active
    if (m_inSketchMode && m_toolManager && m_toolManager->hasActiveTool()) {
        sketch::Vec2d sketchPos = screenToSketch(event->pos());
        m_toolManager->handleMouseMove(sketchPos);
        if (m_selectionManager) {
            m_selectionManager->setHoverItem(std::nullopt);
        }
    } else if (m_inSketchMode && m_sketchRenderer && !m_isOrbiting && !m_isPanning &&
               (!m_deepSelectPopup || !m_deepSelectPopup->isVisible())) {
        // Sketch move: transition from PendingSketchMove when drag past threshold
        if (m_sketchInteractionState == SketchInteractionState::PendingSketchMove &&
            m_activeSketch && (event->buttons() & Qt::LeftButton)) {
            QPoint delta = event->pos() - m_sketchPressPos;
            int distSq = delta.x() * delta.x() + delta.y() * delta.y();
            if (distSq >= kPointDragThresholdPixels * kPointDragThresholdPixels) {
                m_sketchInteractionState = SketchInteractionState::SketchMoving;
                m_moveSketchLastSketchPos = screenToSketch(event->pos());
                setCursor(Qt::SizeAllCursor);
            }
        }
        // Region move: transition from PendingRegionMove when drag past threshold
        if (m_sketchInteractionState == SketchInteractionState::PendingRegionMove &&
            m_activeSketch && (event->buttons() & Qt::LeftButton)) {
            QPoint delta = event->pos() - m_sketchPressPos;
            int distSq = delta.x() * delta.x() + delta.y() * delta.y();
            if (distSq >= kPointDragThresholdPixels * kPointDragThresholdPixels) {
                bool regionContainsLockedReference = false;
                if (!m_regionMoveCandidateId.empty()) {
                    auto entityIds =
                        core::loop::getEntityIdsInRegion(*m_activeSketch, m_regionMoveCandidateId);
                    for (const auto& entityId : entityIds) {
                        if (m_activeSketch->isEntityReferenceLocked(entityId)) {
                            regionContainsLockedReference = true;
                            break;
                        }
                    }
                }
                if (regionContainsLockedReference) {
                    emit statusMessageRequested(tr("Reference geometry is locked"));
                    m_sketchInteractionState = SketchInteractionState::Idle;
                    m_regionMoveCandidateId.clear();
                } else {
                    m_sketchInteractionState = SketchInteractionState::RegionMoving;
                    m_moveSketchLastSketchPos = screenToSketch(event->pos());
                    setCursor(Qt::SizeAllCursor);
                }
            }
        }
        // Point drag state machine
        if (m_sketchInteractionState == SketchInteractionState::PendingPointDrag &&
            m_activeSketch && !m_pointDragCandidateId.empty()) {
            QPoint delta = event->pos() - m_sketchPressPos;
            int distSq = delta.x() * delta.x() + delta.y() * delta.y();
            if (distSq >= kPointDragThresholdPixels * kPointDragThresholdPixels) {
                if (m_activeSketch->isEntityReferenceLocked(m_pointDragCandidateId)) {
                    emit statusMessageRequested(tr("Reference geometry is locked"));
                    m_sketchInteractionState = SketchInteractionState::Idle;
                    m_pointDragCandidateId.clear();
                    clearPointDragSnapPreview();
                } else if (m_activeSketch->hasFixedConstraint(m_pointDragCandidateId)) {
                    emit statusMessageRequested(tr("Point is fixed"));
                    m_sketchInteractionState = SketchInteractionState::Idle;
                    m_pointDragCandidateId.clear();
                    clearPointDragSnapPreview();
                } else {
                    m_activeSketch->beginPointDrag(m_pointDragCandidateId);
                    m_sketchInteractionState = SketchInteractionState::PointDragging;
                    m_sketchRenderer->setEntitySelection(m_pointDragCandidateId,
                        sketch::SelectionState::Dragging);
                }
            }
        }
        if (m_sketchInteractionState == SketchInteractionState::PointDragging &&
            m_activeSketch && m_sketchRenderer && !m_pointDragCandidateId.empty()) {
            sketch::Vec2d sketchPos = screenToSketch(event->pos());
            sketch::Vec2d targetPos = sketchPos;
            if (event->modifiers() & Qt::ShiftModifier) {
                std::vector<sketch::Vec2d> freeDirs = m_activeSketch->getPointFreeDirections(m_pointDragCandidateId);
                if (freeDirs.empty()) {
                    clearPointDragSnapPreview();
                    update();
                    return;
                }
                auto* pt = m_activeSketch->getEntityAs<sketch::SketchPoint>(m_pointDragCandidateId);
                if (pt) {
                    sketch::Vec2d currentPos{pt->position().X(), pt->position().Y()};
                    sketch::Vec2d delta{sketchPos.x - currentPos.x, sketchPos.y - currentPos.y};
                    sketch::Vec2d projectedDelta{0.0, 0.0};
                    for (const auto& d : freeDirs) {
                        double dot = delta.x * d.x + delta.y * d.y;
                        projectedDelta.x += dot * d.x;
                        projectedDelta.y += dot * d.y;
                    }
                    targetPos.x = currentPos.x + projectedDelta.x;
                    targetPos.y = currentPos.y + projectedDelta.y;
                }
            }

            sketch::Vec2d dragTarget = targetPos;
            if (m_toolManager) {
                std::unordered_set<sketch::EntityID> excludeFromSnap{m_pointDragCandidateId};
                const auto snapResolution = sketchTools::resolveSnapForInputEvent(
                    m_toolManager->snapManager(),
                    targetPos,
                    *m_activeSketch,
                    excludeFromSnap,
                    std::nullopt,
                    false,
                    true);
                if (snapResolution.resolvedSnap.snapped) {
                    dragTarget = snapResolution.resolvedSnap.position;
                }
                applyPointDragSnapPreview(snapResolution);
            } else {
                clearPointDragSnapPreview();
            }

            auto result = m_activeSketch->solveWithDrag(m_pointDragCandidateId, dragTarget);
            if (result.success) {
                m_sketchRenderer->updateGeometry();
                updateSketchRenderingState();
                update();
            } else {
                if (!m_pointDragFailureFeedbackShown) {
                    m_pointDragFailureFeedbackShown = true;
                    QString msg = result.errorMessage.empty()
                        ? tr("Constrained or unsolved drag")
                        : QString::fromStdString(result.errorMessage);
                    emit statusMessageRequested(msg);
                }
                update();
            }
            return;
        }
        if (m_sketchInteractionState != SketchInteractionState::PointDragging &&
            m_sketchInteractionState != SketchInteractionState::RegionMoving) {
            if (m_moveSketchModeActive) {
                setCursor(Qt::SizeAllCursor);
            }
            auto pickResult = buildSketchPickResult(event->pos());
            m_selectionManager->updateHover(pickResult);
            update();
        }
    }

    if (!m_inSketchMode && !m_planeSelectionActive && !m_isOrbiting && !m_isPanning &&
        (!m_deepSelectPopup || !m_deepSelectPopup->isVisible())) {
        
        // Check if hovering over tool indicator
        bool overIndicator = isMouseOverIndicator(event->pos());
        if (m_indicatorHovered != overIndicator) {
            m_indicatorHovered = overIndicator;
            update();
        }

        if (overIndicator) {
            setCursor(Qt::PointingHandCursor);
            if (m_selectionManager) {
                m_selectionManager->setHoverItem(std::nullopt);
            }
            update(); // Ensure redraw for color change
        } else {
            if (m_modelingToolManager && !m_modelingToolManager->isDragging()) {
                setCursor(Qt::ArrowCursor);
            }
            if (m_selectionManager && m_modelPicker) {
                auto pickResult = m_referenceSketch
                    ? buildModelPickResult(event->pos())
                    : m_modelPicker->pick(event->pos(),
                                          static_cast<double>(sketch::constants::PICK_TOLERANCE_PIXELS),
                                          buildViewProjection(),
                                          viewportSize());
                m_selectionManager->updateHover(pickResult);
            }
        }
    }

    if (m_planeSelectionActive && !m_inSketchMode && !m_isOrbiting && !m_isPanning) {
        updatePlaneSelectionHover(event->pos());
    }

    if (m_isOrbiting) {
        handleOrbit(delta.x(), delta.y());
    } else if (m_isPanning) {
        handlePan(delta.x(), delta.y());
    }

    // Emit sketch coordinates if in sketch mode, otherwise screen coords
    if (m_inSketchMode && m_activeSketch) {
        sketch::Vec2d sketchPos = screenToSketch(event->pos());
        emit mousePositionChanged(sketchPos.x, sketchPos.y, 0.0);
    } else {
        emit mousePositionChanged(event->pos().x(), event->pos().y(), 0.0);
    }

    QOpenGLWidget::mouseMoveEvent(event);
}

void Viewport::mouseReleaseEvent(QMouseEvent* event) {
    // End Move Sketch gesture
    if (m_inSketchMode && event->button() == Qt::LeftButton &&
        m_sketchInteractionState == SketchInteractionState::SketchMoving) {
        m_sketchInteractionState = SketchInteractionState::Idle;
        m_moveSketchLastSketchPos = sketch::Vec2d{0.0, 0.0};
        setCursor(Qt::ArrowCursor);
        update();
        return;
    }

    // End Move Region gesture
    if (m_inSketchMode && event->button() == Qt::LeftButton &&
        m_sketchInteractionState == SketchInteractionState::RegionMoving) {
        m_sketchInteractionState = SketchInteractionState::Idle;
        m_regionMoveCandidateId.clear();
        m_moveSketchLastSketchPos = sketch::Vec2d{0.0, 0.0};
        setCursor(Qt::ArrowCursor);
        update();
        return;
    }

    // Finalize point drag (sketch mode, no tool)
    if (m_inSketchMode && event->button() == Qt::LeftButton &&
        m_sketchInteractionState == SketchInteractionState::PointDragging) {
        if (m_activeSketch) {
            m_activeSketch->endPointDrag();
        }
        if (m_activeSketch && m_sketchRenderer && !m_pointDragCandidateId.empty()) {
            m_activeSketch->solve();
            m_sketchRenderer->updateGeometry();
            m_sketchRenderer->updateConstraints();
            updateSketchRenderingState();
            updateSketchSelectionFromManager();
            notifySketchUpdated();
        }
        m_sketchInteractionState = SketchInteractionState::Idle;
        m_pointDragCandidateId.clear();
        m_pointDragFailureFeedbackShown = false;
        clearPointDragSnapPreview();
        update();
        return;
    }
    if (m_inSketchMode && event->button() == Qt::LeftButton &&
        (m_sketchInteractionState == SketchInteractionState::PendingPointDrag ||
         m_sketchInteractionState == SketchInteractionState::PendingSketchMove ||
         m_sketchInteractionState == SketchInteractionState::PendingRegionMove)) {
        m_sketchInteractionState = SketchInteractionState::Idle;
        m_pointDragCandidateId.clear();
        m_regionMoveCandidateId.clear();
        clearPointDragSnapPreview();
    }

    // Forward to sketch tool if active
    if (m_inSketchMode && m_toolManager && m_toolManager->hasActiveTool()) {
        sketch::Vec2d sketchPos = screenToSketch(event->pos());
        m_toolManager->handleMouseRelease(sketchPos, event->button());
    }

    if (!m_inSketchMode && m_modelingToolManager && m_modelingToolManager->isDragging()) {
        if (m_modelingToolManager->handleMouseRelease(event->pos(), event->button())) {
            if (!m_modelingToolManager->hasActiveTool()) {
                m_modelingToolManager->cancelActiveTool();
                setExtrudeToolActive(false);
                setRevolveToolActive(false);
                setFilletToolActive(false);
                setShellToolActive(false);
            }
            update();
            return;
        }
    }

    if (event->button() == Qt::RightButton) {
        m_isOrbiting = false;
    } else if (event->button() == Qt::MiddleButton) {
        m_isPanning = false;
    }

    setCursor(Qt::ArrowCursor);
    if (m_planeSelectionActive && !m_inSketchMode) {
        updatePlaneSelectionHover(event->pos());
    }
    QOpenGLWidget::mouseReleaseEvent(event);
}

void Viewport::wheelEvent(QWheelEvent* event) {
    const QPoint pixelDelta = event->pixelDelta();
    const QPoint angleDelta = event->angleDelta();
    const bool hasPixelDelta = !pixelDelta.isNull();
    const bool hasAngleDelta = !angleDelta.isNull();
    const bool isTrackpad = (event->phase() != Qt::NoScrollPhase) ||
        (hasPixelDelta && !hasAngleDelta);
    const bool pinchActive = m_pinchActive || isNativeZoomActive();
    const bool zoomModifier = event->modifiers() & Qt::ControlModifier;

    if (isTrackpad && pinchActive) {
        event->accept();
        return;
    }

    if (isTrackpad && zoomModifier && (hasPixelDelta || hasAngleDelta)) {
        QPointF delta = hasPixelDelta
            ? QPointF(pixelDelta)
            : QPointF(angleDelta) * kAngleDeltaToPixels;
        handleZoom(static_cast<float>(delta.y()));
        m_lastNativeZoomMs = m_nativeZoomTimer.elapsed();
        event->accept();
        return;
    }

    if (isTrackpad && (hasPixelDelta || hasAngleDelta)) {
        QPointF delta = hasPixelDelta
            ? QPointF(pixelDelta)
            : QPointF(angleDelta) * kAngleDeltaToPixels;

        if (event->modifiers() & Qt::ShiftModifier) {
            handleOrbit(delta.x() * kTrackpadOrbitScale, 
                        delta.y() * kTrackpadOrbitScale);
        } else {
            handlePan(delta.x() * kTrackpadPanScale, 
                      delta.y() * kTrackpadPanScale);
        }

        event->accept();
        return;
    }

    if (!hasAngleDelta) {
        event->ignore();
        return;
    }

    float delta = static_cast<float>(angleDelta.y());

    // Shift for slower zoom
    if (event->modifiers() & Qt::ShiftModifier) {
        delta *= kWheelZoomShiftScale;
    }

    handleZoom(delta);
    event->accept();
}

void Viewport::leaveEvent(QEvent* event) {
    // Clear hover state when mouse leaves viewport
    if (m_selectionManager) {
        m_selectionManager->setHoverItem(std::nullopt);
        update();
    }
    QOpenGLWidget::leaveEvent(event);
}

bool Viewport::event(QEvent* event) {
    if (event->type() == QEvent::NativeGesture) {
        QNativeGestureEvent* gestureEvent = static_cast<QNativeGestureEvent*>(event);

        if (gestureEvent->gestureType() == Qt::ZoomNativeGesture && !m_pinchActive) {
            qreal value = gestureEvent->value();
            if (value > 0.5 && value < 1.5) {
                value -= 1.0;
            }
            handleZoom(static_cast<float>(value * kPinchZoomScale));
            m_lastNativeZoomMs = m_nativeZoomTimer.elapsed();
            return true;
        }
    }

    if (event->type() == QEvent::Gesture) {
        QGestureEvent* gestureEvent = static_cast<QGestureEvent*>(event);
        
        // Handle pinch gesture (zoom)
        if (QPinchGesture* pinch = static_cast<QPinchGesture*>(
                gestureEvent->gesture(Qt::PinchGesture))) {
            
            if (pinch->state() == Qt::GestureStarted) {
                m_lastPinchScale = 1.0;
                m_pinchActive = true;
            }
            
            qreal scaleFactor = pinch->scaleFactor();
            qreal delta = (scaleFactor - m_lastPinchScale) * kPinchZoomScale;
            m_lastPinchScale = scaleFactor;
            
            handleZoom(static_cast<float>(delta));

            if (pinch->state() == Qt::GestureFinished ||
                pinch->state() == Qt::GestureCanceled) {
                m_pinchActive = false;
            }
            
            return true;
        }
        
        // Handle pan gesture (two-finger drag)
        if (QPanGesture* pan = static_cast<QPanGesture*>(
                gestureEvent->gesture(Qt::PanGesture))) {
            if (m_pinchActive || isNativeZoomActive()) {
                return true;
            }
            
            QPointF delta = pan->delta();
            
            // Check if Shift is held for orbit mode
            bool shiftHeld = QApplication::keyboardModifiers() & Qt::ShiftModifier;
            
            if (shiftHeld) {
                // Shift + two-finger = orbit
                handleOrbit(static_cast<float>(delta.x()) * kTrackpadOrbitScale,
                           static_cast<float>(delta.y()) * kTrackpadOrbitScale);
            } else {
                // Two-finger = pan
                handlePan(static_cast<float>(delta.x()) * kTrackpadPanScale,
                         static_cast<float>(delta.y()) * kTrackpadPanScale);
            }
            
            return true;
        }
    }
    
    return QOpenGLWidget::event(event);
}

void Viewport::beginPlaneSelection() {
    if (m_inSketchMode) {
        return;
    }
    if (m_selectionManager) {
        m_selectionManager->clearSelection();
    }
    if (m_modelingToolManager) {
        m_modelingToolManager->cancelActiveTool();
    }
    setExtrudeToolActive(false);
    setRevolveToolActive(false);
    setFilletToolActive(false);
    setShellToolActive(false);
    if (m_deepSelectPopup && m_deepSelectPopup->isVisible()) {
        m_deepSelectPopup->hide();
    }
    m_pendingCandidates.clear();
    m_pendingClickPos = QPoint();
    m_pendingModifiers = {};
    m_pendingShellFaceToggle = false;
    m_planeSelectionActive = true;
    m_planeHoverIndex = -1;
    update();
}

void Viewport::cancelPlaneSelection() {
    if (!m_planeSelectionActive) {
        return;
    }
    m_planeSelectionActive = false;
    m_planeHoverIndex = -1;
    setCursor(Qt::ArrowCursor);
    update();
    emit planeSelectionCancelled();
}

void Viewport::updateSketchRenderingState() {
    if (!m_inSketchMode || !m_activeSketch || !m_sketchRenderer) {
        return;
    }

    int dof = m_activeSketch->getDegreesOfFreedom();
    bool overConstrained = m_activeSketch->isOverConstrained();
    m_sketchRenderer->setDOF(overConstrained ? -1 : dof);
    m_sketchRenderer->updateConstraints();
}

void Viewport::applyPointDragSnapPreview(
    const core::sketch::tools::SnapInputResolution& snapResolution) {
    if (!m_sketchRenderer) {
        return;
    }

    bool showGuidePoints = true;
    bool showHints = true;
    if (m_toolManager) {
        const auto& snapManager = m_toolManager->snapManager();
        showGuidePoints = snapManager.showGuidePoints();
        showHints = snapManager.showSnappingHints();
    }

    const auto guides = showGuidePoints
        ? toRendererGuides(snapResolution.activeGuides)
        : std::vector<sketch::SketchRenderer::GuideLineInfo>{};
    m_sketchRenderer->setActiveGuides(guides);

    if (snapResolution.resolvedSnap.snapped) {
        const bool showGuide = showGuidePoints && snapResolution.resolvedSnap.hasGuide;
        const std::string hintText = showHints ? snapResolution.resolvedSnap.hintText : std::string();
        m_sketchRenderer->showSnapIndicator(snapResolution.resolvedSnap.position,
                                            snapResolution.resolvedSnap.type,
                                            snapResolution.resolvedSnap.guideOrigin,
                                            showGuide,
                                            hintText);
    } else {
        m_sketchRenderer->hideSnapIndicator();
    }
}

void Viewport::clearPointDragSnapPreview() {
    if (m_sketchRenderer) {
        m_sketchRenderer->hideSnapIndicator();
    }
}

void Viewport::setDebugToggles(bool normals, bool depth, bool wireframe, bool disableGamma, bool matcap) {
    if (normals && depth) {
        depth = false;
    }
    if (m_debugNormals == normals &&
        m_debugDepth == depth &&
        m_wireframeOnly == wireframe &&
        m_disableGamma == disableGamma &&
        m_useMatcap == matcap) {
        return;
    }

    m_debugNormals = normals;
    m_debugDepth = depth;
    m_wireframeOnly = wireframe;
    m_disableGamma = disableGamma;
    m_useMatcap = matcap;
    update();
    emit debugTogglesChanged(m_debugNormals, m_debugDepth, m_wireframeOnly, m_disableGamma, m_useMatcap);
}

void Viewport::setRenderLightRig(const QVector3D& keyDir,
                                 const QVector3D& fillDir,
                                 float fillIntensity,
                                 float ambientIntensity,
                                 const QVector3D& hemiUpDir,
                                 const QVector3D& gradientDir,
                                 float gradientStrength) {
    m_renderTuning.keyLightDir = keyDir;
    m_renderTuning.fillLightDir = fillDir;
    m_renderTuning.fillLightIntensity = fillIntensity;
    m_renderTuning.ambientIntensity = ambientIntensity;
    m_renderTuning.hemiUpDir = hemiUpDir;
    m_renderTuning.gradientDir = gradientDir;
    m_renderTuning.gradientStrength = gradientStrength;
    update();
}

void Viewport::handlePan(float dx, float dy) {
    m_isNavigating = true;
    m_navigationTimer->start(150);  // 150ms debounce
    m_camera->pan(dx, dy);
    update();
    emit cameraChanged();
}

void Viewport::handleOrbit(float dx, float dy) {
    m_isNavigating = true;
    m_navigationTimer->start(150);  // 150ms debounce
    // Sensitivity adjustment
    m_camera->orbit(-dx * kOrbitSensitivity, dy * kOrbitSensitivity);
    update();
    emit cameraChanged();
}

void Viewport::handleZoom(float delta) {
    m_isNavigating = true;
    m_navigationTimer->start(150);  // 150ms debounce
    m_camera->zoom(delta);
    update();
    emit cameraChanged();
}

bool Viewport::isNativeZoomActive() const {
    if (!m_nativeZoomTimer.isValid() || m_lastNativeZoomMs < 0) {
        return false;
    }

    return (m_nativeZoomTimer.elapsed() - m_lastNativeZoomMs) < kNativeZoomPanSuppressMs;
}

void Viewport::animateCamera(const CameraState& targetState) {
    if (!m_camera || !m_cameraAnimation) return;

    // Capture start state
    CameraState startState;
    startState.position = m_camera->position();
    startState.target = m_camera->target();
    startState.up = m_camera->up();
    startState.angle = m_camera->cameraAngle();

    // Stop any running animation
    m_cameraAnimation->stop();

    // Disconnect previous connections to avoid stacking slots
    m_cameraAnimation->disconnect(this);

    // Connect update slot
    connect(m_cameraAnimation, &QVariantAnimation::valueChanged, this, [this, startState, targetState](const QVariant& value) {
        float t = value.toFloat();

        // Linear interpolation for vectors
        // Note: For 'up' vector and camera rotation, SLERP would be mathematically ideal,
        // but LERP + normalize is sufficient for smooth camera transitions in this context.
        QVector3D newPos = startState.position * (1.0f - t) + targetState.position * t;
        QVector3D newTarget = startState.target * (1.0f - t) + targetState.target * t;
        QVector3D newUp = (startState.up * (1.0f - t) + targetState.up * t).normalized();

        // Apply to camera
        // CRITICAL: Do NOT interpolate angle during animation! setCameraAngle() adjusts
        // position when changing FOV within perspective mode (Camera3D.cpp:239), which
        // conflicts with our spatial interpolation trajectory. Instead, keep projection
        // mode constant during animation and switch only at the end.
        m_camera->setPosition(newPos);
        m_camera->setTarget(newTarget);
        m_camera->setUp(newUp);
        // angle will be set in finished callback

        update();
        emit cameraChanged();
    });

    // Cleanup when finished
    connect(m_cameraAnimation, &QVariantAnimation::finished, this, [this, targetState]() {
        // Set final spatial state first
        m_camera->setPosition(targetState.position);
        m_camera->setTarget(targetState.target);
        m_camera->setUp(targetState.up);

        // Set ortho scale BEFORE changing projection to prevent zoom adjustment
        // This ensures transitions preserve visual scale exactly
        m_camera->setOrthoScale(targetState.orthoScale);

        // Now safe to switch projection - won't recalculate distance
        m_camera->setCameraAngle(targetState.angle);

        // Validation: verify final state matches expected values
        float finalDist = m_camera->distance();
        float finalScale = (m_camera->projectionType() == render::Camera3D::ProjectionType::Perspective)
            ? 2.0f * finalDist * qTan(qDegreesToRadians(m_camera->fov() * 0.5f))
            : m_camera->orthoScale();
        qDebug() << "[AnimationFinished] angle=" << m_camera->cameraAngle()
                 << "distance=" << finalDist << "visualScale=" << finalScale
                 << "expected=" << targetState.orthoScale;

        update();
        emit cameraChanged();
    });

    // Start animation (0.0 to 1.0)
    m_cameraAnimation->setStartValue(0.0f);
    m_cameraAnimation->setEndValue(1.0f);
    m_cameraAnimation->start();
}

// Sketch mode
void Viewport::enterSketchMode(sketch::Sketch* sketch) {
    if (m_inSketchMode || !sketch) return;

    if (m_modelingToolManager) {
        m_modelingToolManager->cancelActiveTool();
    }
    setExtrudeToolActive(false);
    setRevolveToolActive(false);

    m_activeSketch = sketch;
    m_activeSketchId.clear();
    m_activeSketchId = resolveActiveSketchId();
    m_inSketchMode = true;
    m_planeSelectionActive = false;
    m_planeHoverIndex = -1;
    if (m_selectionManager) {
        m_selectionManager->setMode(app::selection::SelectionMode::Sketch);
        app::selection::SelectionFilter filter;
        filter.allowedKinds = {
            app::selection::SelectionKind::SketchPoint,
            app::selection::SelectionKind::SketchEdge,
            app::selection::SelectionKind::SketchRegion,
            app::selection::SelectionKind::SketchConstraint
        };
        m_selectionManager->setFilter(filter);
    }

    // Reset any active tool
    if (m_toolManager) {
        m_toolManager->deactivateTool();
        m_toolManager->setSketch(m_activeSketch);
        m_toolManager->setRenderer(m_sketchRenderer.get());
    }

    // Store current camera state
    m_savedCameraPosition = m_camera->position();
    m_savedCameraTarget = m_camera->target();
    m_savedCameraUp = m_camera->up();
    m_savedCameraAngle = m_camera->cameraAngle();

    // Calculate current visual scale (world height visible in viewport)
    // This preserves zoom level when switching projection modes
    float currentVisualScale;
    float currentDist = m_camera->distance();
    if (m_camera->projectionType() == render::Camera3D::ProjectionType::Perspective) {
        float halfFovRad = qDegreesToRadians(m_camera->fov() * 0.5f);
        currentVisualScale = 2.0f * currentDist * qTan(halfFovRad);
        qDebug() << "[EnterSketch] Perspective→Ortho: dist=" << currentDist
                 << "FOV=" << m_camera->fov() << "visualScale=" << currentVisualScale;
    } else {
        currentVisualScale = m_camera->orthoScale();
        qDebug() << "[EnterSketch] Ortho→Ortho: preserving orthoScale=" << currentVisualScale;
    }

    // Align camera to sketch plane and switch to orthographic
    const auto& plane = sketch->getPlane();

    // Use plane vectors directly for camera alignment
    // Normal -> View direction (from camera towards target)
    // Y-Axis -> Camera Up vector
    QVector3D normal(plane.normal.x, plane.normal.y, plane.normal.z);
    QVector3D up(plane.yAxis.x, plane.yAxis.y, plane.yAxis.z);

    // Ensure vectors are normalized
    normal.normalize();
    up.normalize();

    QVector3D target(plane.origin.x, plane.origin.y, plane.origin.z);
    float dist = m_camera->distance();

    CameraState targetState;
    targetState.target = target;
    targetState.up = up;
    targetState.position = target + normal * dist;
    targetState.angle = 0.0f; // 0° = orthographic
    targetState.orthoScale = currentVisualScale;  // Preserve zoom level

    // Animate to sketch view
    animateCamera(targetState);

    // Bind sketch to renderer
    if (m_sketchRenderer) {
        m_sketchRenderer->setSketch(sketch);
        m_sketchRenderer->updateGeometry();
        updateSketchRenderingState();
    }

    // Initialize tool manager
    m_toolManager = std::make_unique<sketchTools::SketchToolManager>(this);
    m_toolManager->setSketch(sketch);
    m_toolManager->setRenderer(m_sketchRenderer.get());

    // Connect tool signals
    connect(m_toolManager.get(), &sketchTools::SketchToolManager::geometryCreated, this, [this]() {
        if (m_sketchRenderer) {
            m_sketchRenderer->updateGeometry();
            updateSketchRenderingState();
        }
        update();
        emit sketchUpdated();  // Notify DOF changes
    });
    connect(m_toolManager.get(), &sketchTools::SketchToolManager::updateRequested, this, [this]() {
        update();
    });

    updateSnapGeometry();

    update();
    emit sketchModeChanged(true);
}

void Viewport::exitSketchMode() {
    if (!m_inSketchMode) return;

    if (m_activeSketch) {
        m_activeSketch->endPointDrag();
    }
    clearPointDragSnapPreview();

    m_inSketchMode = false;
    m_activeSketch = nullptr;
    m_activeSketchId.clear();
    if (m_selectionManager) {
        m_selectionManager->setMode(app::selection::SelectionMode::Model);
        updateModelSelectionFilter();
    }

    // Reset any active tool
    if (m_toolManager) {
        m_toolManager->deactivateTool();
        m_toolManager->setSketch(m_activeSketch);
        m_toolManager->setRenderer(m_sketchRenderer.get());
    }

    updateSnapGeometry();

    // Rebind renderer to reference sketch (if any)
    if (m_sketchRenderer) {
        m_sketchRenderer->setSketch(m_referenceSketch);
    }

    // Mark document sketches dirty for rebuild
    m_documentSketchesDirty = true;

    // Restore camera to previous state with animation
    CameraState savedState;
    savedState.position = m_savedCameraPosition;
    savedState.target = m_savedCameraTarget;
    savedState.up = m_savedCameraUp;
    savedState.angle = m_savedCameraAngle;

    // Calculate ortho scale for transition
    // Edge case: if saved state was already ortho (angle ≈ 0°), preserve current ortho scale
    if (m_savedCameraAngle < 0.01f) {
        // Ortho → Ortho: just preserve current ortho scale (no projection change)
        savedState.orthoScale = m_camera->orthoScale();
        qDebug() << "[ExitSketch] Ortho→Ortho: preserving orthoScale=" << savedState.orthoScale;
    } else {
        // Ortho → Perspective: calculate ortho scale that produces correct distance
        // Formula: when setCameraAngle switches to perspective, it computes:
        //   newDistance = orthoScale / (2 * tan(fov/2))
        // So to get savedDistance, we need: orthoScale = savedDistance * 2 * tan(fov/2)
        float savedDistance = (m_savedCameraPosition - m_savedCameraTarget).length();
        float halfFovRad = qDegreesToRadians(m_savedCameraAngle * 0.5f);
        savedState.orthoScale = 2.0f * savedDistance * qTan(halfFovRad);
        qDebug() << "[ExitSketch] Ortho→Perspective: savedDist=" << savedDistance
                 << "savedFOV=" << m_savedCameraAngle << "requiredOrthoScale=" << savedState.orthoScale;
    }

    animateCamera(savedState);

    update();
    emit sketchModeChanged(false);
}

bool Viewport::activateExtrudeTool() {
    if (m_inSketchMode || !m_selectionManager || !m_modelingToolManager || !m_referenceSketch) {
        setExtrudeToolActive(false);
        setFilletToolActive(false);
        setShellToolActive(false);
        return false;
    }

    if (m_extrudeToolActive) {
        setRevolveToolActive(false);
        setFilletToolActive(false);
        setShellToolActive(false);
        return true;
    }

    const auto& selection = m_selectionManager->selection();
    if (selection.size() == 1 &&
        selection.front().kind == app::selection::SelectionKind::SketchRegion) {
        m_modelingToolManager->activateExtrude(selection.front());
        setRevolveToolActive(false);
        setFilletToolActive(false);
        setShellToolActive(false);
        const bool activated = m_modelingToolManager->hasActiveTool();
        setExtrudeToolActive(activated);
        return activated;
    }

    setExtrudeToolActive(false);
    setFilletToolActive(false);
    setShellToolActive(false);
    return false;
}

// Standard view slots
void Viewport::setFrontView() {
    m_camera->setFrontView();
    update();
    emit cameraChanged();
}

void Viewport::setBackView() {
    m_camera->setBackView();
    update();
    emit cameraChanged();
}

void Viewport::setLeftView() {
    m_camera->setLeftView();
    update();
    emit cameraChanged();
}

void Viewport::setRightView() {
    m_camera->setRightView();
    update();
    emit cameraChanged();
}

void Viewport::setTopView() {
    m_camera->setTopView();
    update();
    emit cameraChanged();
}

void Viewport::setBottomView() {
    m_camera->setBottomView();
    update();
    emit cameraChanged();
}

void Viewport::setIsometricView() {
    m_camera->setIsometricView();
    update();
    emit cameraChanged();
}

void Viewport::resetView() {
    m_camera->reset();
    update();
    emit cameraChanged();
}

void Viewport::setCameraAngle(float degrees) {
    m_camera->setCameraAngle(degrees);
    update();
    emit cameraChanged();
}

void Viewport::toggleGrid() {
    m_grid->setVisible(!m_grid->isVisible());
    update();
}

QImage Viewport::captureThumbnail(int maxSize) {
    if (maxSize <= 0) {
        return {};
    }
    if (!m_camera) {
        return {};
    }

    // Qt6 grabFramebuffer() handles MSAA internally
    const QVector3D savedPosition = m_camera->position();
    const QVector3D savedTarget = m_camera->target();
    const QVector3D savedUp = m_camera->up();
    const float savedCameraAngle = m_camera->cameraAngle();
    const float savedFov = m_camera->fov();
    const float savedOrthoScale = m_camera->orthoScale();

    m_camera->setCameraAngle(kThumbnailCameraAngle);
    m_camera->setIsometricView();

    QImage frame = grabFramebuffer();

    m_camera->setCameraAngle(savedCameraAngle);
    m_camera->setFov(savedFov);
    m_camera->setOrthoScale(savedOrthoScale);
    m_camera->setPosition(savedPosition);
    m_camera->setTarget(savedTarget);
    m_camera->setUp(savedUp);

    if (frame.isNull()) {
        return {};
    }
    frame.setDevicePixelRatio(1.0);

    QImage scaled = frame;
    if (frame.width() > maxSize || frame.height() > maxSize) {
        scaled = frame.scaled(maxSize, maxSize,
                              Qt::KeepAspectRatio,
                              Qt::SmoothTransformation);
    }

    QImage result(QSize(maxSize, maxSize), QImage::Format_ARGB32_Premultiplied);
    if (result.isNull()) {
        return {};
    }

    QColor background = m_backgroundColor.isValid() ? m_backgroundColor : QColor(32, 32, 32);
    background.setAlpha(255);
    result.fill(background);

    QPainter painter(&result);
    painter.setRenderHint(QPainter::SmoothPixmapTransform, true);
    const int x = (maxSize - scaled.width()) / 2;
    const int y = (maxSize - scaled.height()) / 2;
    painter.drawImage(QPoint(x, y), scaled);
    return result;
}

void Viewport::keyPressEvent(QKeyEvent* event) {
    if (m_planeSelectionActive && !m_inSketchMode && event->key() == Qt::Key_Escape) {
        cancelPlaneSelection();
        event->accept();
        return;
    }

    // Debug visualization toggles (F1-F5)
    switch (event->key()) {
    case Qt::Key_F1:
        setDebugToggles(!m_debugNormals, false, m_wireframeOnly, m_disableGamma, m_useMatcap);
        qDebug() << "Debug normals:" << (m_debugNormals ? "ON" : "OFF");
        event->accept();
        return;
    case Qt::Key_F2:
        setDebugToggles(false, !m_debugDepth, m_wireframeOnly, m_disableGamma, m_useMatcap);
        qDebug() << "Debug depth:" << (m_debugDepth ? "ON" : "OFF");
        event->accept();
        return;
    case Qt::Key_F3:
        setDebugToggles(m_debugNormals, m_debugDepth, !m_wireframeOnly, m_disableGamma, m_useMatcap);
        qDebug() << "Wireframe only:" << (m_wireframeOnly ? "ON" : "OFF");
        event->accept();
        return;
    case Qt::Key_F4:
        setDebugToggles(m_debugNormals, m_debugDepth, m_wireframeOnly, !m_disableGamma, m_useMatcap);
        qDebug() << "Gamma correction:" << (m_disableGamma ? "DISABLED" : "ENABLED");
        event->accept();
        return;
    case Qt::Key_F5:
        setDebugToggles(m_debugNormals, m_debugDepth, m_wireframeOnly, m_disableGamma, !m_useMatcap);
        qDebug() << "MatCap shading:" << (m_useMatcap ? "ON" : "OFF");
        event->accept();
        return;
    default:
        break;
    }

    // Esc exits Move Sketch mode (and cancels in-progress move)
    if (m_inSketchMode && m_moveSketchModeActive && event->key() == Qt::Key_Escape) {
        setMoveSketchMode(false);
        event->accept();
        return;
    }

    // G toggles Move Sketch mode when in sketch mode
    if (m_inSketchMode && event->key() == Qt::Key_G) {
        setMoveSketchMode(!m_moveSketchModeActive);
        event->accept();
        return;
    }

    // Forward to sketch tool if active
    if (m_inSketchMode && m_toolManager && m_toolManager->hasActiveTool()) {
        m_toolManager->handleKeyPress(static_cast<Qt::Key>(event->key()));
        event->accept();
        return;
    }

    if (!m_inSketchMode && m_modelingToolManager && m_modelingToolManager->hasActiveTool()) {
        if (event->key() == Qt::Key_Tab) {
            if (m_modelingToolManager->toggleFilletMode()) {
                update();
                event->accept();
                return;
            }
        }
        if (event->key() == Qt::Key_Return || event->key() == Qt::Key_Enter) {
            if (m_modelingToolManager->confirmShellFaceSelection()) {
                event->accept();
                return;
            }
        }
    }

    if (!m_inSketchMode && m_modelingToolManager &&
        m_modelingToolManager->hasActiveTool() &&
        event->key() == Qt::Key_Escape) {
        m_modelingToolManager->cancelActiveTool();
        setExtrudeToolActive(false);
        setRevolveToolActive(false);
        setFilletToolActive(false);
        setShellToolActive(false);
        event->accept();
        return;
    }

    QOpenGLWidget::keyPressEvent(event);
}

// Tool management
sketchTools::SketchToolManager* Viewport::toolManager() const {
    return m_toolManager.get();
}

sketch::SketchRenderer* Viewport::sketchRenderer() const {
    return m_sketchRenderer.get();
}

sketch::Vec2d Viewport::screenToSketch(const QPoint& screenPos) const {
    // Convert screen coordinates to sketch plane coordinates
    // This involves unprojecting from screen space through the camera
    // to the sketch plane

    if (!m_activeSketch || !m_camera) {
        return {0.0, 0.0};
    }

    return screenToSketchPlane(screenPos, m_activeSketch->getPlane());
}

sketch::Vec2d Viewport::screenToSketchPlane(const QPoint& screenPos,
                                            const sketch::SketchPlane& plane) const {
    if (!m_camera) {
        return {0.0, 0.0};
    }

    float aspectRatio = static_cast<float>(m_width) / static_cast<float>(m_height);

    // Get view and projection matrices
    QMatrix4x4 view = m_camera->viewMatrix();
    QMatrix4x4 projection = m_camera->projectionMatrix(aspectRatio);
    QMatrix4x4 viewProj = projection * view;
    bool invertible = false;
    QMatrix4x4 invViewProj = viewProj.inverted(&invertible);

    if (!invertible) {
        return {0.0, 0.0};
    }

    // Normalize screen coordinates to [-1, 1]
    float ndcX = (2.0f * screenPos.x() / m_width) - 1.0f;
    float ndcY = 1.0f - (2.0f * screenPos.y() / m_height);  // Y is flipped

    // Create ray in world space
    QVector4D nearPoint = invViewProj * QVector4D(ndcX, ndcY, -1.0f, 1.0f);
    QVector4D farPoint = invViewProj * QVector4D(ndcX, ndcY, 1.0f, 1.0f);

    if (std::abs(nearPoint.w()) < 1e-8f || std::abs(farPoint.w()) < 1e-8f) {
        return {0.0, 0.0};
    }

    QVector3D rayOrigin = nearPoint.toVector3D() / nearPoint.w();
    QVector3D rayEnd = farPoint.toVector3D() / farPoint.w();
    QVector3D rayDir = (rayEnd - rayOrigin).normalized();

    QVector3D planeOrigin(plane.origin.x, plane.origin.y, plane.origin.z);
    QVector3D planeNormal(plane.normal.x, plane.normal.y, plane.normal.z);

    // Ray-plane intersection
    float denom = QVector3D::dotProduct(rayDir, planeNormal);
    if (std::abs(denom) < 1e-8f) {
        // Ray parallel to plane - use closest point
        QVector3D toPlane = planeOrigin - rayOrigin;
        float distToPlane = QVector3D::dotProduct(toPlane, planeNormal);
        QVector3D closestPoint = rayOrigin + planeNormal * distToPlane;
        sketch::Vec3d worldPt{closestPoint.x(), closestPoint.y(), closestPoint.z()};
        return plane.toSketch(worldPt);
    }

    float t = QVector3D::dotProduct(planeOrigin - rayOrigin, planeNormal) / denom;
    QVector3D intersection = rayOrigin + rayDir * t;

    // Convert world point to sketch 2D coordinates
    sketch::Vec3d worldPt{intersection.x(), intersection.y(), intersection.z()};
    return plane.toSketch(worldPt);
}

void Viewport::updatePlaneSelectionHover(const QPoint& screenPos) {
    int hitIndex = -1;
    if (pickPlaneSelection(screenPos, &hitIndex)) {
        if (m_planeHoverIndex != hitIndex) {
            m_planeHoverIndex = hitIndex;
            update();
        }
        if (!m_isOrbiting && !m_isPanning) {
            setCursor(Qt::PointingHandCursor);
        }
        return;
    }

    if (m_planeHoverIndex != -1) {
        m_planeHoverIndex = -1;
        update();
    }
    if (!m_isOrbiting && !m_isPanning) {
        setCursor(Qt::ArrowCursor);
    }
}

QMatrix4x4 Viewport::buildViewProjection() const {
    if (!m_camera) {
        return QMatrix4x4();
    }
    float aspectRatio = static_cast<float>(m_width) / static_cast<float>(m_height);
    QMatrix4x4 projection = m_camera->projectionMatrix(aspectRatio);
    QMatrix4x4 view = m_camera->viewMatrix();
    return projection * view;
}

QSize Viewport::viewportSize() const {
    return QSize(m_width, m_height);
}

std::string Viewport::resolveActiveSketchId() const {
    if (!m_activeSketchId.empty()) {
        return m_activeSketchId;
    }
    if (!m_document || !m_activeSketch) {
        return {};
    }
    for (const auto& id : m_document->getSketchIds()) {
        if (m_document->getSketch(id) == m_activeSketch) {
            return id;
        }
    }
    return {};
}

void Viewport::updateSketchSelectionFromManager() {
    if (!m_sketchRenderer) {
        return;
    }

    m_sketchRenderer->clearSelection();
    m_sketchRenderer->clearRegionSelection();

    const bool hasSketchContext = m_inSketchMode || (m_referenceSketch != nullptr);
    if (!hasSketchContext || !m_selectionManager) {
        update();
        return;
    }

    for (const auto& item : m_selectionManager->selection()) {
        if (item.kind == app::selection::SelectionKind::SketchRegion) {
            m_sketchRenderer->toggleRegionSelection(item.id.elementId);
            continue;
        }
        if (m_inSketchMode &&
            (item.kind == app::selection::SelectionKind::SketchPoint ||
             item.kind == app::selection::SelectionKind::SketchEdge)) {
            m_sketchRenderer->setEntitySelection(item.id.elementId,
                                                 sketch::SelectionState::Selected);
        }
    }

    update();
}

void Viewport::handleModelSelectionChanged() {
    if (m_inSketchMode || !m_selectionManager || !m_modelingToolManager) {
        return;
    }

    const auto& selection = m_selectionManager->selection();
    // Forward selection change to tool manager (e.g. for Revolve axis picking)
    m_modelingToolManager->onSelectionChanged(selection);

    // Emit context based on selection kind
    int context = 0;  // Default
    if (!selection.empty()) {
        switch (selection.front().kind) {
            case app::selection::SelectionKind::Edge:
                context = 1;
                break;
            case app::selection::SelectionKind::Face:
                context = 2;
                break;
            case app::selection::SelectionKind::Body:
                context = 3;
                break;
            default:
                context = 0;
                break;
        }
    }
    emit selectionContextChanged(context);

    if (m_revolveToolActive) {
        return;
    }
    if (m_shellToolActive) {
        auto shellBodyId = m_modelingToolManager
            ? m_modelingToolManager->activeShellBodyId()
            : std::nullopt;
        if (!shellBodyId.has_value()) {
            m_modelingToolManager->cancelActiveTool();
            setShellToolActive(false);
        } else if (!selection.empty()) {
            const auto& front = selection.front();
            const bool sameBody = (front.id.ownerId == shellBodyId.value()) ||
                (front.id.elementId == shellBodyId.value());
            if (!sameBody) {
                m_modelingToolManager->cancelActiveTool();
                setShellToolActive(false);
            } else {
                return;
            }
        } else {
            return;
        }
    }

    // Auto-activate Extrude if selection is valid (SketchRegion OR Face)
    bool canExtrude = false;
    if (selection.size() == 1) {
        if (selection.front().kind == app::selection::SelectionKind::SketchRegion && m_referenceSketch) {
            canExtrude = true;
        } else if (selection.front().kind == app::selection::SelectionKind::Face) {
            canExtrude = true;
        }
    }

    if (canExtrude) {
        m_modelingToolManager->activateExtrude(selection.front());
        setFilletToolActive(false);
        setShellToolActive(false);
        setExtrudeToolActive(m_modelingToolManager->hasActiveTool());
        return;
    }

    m_modelingToolManager->cancelActiveTool();
    setExtrudeToolActive(false);
    setFilletToolActive(false);
    setShellToolActive(false);
}

void Viewport::setExtrudeToolActive(bool active) {
    if (m_extrudeToolActive == active) {
        return;
    }
    m_extrudeToolActive = active;
    emit extrudeToolActiveChanged(active);
}

void Viewport::setFilletToolActive(bool active) {
    if (m_filletToolActive == active) {
        return;
    }
    m_filletToolActive = active;
    emit filletToolActiveChanged(active);
}

void Viewport::setShellToolActive(bool active) {
    if (m_shellToolActive == active) {
        return;
    }
    m_shellToolActive = active;
    emit shellToolActiveChanged(active);
}

void Viewport::updateSketchHoverFromManager() {
    if (!m_sketchRenderer) {
        return;
    }

    m_sketchRenderer->setHoverEntity("");
    m_sketchRenderer->clearRegionHover();

    const bool hasSketchContext = m_inSketchMode || (m_referenceSketch != nullptr);
    if (!hasSketchContext || !m_selectionManager) {
        update();
        return;
    }

    auto hover = m_selectionManager->hover();
    if (!hover.has_value()) {
        update();
        return;
    }

    switch (hover->kind) {
        case app::selection::SelectionKind::SketchRegion:
            m_sketchRenderer->setRegionHover(hover->id.elementId);
            break;
        case app::selection::SelectionKind::SketchPoint:
        case app::selection::SelectionKind::SketchEdge:
            if (m_inSketchMode) {
                m_sketchRenderer->setHoverEntity(hover->id.elementId);
            }
            break;
        default:
            break;
    }

    update();
}

app::selection::PickResult Viewport::buildSketchPickResult(const QPoint& screenPos) const {
    if (!m_sketchRenderer || !m_activeSketch || !m_sketchPicker) {
        return {};
    }

    const double pixelScale = (m_pixelScale > 0.0) ? m_pixelScale : 1.0;
    const double tolerancePixels = static_cast<double>(sketch::constants::PICK_TOLERANCE_PIXELS);
    const std::string sketchId = resolveActiveSketchId();

    selection::SketchPickerAdapter::Options options;
    options.allowConstraints = true;
    options.allowRegions = true;

    sketch::Vec2d sketchPos = screenToSketch(screenPos);
    return m_sketchPicker->pick(*m_sketchRenderer,
                                *m_activeSketch,
                                sketchPos,
                                sketchId,
                                pixelScale,
                                tolerancePixels,
                                options);
}

app::selection::PickResult Viewport::buildReferenceSketchPickResult(const QPoint& screenPos) {
    if (!m_sketchRenderer || !m_referenceSketch || !m_sketchPicker || m_referenceSketchId.empty()) {
        return {};
    }

    const double pixelScale = (m_pixelScale > 0.0) ? m_pixelScale : 1.0;
    const double tolerancePixels = static_cast<double>(sketch::constants::PICK_TOLERANCE_PIXELS);

    selection::SketchPickerAdapter::Options options;
    options.allowConstraints = false;
    options.allowRegions = true;

    if (m_documentSketchesDirty) {
        m_sketchRenderer->updateGeometry();
        m_documentSketchesDirty = false;
    }

    sketch::Vec2d sketchPos = screenToSketchPlane(screenPos, m_referenceSketch->getPlane());
    return m_sketchPicker->pick(*m_sketchRenderer,
                                *m_referenceSketch,
                                sketchPos,
                                m_referenceSketchId,
                                pixelScale,
                                tolerancePixels,
                                options);
}

app::selection::PickResult Viewport::buildModelPickResult(const QPoint& screenPos) {
    app::selection::PickResult result;
    if (m_modelPicker) {
        result = m_modelPicker->pick(screenPos,
                                     static_cast<double>(sketch::constants::PICK_TOLERANCE_PIXELS),
                                     buildViewProjection(),
                                     viewportSize());
    }

    if (m_referenceSketch) {
        auto sketchResult = buildReferenceSketchPickResult(screenPos);
        // Prefer active reference-sketch entities/regions over coplanar model faces to avoid
        // hide/unhide workflows for region operations on face-hosted sketches.
        for (auto& hit : sketchResult.hits) {
            hit.priority -= 100;
        }
        result.hits.insert(result.hits.end(), sketchResult.hits.begin(), sketchResult.hits.end());
    }

    return result;
}

QStringList Viewport::buildDeepSelectLabels(
    const std::vector<app::selection::SelectionItem>& candidates) const {
    QStringList labels;
    labels.reserve(static_cast<int>(candidates.size()));

    for (const auto& item : candidates) {
        QString label;
        switch (item.kind) {
            case app::selection::SelectionKind::SketchPoint:
                label = tr("Sketch Point");
                break;
            case app::selection::SelectionKind::SketchEdge:
                label = tr("Sketch Edge");
                break;
            case app::selection::SelectionKind::SketchRegion:
                label = tr("Sketch Region");
                break;
            case app::selection::SelectionKind::SketchConstraint:
                label = tr("Sketch Constraint");
                break;
            case app::selection::SelectionKind::Vertex:
                label = tr("Vertex");
                break;
            case app::selection::SelectionKind::Edge:
                label = tr("Edge");
                break;
            case app::selection::SelectionKind::Face:
                label = tr("Face");
                break;
            case app::selection::SelectionKind::Body:
                label = tr("Body");
                break;
            default:
                label = tr("Entity");
                break;
        }

        if (!item.id.elementId.empty()) {
            label += " (" + QString::fromStdString(item.id.elementId) + ")";
        }
        labels.append(label);
    }
    return labels;
}

bool Viewport::pickPlaneSelection(const QPoint& screenPos, int* outIndex) const {
    if (!m_camera) {
        return false;
    }

    float aspectRatio = static_cast<float>(m_width) / static_cast<float>(m_height);
    QMatrix4x4 view = m_camera->viewMatrix();
    QMatrix4x4 projection = m_camera->projectionMatrix(aspectRatio);
    QMatrix4x4 viewProj = projection * view;
    bool invertible = false;
    QMatrix4x4 invViewProj = viewProj.inverted(&invertible);

    if (!invertible) {
        return false;
    }

    float ndcX = (2.0f * screenPos.x() / m_width) - 1.0f;
    float ndcY = 1.0f - (2.0f * screenPos.y() / m_height);

    QVector4D nearPoint = invViewProj * QVector4D(ndcX, ndcY, -1.0f, 1.0f);
    QVector4D farPoint = invViewProj * QVector4D(ndcX, ndcY, 1.0f, 1.0f);

    if (std::abs(nearPoint.w()) < 1e-8f || std::abs(farPoint.w()) < 1e-8f) {
        return false;
    }

    QVector3D rayOrigin = nearPoint.toVector3D() / nearPoint.w();
    QVector3D rayEnd = farPoint.toVector3D() / farPoint.w();
    QVector3D rayDir = (rayEnd - rayOrigin).normalized();

    float bestT = std::numeric_limits<float>::max();
    int bestIndex = -1;
    const auto selections = planeSelections(ThemeManager::instance().currentTheme().viewport.planes);

    for (int i = 0; i < static_cast<int>(selections.size()); ++i) {
        const auto& selection = selections[i];
        PlaneAxes axes = buildPlaneAxes(selection.plane);
        QVector3D origin(selection.plane.origin.x, selection.plane.origin.y, selection.plane.origin.z);

        float denom = QVector3D::dotProduct(rayDir, axes.normal);
        if (std::abs(denom) < 1e-8f) {
            continue;
        }

        float t = QVector3D::dotProduct(origin - rayOrigin, axes.normal) / denom;
        if (t < 0.0f) {
            continue;
        }

        QVector3D hitPoint = rayOrigin + rayDir * t;
        QVector3D rel = hitPoint - origin;
        float u = QVector3D::dotProduct(rel, axes.xAxis);
        float v = QVector3D::dotProduct(rel, axes.yAxis);

        if (std::abs(u) <= kPlaneSelectHalf && std::abs(v) <= kPlaneSelectHalf) {
            if (t < bestT) {
                bestT = t;
                bestIndex = i;
            }
        }
    }

    if (bestIndex >= 0) {
        if (outIndex) {
            *outIndex = bestIndex;
        }
        return true;
    }

    return false;
}

void Viewport::drawModelSelectionOverlay(const QMatrix4x4& viewProjection) {
    if (!m_selectionManager || !m_modelPicker || m_inSketchMode) {
        return;
    }

    const auto hover = m_selectionManager->hover();
    const auto& selection = m_selectionManager->selection();
    if (!hover.has_value() && selection.empty()) {
        return;
    }

    struct HighlightStyle {
        QColor faceFillHover;
        QColor faceOutlineHover;
        QColor faceFillSelected;
        QColor faceOutlineSelected;
        QColor edgeHover;
        QColor edgeSelected;
        QColor vertexHover;
        QColor vertexSelected;
    };

    HighlightStyle style;
    const ThemeViewportSelectionColors& themeSelection =
        ThemeManager::instance().currentTheme().viewport.selection;
    style.faceFillHover = themeSelection.faceFillHover;
    style.faceOutlineHover = themeSelection.faceOutlineHover;
    style.faceFillSelected = themeSelection.faceFillSelected;
    style.faceOutlineSelected = themeSelection.faceOutlineSelected;
    style.edgeHover = themeSelection.edgeHover;
    style.edgeSelected = themeSelection.edgeSelected;
    style.vertexHover = themeSelection.vertexHover;
    style.vertexSelected = themeSelection.vertexSelected;

    auto isSameItem = [](const app::selection::SelectionItem& a,
                         const app::selection::SelectionItem& b) {
        return a.kind == b.kind &&
               a.id.ownerId == b.id.ownerId &&
               a.id.elementId == b.id.elementId;
    };

    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing, true);

    auto drawFace = [&](const app::selection::SelectionItem& item, bool hovered) {
        std::vector<std::array<QVector3D, 3>> triangles;
        if (!m_modelPicker->getFaceTriangles(item.id.ownerId, item.id.elementId, triangles)) {
            return;
        }
        const QColor fill = hovered ? style.faceFillHover : style.faceFillSelected;
        const QColor outline = hovered ? style.faceOutlineHover : style.faceOutlineSelected;

        // Draw filled triangles WITHOUT outline (no internal mesh lines)
        painter.setPen(Qt::NoPen);
        painter.setBrush(fill);
        for (const auto& tri : triangles) {
            QPolygonF poly;
            bool projected = true;
            for (const auto& v : tri) {
                QPointF screenPos;
                if (!projectToScreen(viewProjection, v, static_cast<float>(m_width),
                                     static_cast<float>(m_height), &screenPos)) {
                    projected = false;
                    break;
                }
                poly << screenPos;
            }
            if (projected && poly.size() == 3) {
                painter.drawPolygon(poly);
            }
        }

        // Draw boundary edges only (from OCCT topology, not tessellation)
        std::vector<std::vector<QVector3D>> boundaryEdges;
        if (m_modelPicker->getFaceBoundaryEdges(item.id.ownerId, item.id.elementId, boundaryEdges)) {
            painter.setPen(QPen(outline, hovered ? 1.5 : 2.0));
            painter.setBrush(Qt::NoBrush);
            for (const auto& edgePts : boundaryEdges) {
                QPolygonF line;
                for (const auto& pt : edgePts) {
                    QPointF screenPos;
                    if (projectToScreen(viewProjection, pt, static_cast<float>(m_width),
                                        static_cast<float>(m_height), &screenPos)) {
                        line << screenPos;
                    }
                }
                if (line.size() >= 2) {
                    painter.drawPolyline(line);
                }
            }
        }
    };

    auto drawEdge = [&](const app::selection::SelectionItem& item, bool hovered) {
        std::vector<QVector3D> polyline;
        if (!m_modelPicker->getEdgePolyline(item.id.ownerId, item.id.elementId, polyline) ||
            polyline.size() < 2) {
            return;
        }
        painter.setPen(QPen(hovered ? style.edgeHover : style.edgeSelected, hovered ? 2.0 : 3.0));
        QPolygonF line;
        line.reserve(static_cast<int>(polyline.size()));
        for (const auto& point : polyline) {
            QPointF screenPos;
            if (!projectToScreen(viewProjection, point, static_cast<float>(m_width),
                                 static_cast<float>(m_height), &screenPos)) {
                continue;
            }
            line << screenPos;
        }
        if (line.size() >= 2) {
            painter.drawPolyline(line);
        }
    };

    auto drawVertex = [&](const app::selection::SelectionItem& item, bool hovered) {
        QVector3D vertex;
        if (!m_modelPicker->getVertexPosition(item.id.ownerId, item.id.elementId, vertex)) {
            return;
        }
        QPointF screenPos;
        if (!projectToScreen(viewProjection, vertex, static_cast<float>(m_width),
                             static_cast<float>(m_height), &screenPos)) {
            return;
        }
        painter.setPen(Qt::NoPen);
        painter.setBrush(hovered ? style.vertexHover : style.vertexSelected);
        const double radius = hovered ? 4.0 : 5.0;
        painter.drawEllipse(screenPos, radius, radius);
    };

    auto drawBody = [&](const app::selection::SelectionItem& item, bool hovered) {
        std::vector<std::array<QVector3D, 3>> triangles;
        if (!m_modelPicker->getBodyTriangles(item.id.ownerId, triangles)) {
            return;
        }
        const QColor fill = hovered ? style.faceFillHover : style.faceFillSelected;

        // Draw filled triangles WITHOUT outline (no internal mesh lines)
        painter.setPen(Qt::NoPen);
        painter.setBrush(fill);
        for (const auto& tri : triangles) {
            QPolygonF poly;
            bool projected = true;
            for (const auto& v : tri) {
                QPointF screenPos;
                if (!projectToScreen(viewProjection, v, static_cast<float>(m_width),
                                     static_cast<float>(m_height), &screenPos)) {
                    projected = false;
                    break;
                }
                poly << screenPos;
            }
            if (projected && poly.size() == 3) {
                painter.drawPolygon(poly);
            }
        }
        // Note: Body selection doesn't draw boundary edges - just fill
    };

    for (const auto& item : selection) {
        if (item.kind == app::selection::SelectionKind::Face) {
            drawFace(item, false);
        } else if (item.kind == app::selection::SelectionKind::Edge) {
            drawEdge(item, false);
        } else if (item.kind == app::selection::SelectionKind::Vertex) {
            drawVertex(item, false);
        } else if (item.kind == app::selection::SelectionKind::Body) {
            drawBody(item, false);
        }
    }

    if (hover.has_value()) {
        bool alreadySelected = std::any_of(selection.begin(), selection.end(),
                                           [&](const app::selection::SelectionItem& item) {
            return isSameItem(item, hover.value());
        });
        if (!alreadySelected) {
            if (hover->kind == app::selection::SelectionKind::Face) {
                drawFace(*hover, true);
            } else if (hover->kind == app::selection::SelectionKind::Edge) {
                drawEdge(*hover, true);
            } else if (hover->kind == app::selection::SelectionKind::Vertex) {
                drawVertex(*hover, true);
            } else if (hover->kind == app::selection::SelectionKind::Body) {
                drawBody(*hover, true);
            }
        }
    }
}

namespace {
struct IndicatorGeometry {
    QPainterPath path;
    QPointF startScreen;
    QPointF endScreen;
    QPolygonF head;
    QPolygonF backHead;
    QPointF labelPos;
    bool visible = false;
};

IndicatorGeometry calculateIndicatorGeometry(
    const ui::tools::ModelingTool::Indicator& indicator,
    const QMatrix4x4& viewProjection,
    float width,
    float height,
    double pixelScale)
{
    IndicatorGeometry geo;
    if (indicator.direction.lengthSquared() < 1e-6f) {
        return geo;
    }

    QVector3D dir = indicator.direction.normalized();

    const float visualLength = 30.0f; // 30 pixels per side (Compact)
    
    QPointF originScreen;
    if (!projectToScreen(viewProjection, indicator.origin, width, height, &originScreen)) {
        return geo;
    }
    
    // Project direction to screen to get 2D orientation
    QVector3D worldEnd = indicator.origin + dir * static_cast<float>(pixelScale * visualLength);
    QPointF endScreen;
    if (!projectToScreen(viewProjection, worldEnd, width, height, &endScreen)) {
        return geo;
    }

    QVector2D screenDir(endScreen - originScreen);
    if (screenDir.lengthSquared() < 1e-4f) {
        return geo;
    }
    screenDir.normalize();
    QVector2D perp(-screenDir.y(), screenDir.x());

    // Arrow Head parameters (Thick and distinct)
    const float headLength = 16.0f;
    const float headWidth = 10.0f;
    
    // Calculate endpoints
    geo.endScreen = originScreen + QPointF(screenDir.x() * visualLength, screenDir.y() * visualLength);
    
    // Front Head
    {
        QPointF headBase = geo.endScreen - QPointF(screenDir.x() * headLength, screenDir.y() * headLength);
        QPointF left = headBase + QPointF(perp.x() * headWidth, perp.y() * headWidth);
        QPointF right = headBase - QPointF(perp.x() * headWidth, perp.y() * headWidth);
        geo.head << geo.endScreen << left << right;
    }

    if (indicator.isDoubleSided) {
        geo.startScreen = originScreen - QPointF(screenDir.x() * visualLength, screenDir.y() * visualLength);
        
        // Back Head
        QPointF backHeadBase = geo.startScreen + QPointF(screenDir.x() * headLength, screenDir.y() * headLength);
        QPointF backLeft = backHeadBase + QPointF(perp.x() * headWidth, perp.y() * headWidth);
        QPointF backRight = backHeadBase - QPointF(perp.x() * headWidth, perp.y() * headWidth);
        geo.backHead << geo.startScreen << backLeft << backRight;
    } else {
        geo.startScreen = originScreen;
    }

    // Build the hit-test path
    // 1. Line segment
    QPainterPath linePath;
    linePath.moveTo(geo.startScreen);
    linePath.lineTo(geo.endScreen);
    
    // Use a stroker to make the line thick for hit testing
    QPainterPathStroker stroker;
    stroker.setWidth(60.0); // 60 pixel hit zone (Generous tolerance)
    stroker.setCapStyle(Qt::RoundCap);
    geo.path = stroker.createStroke(linePath);
    
    // 2. Add arrowheads to path
    geo.path.addPolygon(geo.head);
    if (!geo.backHead.isEmpty()) {
        geo.path.addPolygon(geo.backHead);
    }

    geo.labelPos = geo.endScreen + QPointF(perp.x() * 20.0f, perp.y() * 20.0f);
    geo.visible = true;
    return geo;
}
} // namespace

bool Viewport::isMouseOverIndicator(const QPoint& screenPos) const {
    if (!m_modelingToolManager) return false;
    auto indicator = m_modelingToolManager->activeIndicator();
    if (!indicator.has_value()) return false;

    QMatrix4x4 viewProjection = buildViewProjection();
    IndicatorGeometry geo = calculateIndicatorGeometry(*indicator, viewProjection, 
                                                     width(), height(), m_pixelScale);
    if (!geo.visible) return false;

    return geo.path.contains(QPointF(screenPos));
}

void Viewport::drawModelToolOverlay(const QMatrix4x4& viewProjection) {
    if (m_inSketchMode || !m_modelingToolManager) {
        return;
    }

    auto indicator = m_modelingToolManager->activeIndicator();
    if (!indicator.has_value()) {
        return;
    }

    IndicatorGeometry geo = calculateIndicatorGeometry(*indicator, viewProjection, 
                                                     width(), height(), m_pixelScale);
    if (!geo.visible) {
        return;
    }

    const ThemeViewportOverlayColors& overlay =
        ThemeManager::instance().currentTheme().viewport.overlay;
    QColor color = overlay.toolIndicator; // Normal state

    const bool isDragging = m_modelingToolManager->isDragging();
    
    if (isDragging) {
        // Dragging state: High contrast (White)
        color = Qt::white;
    } else if (m_indicatorHovered) {
        // Hover state: Brighter/Lighter
        color = color.lighter(130);
    }

    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing, true);

    // Draw White Border (Thickest)
    QPen borderPen(Qt::white, 8.0, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
    painter.setPen(borderPen);
    painter.setBrush(Qt::white);
    
    painter.drawLine(geo.startScreen, geo.endScreen);
    painter.drawPolygon(geo.head);
    if (!geo.backHead.isEmpty()) {
        painter.drawPolygon(geo.backHead);
    }

    // Draw Inner Color (Thick)
    QPen innerPen(color, 3.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
    painter.setPen(innerPen);
    painter.setBrush(color);
    
    painter.drawLine(geo.startScreen, geo.endScreen);
    painter.drawPolygon(geo.head);
    if (!geo.backHead.isEmpty()) {
        painter.drawPolygon(geo.backHead);
    }

    if (indicator->showDistance) {
        const double distanceValue = std::abs(indicator->distance);
        QString text = QString::number(distanceValue, 'f', 2);

        QFont font = painter.font();
        font.setPointSize(10);
        font.setBold(true);
        painter.setFont(font);

        QColor textColor = overlay.toolLabelText;
        QColor bgColor = overlay.toolLabelBackground;

        QFontMetrics metrics(font);
        const int padding = 4;
        QRectF labelRect(geo.labelPos.x(), geo.labelPos.y(),
                         metrics.horizontalAdvance(text) + padding * 2,
                         metrics.height() + padding * 2);

        painter.setPen(Qt::NoPen);
        painter.setBrush(bgColor);
        painter.drawRoundedRect(labelRect, 4.0, 4.0);

        painter.setPen(textColor);
        painter.drawText(labelRect, Qt::AlignCenter, text);
    }
}

void Viewport::drawPlaneSelectionOverlay(const QMatrix4x4& viewProjection) {
    if (!m_planeSelectionActive) {
        return;
    }

    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing, true);

    QFont labelFont = painter.font();
    labelFont.setBold(true);
    labelFont.setPointSize(labelFont.pointSize() + 1);
    painter.setFont(labelFont);

    const ThemeViewportPlaneColors& planeColors =
        ThemeManager::instance().currentTheme().viewport.planes;
    const QColor textColor = planeColors.labelText;
    const auto selections = planeSelections(planeColors);
    for (int i = 0; i < static_cast<int>(selections.size()); ++i) {
        const auto& selection = selections[i];
        PlaneAxes axes = buildPlaneAxes(selection.plane);
        QVector3D origin(selection.plane.origin.x, selection.plane.origin.y, selection.plane.origin.z);

        QVector<QVector3D> corners;
        corners.reserve(4);
        corners.append(origin + axes.xAxis * -kPlaneSelectHalf + axes.yAxis * -kPlaneSelectHalf);
        corners.append(origin + axes.xAxis * kPlaneSelectHalf + axes.yAxis * -kPlaneSelectHalf);
        corners.append(origin + axes.xAxis * kPlaneSelectHalf + axes.yAxis * kPlaneSelectHalf);
        corners.append(origin + axes.xAxis * -kPlaneSelectHalf + axes.yAxis * kPlaneSelectHalf);

        QPolygonF polygon;
        bool projectedAll = true;
        for (const auto& corner : corners) {
            QPointF screenPos;
            if (!projectToScreen(viewProjection, corner, static_cast<float>(m_width),
                                 static_cast<float>(m_height), &screenPos)) {
                projectedAll = false;
                break;
            }
            polygon << screenPos;
        }

        if (!projectedAll) {
            continue;
        }

        QColor fillColor = selection.color;
        QColor outlineColor = selection.color;
        outlineColor.setAlpha(200);

        bool hovered = (i == m_planeHoverIndex);
        if (hovered) {
            fillColor = fillColor.lighter(130);
            fillColor.setAlpha(140);
            outlineColor = outlineColor.lighter(150);
        }

        painter.setPen(QPen(outlineColor, hovered ? 2.5 : 1.5));
        painter.setBrush(fillColor);
        painter.drawPolygon(polygon);

        QPointF center;
        if (projectToScreen(viewProjection, origin, static_cast<float>(m_width),
                            static_cast<float>(m_height), &center)) {
            painter.setPen(textColor);
            QRectF labelRect(center.x() - 18, center.y() - 10, 36, 20);
            painter.drawText(labelRect, Qt::AlignCenter, selection.label);
        }
    }
}

void Viewport::activateLineTool() {
    setMoveSketchMode(false);
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Line);
    }
}

void Viewport::activateCircleTool() {
    setMoveSketchMode(false);
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Circle);
    }
}

void Viewport::activateRectangleTool() {
    setMoveSketchMode(false);
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Rectangle);
    }
}

void Viewport::activateArcTool() {
    setMoveSketchMode(false);
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Arc);
    }
}

void Viewport::activateEllipseTool() {
    setMoveSketchMode(false);
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Ellipse);
    }
}

void Viewport::activateTrimTool() {
    setMoveSketchMode(false);
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Trim);
    }
}

void Viewport::activateMirrorTool() {
    setMoveSketchMode(false);
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Mirror);
    }
}

void Viewport::deactivateTool() {
    if (m_toolManager) {
        m_toolManager->deactivateTool();
    }
}

void Viewport::setDocument(app::Document* document) {
    m_document = document;
    m_documentSketchesDirty = true;
    if (m_document && !m_referenceSketchId.empty()) {
        m_referenceSketch = m_document->getSketch(m_referenceSketchId);
    } else {
        m_referenceSketch = nullptr;
    }
    if (!m_inSketchMode && m_sketchRenderer) {
        m_sketchRenderer->setSketch(m_referenceSketch);
    }
    if (m_modelingToolManager) {
        m_modelingToolManager->setDocument(m_document);
    }

    // Connect to document signals to mark geometry dirty
    if (m_document) {
        connect(m_document, &app::Document::sketchAdded, this, [this]() {
            m_documentSketchesDirty = true;
            update();
        });
        connect(m_document, &app::Document::sketchRemoved, this, [this]() {
            if (!m_referenceSketchId.empty() &&
                m_document &&
                m_document->getSketch(m_referenceSketchId) == nullptr) {
                m_referenceSketchId.clear();
                m_referenceSketch = nullptr;
                updateModelSelectionFilter();
            }
            m_documentSketchesDirty = true;
            update();
        });
        connect(m_document, &app::Document::bodyAdded, this, [this]() {
            syncModelMeshes();
            update();
        });
        connect(m_document, &app::Document::bodyRemoved, this, [this]() {
            syncModelMeshes();
            update();
        });
        connect(m_document, &app::Document::bodyUpdated, this, [this]() {
            syncModelMeshes();
            update();
        });
        connect(m_document, &app::Document::bodyVisibilityChanged, this, [this]() {
            syncModelMeshes();
            update();
        });
        connect(m_document, &app::Document::isolationChanged, this, [this]() {
            syncModelMeshes();
            update();
        });
        connect(m_document, &app::Document::documentCleared, this, [this]() {
            if (m_inSketchMode) {
                exitSketchMode();
            }
            m_referenceSketchId.clear();
            m_referenceSketch = nullptr;
            if (m_sketchRenderer) {
                m_sketchRenderer->setSketch(nullptr);
            }
            m_documentSketchesDirty = true;
            clearModelPreviewMeshes();
            clearPreviewHiddenBody();
            updateModelSelectionFilter();
            syncModelMeshes();
            update();
        });
    }

    updateModelSelectionFilter();
    syncModelMeshes();
    update();
}

void Viewport::setCommandProcessor(app::commands::CommandProcessor* processor) {
    m_commandProcessor = processor;
    if (m_modelingToolManager) {
        m_modelingToolManager->setCommandProcessor(processor);
    }
}

void Viewport::setReferenceSketch(const QString& sketchId) {
    const std::string id = sketchId.toStdString();
    bool sketchBackfilled = false;
    if (m_document && !id.empty()) {
        sketchBackfilled = m_document->ensureHostFaceBoundariesProjected(id);
    }

    if (id == m_referenceSketchId && m_referenceSketch && !sketchBackfilled) {
        return;
    }
    m_referenceSketchId = id;
    if (m_document && !m_referenceSketchId.empty()) {
        m_referenceSketch = m_document->getSketch(m_referenceSketchId);
    } else {
        m_referenceSketch = nullptr;
    }
    if (!m_inSketchMode && m_sketchRenderer) {
        m_sketchRenderer->setSketch(m_referenceSketch);
    }
    m_documentSketchesDirty = true;
    updateModelSelectionFilter();
    update();
}

void Viewport::updateModelSelectionFilter() {
    if (!m_selectionManager || m_inSketchMode) {
        return;
    }

    app::selection::SelectionFilter filter;
    filter.allowedKinds = {
        app::selection::SelectionKind::Vertex,
        app::selection::SelectionKind::Edge,
        app::selection::SelectionKind::Face,
        app::selection::SelectionKind::Body
    };
    if (m_referenceSketch) {
        filter.allowedKinds.insert(app::selection::SelectionKind::SketchRegion);
        if (m_revolveToolActive) {
            filter.allowedKinds.insert(app::selection::SelectionKind::SketchEdge);
        }
    }
    m_selectionManager->setFilter(filter);
}

void Viewport::setModelPickMeshes(std::vector<selection::ModelPickerAdapter::Mesh>&& meshes) {
    if (m_modelPicker) {
        m_modelPicker->setMeshes(std::move(meshes));
    }
}

std::vector<app::selection::SelectionItem> Viewport::modelSelection() const {
    if (!m_selectionManager) {
        return {};
    }
    return m_selectionManager->selection();
}

void Viewport::setModelPreviewMeshes(const std::vector<render::SceneMeshStore::Mesh>& meshes) {
    if (m_bodyRenderer) {
        m_bodyRenderer->setPreviewMeshes(meshes);
        update();
    }
}

void Viewport::clearModelPreviewMeshes() {
    if (m_bodyRenderer) {
        m_bodyRenderer->clearPreview();
        update();
    }
    clearPreviewHiddenBody();
}

void Viewport::setPreviewHiddenBody(const std::string& bodyId) {
    if (m_previewHiddenBodyId == bodyId) {
        return;
    }
    m_previewHiddenBodyId = bodyId;
    syncModelMeshes();
    update();
}

void Viewport::clearPreviewHiddenBody() {
    if (m_previewHiddenBodyId.empty()) {
        return;
    }
    m_previewHiddenBodyId.clear();
    syncModelMeshes();
    update();
}

void Viewport::syncModelMeshes() {
    if (!m_document || !m_modelPicker) {
        return;
    }
    const auto& store = m_document->meshStore();

    // Build filtered list of visible body meshes
    std::vector<render::SceneMeshStore::Mesh> visibleMeshes;
    store.forEachMesh([&](const render::SceneMeshStore::Mesh& mesh) {
        if (m_document->isBodyVisible(mesh.bodyId) &&
            (m_previewHiddenBodyId.empty() || mesh.bodyId != m_previewHiddenBodyId)) {
            visibleMeshes.push_back(mesh);
        }
    });

    if (m_bodyRenderer) {
        m_bodyRenderer->setMeshes(visibleMeshes);
    }

    // Build pick meshes from visible bodies only
    std::vector<selection::ModelPickerAdapter::Mesh> pickMeshes;
    for (const auto& mesh : visibleMeshes) {
        selection::ModelPickerAdapter::Mesh pickMesh;
        pickMesh.bodyId = mesh.bodyId;
        pickMesh.vertices.reserve(mesh.vertices.size());
        for (const auto& v : mesh.vertices) {
            QVector4D transformed = mesh.modelMatrix * QVector4D(v, 1.0f);
            pickMesh.vertices.emplace_back(transformed.x(), transformed.y(), transformed.z());
        }
        pickMesh.triangles.reserve(mesh.triangles.size());
        for (const auto& tri : mesh.triangles) {
            selection::ModelPickerAdapter::Triangle pickTri;
            pickTri.i0 = tri.i0;
            pickTri.i1 = tri.i1;
            pickTri.i2 = tri.i2;
            pickTri.faceId = tri.faceId;
            pickMesh.triangles.push_back(pickTri);
        }
        for (const auto& [faceId, topo] : mesh.topologyByFace) {
            selection::ModelPickerAdapter::FaceTopology faceTopo;
            for (const auto& edge : topo.edges) {
                selection::ModelPickerAdapter::EdgePolyline edgeLine;
                edgeLine.edgeId = edge.edgeId;
                edgeLine.points.reserve(edge.points.size());
                for (const auto& pt : edge.points) {
                    QVector4D transformed = mesh.modelMatrix * QVector4D(pt, 1.0f);
                    edgeLine.points.emplace_back(transformed.x(), transformed.y(), transformed.z());
                }
                faceTopo.edges.push_back(std::move(edgeLine));
            }
            for (const auto& vertex : topo.vertices) {
                selection::ModelPickerAdapter::VertexSample sample;
                sample.vertexId = vertex.vertexId;
                QVector4D transformed = mesh.modelMatrix * QVector4D(vertex.position, 1.0f);
                sample.position = QVector3D(transformed.x(), transformed.y(), transformed.z());
                faceTopo.vertices.push_back(std::move(sample));
            }
            pickMesh.topologyByFace[faceId] = std::move(faceTopo);
        }
        pickMesh.faceGroupByFaceId = mesh.faceGroupByFaceId;
        pickMeshes.push_back(std::move(pickMesh));
    }
    setModelPickMeshes(std::move(pickMeshes));
}

void Viewport::notifySketchUpdated() {
    emit sketchUpdated();
}

void Viewport::setMoveSketchModeChangedCallback(std::function<void(bool)> callback) {
    m_moveSketchModeChangedCallback = std::move(callback);
}

void Viewport::setMoveSketchMode(bool active) {
    if (m_moveSketchModeActive == active) {
        return;
    }
    m_moveSketchModeActive = active;
    if (active) {
        setCursor(Qt::SizeAllCursor);
    } else {
        if (m_activeSketch) {
            m_activeSketch->endPointDrag();
        }
        clearPointDragSnapPreview();
        m_sketchInteractionState = SketchInteractionState::Idle;
        m_pointDragCandidateId.clear();
        m_moveSketchLastSketchPos = sketch::Vec2d{0.0, 0.0};
        setCursor(Qt::ArrowCursor);
    }
    if (m_moveSketchModeChangedCallback) {
        m_moveSketchModeChangedCallback(m_moveSketchModeActive);
    }
    update();
}

void Viewport::updateSnapSettings(const SnapSettingsPanel::SnapSettings& settings) {
    if (!m_toolManager) return;

    auto& sm = m_toolManager->snapManager();
    sm.setGridSnapEnabled(settings.grid);
    sm.setSnapEnabled(core::sketch::SnapType::SketchGuide, settings.sketchGuideLines);
    // sm.setSnapEnabled(core::sketch::SnapType::Vertex, settings.sketchGuidePoints); // Vertex is fundamental, maybe dont disable?
    // User requested "Sketch Guide Points" toggle. Let's assume it means Vertex/Endpoint/Midpoint etc?
    // Or just "Points" (Vertex).
    // Let's toggle Point-like snaps for "Sketch Guide Points"
    sm.setSnapEnabled(core::sketch::SnapType::Vertex, settings.sketchGuidePoints);
    sm.setSnapEnabled(core::sketch::SnapType::Endpoint, settings.sketchGuidePoints);
    sm.setSnapEnabled(core::sketch::SnapType::Midpoint, settings.sketchGuidePoints);
    sm.setSnapEnabled(core::sketch::SnapType::Center, settings.sketchGuidePoints);
    sm.setSnapEnabled(core::sketch::SnapType::Quadrant, settings.sketchGuidePoints);
    // Intersection?
    sm.setSnapEnabled(core::sketch::SnapType::Intersection, settings.sketchGuidePoints);

    sm.setSnapEnabled(core::sketch::SnapType::ActiveLayer3D, settings.activeLayer3DPoints || settings.activeLayer3DEdges);
    // Note: SnapManager distinguishes points vs edges in findExternalSnaps but flag is one.
    // For now we pass both data if enable.
    
    sm.setShowGuidePoints(settings.showGuidePoints);
    sm.setShowSnappingHints(settings.showSnappingHints);
    
    update();
}

void Viewport::updateSnapGeometry() {
    if (!m_inSketchMode || !m_activeSketch || !m_toolManager || !m_document) return;

    std::vector<core::sketch::Vec2d> points;
    std::vector<std::pair<core::sketch::Vec2d, core::sketch::Vec2d>> lines;

    const auto& plane = m_activeSketch->getPlane();

    // Iterate over all bodies in the document
    auto bodyIds = m_document->getBodyIds();
    for (const auto& bodyId : bodyIds) {
        const auto* body = m_document->getBodyShape(bodyId);
        if (!body || !m_document->isBodyVisible(bodyId)) continue;
        
        // Extract Vertices
        TopExp_Explorer exV;
        for (exV.Init(*body, TopAbs_VERTEX); exV.More(); exV.Next()) {
            const TopoDS_Vertex& v = TopoDS::Vertex(exV.Current());
            gp_Pnt p = BRep_Tool::Pnt(v);
            core::sketch::Vec3d p3d{p.X(), p.Y(), p.Z()};
            points.push_back(plane.toSketch(p3d));
        }

        // Extract Edges (Linear only for now, or discretized)
        // For simplicity, snapping to "Distant Edges" usually means snapping to the projected line of an edge.
        // We will take endpoints of edges for now as lines, or sample them.
        // Implementing full curve projection is complex (Curve-Plane intersection or projection).
        // Let's stick to Vertices for this iteration as "Guide Points" is the main request.
        // "Distant Edges" - maybe just linear edges?
        TopExp_Explorer exE;
        for (exE.Init(*body, TopAbs_EDGE); exE.More(); exE.Next()) {
             const TopoDS_Edge& e = TopoDS::Edge(exE.Current());
             // Check if linear... BRepAdaptor_Curve
             // For now, simpler: get vertices of edge
             // This is redundant with vertex iteration above unless we want lines.
        }
    }
    
    // Pass to SnapManager
    m_toolManager->snapManager().setExternalGeometry(points, lines);
}

} // namespace ui
} // namespace onecad
