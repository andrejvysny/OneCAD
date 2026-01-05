#include "Viewport.h"
#include "../../render/BodyRenderer.h"
#include "../../render/Camera3D.h"
#include "../../render/Grid3D.h"
#include "../../core/sketch/SketchRenderer.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/SketchConstraint.h"
#include "../../core/sketch/constraints/Constraints.h"
#include "../../core/sketch/tools/SketchToolManager.h"
#include "../../app/document/Document.h"
#include "../../app/selection/SelectionManager.h"
#include "../../app/selection/SelectionTypes.h"
#include "../viewcube/ViewCube.h"
#include "../theme/ThemeManager.h"
#include "../sketch/DimensionEditor.h"
#include "../selection/DeepSelectPopup.h"
#include "../selection/SketchPickerAdapter.h"
#include "../selection/ModelPickerAdapter.h"
#include "../tools/ModelingToolManager.h"

#include <QKeyEvent>
#include <QPainter>
#include <QPolygonF>
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
#include <QEasingCurve>
#include <QtMath>
#include <QDebug>
#include <array>
#include <limits>
#include <optional>

namespace onecad {
namespace ui {

namespace sketch = core::sketch;
namespace sketchTools = core::sketch::tools;

namespace {
constexpr float kOrbitSensitivity = 0.3f;
constexpr float kTrackpadPanScale = 1.0f;
constexpr float kTrackpadOrbitScale = 0.35f;
constexpr float kPinchZoomScale = 1000.0f;
constexpr float kWheelZoomShiftScale = 0.2f;
constexpr float kAngleDeltaToPixels = 1.0f / 8.0f;
constexpr qint64 kNativeZoomPanSuppressMs = 120;
constexpr float kPlaneSelectSize = 120.0f;
constexpr float kPlaneSelectHalf = kPlaneSelectSize * 0.5f;
} // namespace

namespace {
struct PlaneSelectionVisual {
    core::sketch::SketchPlane plane;
    QString label;
    QColor color;
};

const std::array<PlaneSelectionVisual, 3>& planeSelections() {
    static const std::array<PlaneSelectionVisual, 3> selections = {{
        {core::sketch::SketchPlane::XY(), QStringLiteral("XY"), QColor(80, 160, 255, 90)},
        {core::sketch::SketchPlane::XZ(), QStringLiteral("XZ"), QColor(120, 200, 140, 90)},
        {core::sketch::SketchPlane::YZ(), QStringLiteral("YZ"), QColor(255, 170, 90, 90)}
    }};
    return selections;
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
        m_selectionManager->applySelectionCandidate(
            m_pendingCandidates[static_cast<size_t>(index)],
            m_pendingModifiers,
            m_pendingClickPos
        );
        update();
    });
    connect(m_deepSelectPopup, &selection::DeepSelectPopup::popupClosed,
            this, [this]() {
        m_pendingCandidates.clear();
        m_pendingClickPos = QPoint();
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
}

void Viewport::updateTheme() {
    if (ThemeManager::instance().isDark()) {
        m_backgroundColor = QColor(45, 45, 48); // #2d2d30
        if (m_grid) {
            m_grid->setMajorColor(QColor(80, 80, 80));
            m_grid->setMinorColor(QColor(50, 50, 50));
            m_grid->forceUpdate();
        }
    } else {
        m_backgroundColor = QColor(243, 243, 243); // #f3f3f3
        if (m_grid) {
            m_grid->setMajorColor(QColor(200, 200, 200));
            m_grid->setMinorColor(QColor(225, 225, 225));
            m_grid->forceUpdate();
        }
    }
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
    m_grid->render(viewProjection,
                   static_cast<float>(pixelScale),
                   static_cast<float>(viewExtent));

    if (m_bodyRenderer) {
        render::BodyRenderer::RenderStyle style;
        if (ThemeManager::instance().isDark()) {
            style.baseColor = QColor(160, 160, 160);
            style.edgeColor = QColor(210, 210, 210);
        } else {
            style.baseColor = QColor(140, 140, 140);
            style.edgeColor = QColor(80, 80, 80);
        }
        style.drawEdges = true;
        style.previewAlpha = 0.35f;
        if (m_inSketchMode) {
            style.ghosted = true;
            style.ghostFactor = 0.6f;
            style.baseAlpha = 0.25f;
            style.edgeAlpha = 0.5f;
        }

        QVector3D lightDir = -m_camera->forward();
        if (lightDir.lengthSquared() < 1e-6f) {
            lightDir = QVector3D(0.0f, 0.0f, 1.0f);
        } else {
            lightDir.normalize();
        }
        m_bodyRenderer->render(viewProjection, lightDir, style);
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

        // Render tool preview
        if (m_toolManager) {
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
            
            // Setup font/pen based on theme
            bool isDark = ThemeManager::instance().isDark();
            QColor textColor = isDark ? Qt::white : Qt::black;
            QColor bgColor = isDark ? QColor(0, 0, 0, 180) : QColor(255, 255, 255, 180);
            
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
}

void Viewport::mousePressEvent(QMouseEvent* event) {
    m_lastMousePos = event->pos();

    if (event->button() == Qt::LeftButton && m_deepSelectPopup &&
        m_deepSelectPopup->isVisible()) {
        m_deepSelectPopup->hide();
        m_pendingCandidates.clear();
        m_pendingClickPos = QPoint();
        m_pendingModifiers = {};
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
            return;
        }

        update();
        return;
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

            bool allowTool = false;
            if (!modifiers.shift && !modifiers.toggle &&
                m_modelingToolManager && m_modelingToolManager->hasActiveTool()) {
                auto topCandidate = m_selectionManager->topCandidate(pickResult);
                const auto& selection = m_selectionManager->selection();
                if (topCandidate.has_value() && selection.size() == 1) {
                    app::selection::SelectionKey topKey{topCandidate->kind, topCandidate->id};
                    app::selection::SelectionKey selKey{selection.front().kind, selection.front().id};
                    if (topKey == selKey) {
                        allowTool = true;
                    }
                }
            }

            if (allowTool && m_modelingToolManager->handleMousePress(event->pos(), event->button())) {
                event->accept();
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
                return;
            }

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

    QOpenGLWidget::mouseDoubleClickEvent(event);
}

void Viewport::mouseMoveEvent(QMouseEvent* event) {
    QPoint delta = event->pos() - m_lastMousePos;
    m_lastMousePos = event->pos();

    if (!m_inSketchMode && m_modelingToolManager && m_modelingToolManager->isDragging() &&
        (event->buttons() & Qt::LeftButton)) {
        if (m_modelingToolManager->handleMouseMove(event->pos())) {
            update();
            return;
        }
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
        auto pickResult = buildSketchPickResult(event->pos());
        m_selectionManager->updateHover(pickResult);
        update();
    }

    if (!m_inSketchMode && !m_planeSelectionActive && !m_isOrbiting && !m_isPanning &&
        (!m_deepSelectPopup || !m_deepSelectPopup->isVisible())) {
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
    // Forward to sketch tool if active
    if (m_inSketchMode && m_toolManager && m_toolManager->hasActiveTool()) {
        sketch::Vec2d sketchPos = screenToSketch(event->pos());
        m_toolManager->handleMouseRelease(sketchPos, event->button());
    }

    if (!m_inSketchMode && m_modelingToolManager && m_modelingToolManager->isDragging()) {
        if (m_modelingToolManager->handleMouseRelease(event->pos(), event->button())) {
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
    if (m_deepSelectPopup && m_deepSelectPopup->isVisible()) {
        m_deepSelectPopup->hide();
    }
    m_pendingCandidates.clear();
    m_pendingClickPos = QPoint();
    m_pendingModifiers = {};
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

void Viewport::handlePan(float dx, float dy) {
    m_camera->pan(dx, dy);
    update();
    emit cameraChanged();
}

void Viewport::handleOrbit(float dx, float dy) {
    // Sensitivity adjustment
    m_camera->orbit(-dx * kOrbitSensitivity, dy * kOrbitSensitivity);
    update();
    emit cameraChanged();
}

void Viewport::handleZoom(float delta) {
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

    update();
    emit sketchModeChanged(true);
}

void Viewport::exitSketchMode() {
    if (!m_inSketchMode) return;

    m_inSketchMode = false;
    m_activeSketch = nullptr;
    m_activeSketchId.clear();
    if (m_selectionManager) {
        m_selectionManager->setMode(app::selection::SelectionMode::Model);
        updateModelSelectionFilter();
    }

    // Clean up tool manager
    if (m_toolManager) {
        m_toolManager->deactivateTool();
        m_toolManager.reset();
    }

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

void Viewport::keyPressEvent(QKeyEvent* event) {
    if (m_planeSelectionActive && !m_inSketchMode && event->key() == Qt::Key_Escape) {
        cancelPlaneSelection();
        event->accept();
        return;
    }

    // Forward to sketch tool if active
    if (m_inSketchMode && m_toolManager && m_toolManager->hasActiveTool()) {
        m_toolManager->handleKeyPress(static_cast<Qt::Key>(event->key()));
        event->accept();
        return;
    }

    if (!m_inSketchMode && m_modelingToolManager &&
        m_modelingToolManager->hasActiveTool() &&
        event->key() == Qt::Key_Escape) {
        m_modelingToolManager->cancelActiveTool();
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
    if (selection.size() == 1 &&
        selection.front().kind == app::selection::SelectionKind::SketchRegion &&
        m_referenceSketch) {
        m_modelingToolManager->activateExtrude(selection.front());
    } else {
        m_modelingToolManager->cancelActiveTool();
    }
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
    const auto& selections = planeSelections();

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
    if (ThemeManager::instance().isDark()) {
        style.faceFillHover = QColor(96, 96, 255, 64);
        style.faceOutlineHover = QColor(96, 96, 255, 160);
        style.faceFillSelected = QColor(64, 128, 255, 90);
        style.faceOutlineSelected = QColor(64, 128, 255, 200);
    } else {
        style.faceFillHover = QColor(128, 128, 255, 64);
        style.faceOutlineHover = QColor(128, 128, 255, 160);
        style.faceFillSelected = QColor(32, 96, 255, 90);
        style.faceOutlineSelected = QColor(32, 96, 255, 200);
    }
    style.edgeHover = style.faceOutlineHover;
    style.edgeSelected = style.faceOutlineSelected;
    style.vertexHover = style.faceOutlineHover;
    style.vertexSelected = style.faceOutlineSelected;

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
        painter.setBrush(fill);
        painter.setPen(QPen(outline, hovered ? 1.5 : 2.0));
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
        const QColor outline = hovered ? style.faceOutlineHover : style.faceOutlineSelected;
        painter.setBrush(fill);
        painter.setPen(QPen(outline, hovered ? 1.2 : 1.8));
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

    const QColor textColor = ThemeManager::instance().isDark()
        ? QColor(230, 230, 230)
        : QColor(30, 30, 30);

    const auto& selections = planeSelections();
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
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Line);
    }
}

void Viewport::activateCircleTool() {
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Circle);
    }
}

void Viewport::activateRectangleTool() {
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Rectangle);
    }
}

void Viewport::activateArcTool() {
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Arc);
    }
}

void Viewport::activateEllipseTool() {
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Ellipse);
    }
}

void Viewport::activateTrimTool() {
    if (m_toolManager) {
        m_toolManager->activateTool(sketchTools::ToolType::Trim);
    }
}

void Viewport::activateMirrorTool() {
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
    if (id == m_referenceSketchId && m_referenceSketch) {
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
    }
    m_selectionManager->setFilter(filter);
}

void Viewport::setModelPickMeshes(std::vector<selection::ModelPickerAdapter::Mesh>&& meshes) {
    if (m_modelPicker) {
        m_modelPicker->setMeshes(std::move(meshes));
    }
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
}

void Viewport::syncModelMeshes() {
    if (!m_document || !m_modelPicker) {
        return;
    }
    const auto& store = m_document->meshStore();
    if (m_bodyRenderer) {
        m_bodyRenderer->setMeshes(store);
    }

    std::vector<selection::ModelPickerAdapter::Mesh> pickMeshes;
    store.forEachMesh([&](const render::SceneMeshStore::Mesh& mesh) {
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
        pickMeshes.push_back(std::move(pickMesh));
    });
    setModelPickMeshes(std::move(pickMeshes));
}

void Viewport::notifySketchUpdated() {
    emit sketchUpdated();
}

} // namespace ui
} // namespace onecad
