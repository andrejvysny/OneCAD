#include "Viewport.h"
#include "../../render/Camera3D.h"
#include "../../render/Grid3D.h"
#include "../../core/sketch/SketchRenderer.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/tools/SketchToolManager.h"
#include "../viewcube/ViewCube.h"
#include "../theme/ThemeManager.h"

#include <QKeyEvent>

#include <QMouseEvent>
#include <QWheelEvent>
#include <QGestureEvent>
#include <QPinchGesture>
#include <QPanGesture>
#include <QNativeGestureEvent>
#include <QApplication>
#include <QOpenGLContext>
#include <QSizePolicy>
#include <QtMath>
#include <iostream>

namespace onecad {
namespace ui {

namespace sketch = core::sketch;
namespace tools = core::sketch::tools;

namespace {
constexpr float kOrbitSensitivity = 0.3f;
constexpr float kTrackpadPanScale = 1.0f;
constexpr float kTrackpadOrbitScale = 0.35f;
constexpr float kPinchZoomScale = 1000.0f;
constexpr float kWheelZoomShiftScale = 0.2f;
constexpr float kAngleDeltaToPixels = 1.0f / 8.0f;
constexpr qint64 kNativeZoomPanSuppressMs = 120;
} // namespace

Viewport::Viewport(QWidget* parent)
    : QOpenGLWidget(parent) {

    m_camera = std::make_unique<render::Camera3D>();
    m_grid = std::make_unique<render::Grid3D>();
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

    // Theme integration
    m_themeConnection = connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
                                this, &Viewport::updateTheme, Qt::UniqueConnection);
    updateTheme();
    // Note: QSurfaceFormat is now set globally in main.cpp
    // This ensures the format is applied BEFORE context creation
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
    std::cout << "OpenGL Version: " << (version ? version : "unknown") << std::endl;
    std::cout << "OpenGL Renderer: " << (renderer ? renderer : "unknown") << std::endl;
    
    // Background color set via updateTheme
    glClearColor(m_backgroundColor.redF(), m_backgroundColor.greenF(), m_backgroundColor.blueF(), m_backgroundColor.alphaF());
    
    glEnable(GL_DEPTH_TEST);
    glEnable(GL_MULTISAMPLE);
    glEnable(GL_LINE_SMOOTH);
    glHint(GL_LINE_SMOOTH_HINT, GL_NICEST);
    
    // Disable states we don't want by default
    glDisable(GL_CULL_FACE);
    
    m_grid->initialize();

    // Create and initialize sketch renderer (requires OpenGL context)
    m_sketchRenderer = std::make_unique<sketch::SketchRenderer>();
    if (!m_sketchRenderer->initialize()) {
        std::cerr << "Failed to initialize SketchRenderer" << std::endl;
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
    
    // Render grid
    m_grid->render(viewProjection, m_camera->distance(), m_camera->position());

    // Render sketch if in sketch mode
    if (m_inSketchMode && m_activeSketch && m_sketchRenderer) {
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
    }
}

void Viewport::mousePressEvent(QMouseEvent* event) {
    m_lastMousePos = event->pos();

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

    if (event->button() == Qt::RightButton) {
        m_isOrbiting = true;
        setCursor(Qt::ClosedHandCursor);
    } else if (event->button() == Qt::MiddleButton) {
        m_isPanning = true;
        setCursor(Qt::SizeAllCursor);
    }

    QOpenGLWidget::mousePressEvent(event);
}

void Viewport::mouseMoveEvent(QMouseEvent* event) {
    QPoint delta = event->pos() - m_lastMousePos;
    m_lastMousePos = event->pos();

    // Forward to sketch tool if active
    if (m_inSketchMode && m_toolManager && m_toolManager->hasActiveTool()) {
        sketch::Vec2d sketchPos = screenToSketch(event->pos());
        m_toolManager->handleMouseMove(sketchPos);
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

    if (event->button() == Qt::RightButton) {
        m_isOrbiting = false;
    } else if (event->button() == Qt::MiddleButton) {
        m_isPanning = false;
    }

    setCursor(Qt::ArrowCursor);
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

// Sketch mode
void Viewport::enterSketchMode(sketch::Sketch* sketch) {
    if (m_inSketchMode || !sketch) return;

    m_activeSketch = sketch;
    m_inSketchMode = true;

    // Store current camera state
    m_savedCameraPosition = m_camera->position();
    m_savedCameraTarget = m_camera->target();
    m_savedCameraUp = m_camera->up();
    m_savedCameraAngle = m_camera->cameraAngle();

    // Align camera to sketch plane and switch to orthographic
    const auto& plane = sketch->getPlane();
    QVector3D normal(plane.normal.x, plane.normal.y, plane.normal.z);
    QVector3D xAxis(plane.xAxis.x, plane.xAxis.y, plane.xAxis.z);
    QVector3D yAxis(plane.yAxis.x, plane.yAxis.y, plane.yAxis.z);

    if (normal.lengthSquared() < 1e-8f) {
        normal = QVector3D::crossProduct(xAxis, yAxis);
    }
    if (normal.lengthSquared() < 1e-8f) {
        normal = QVector3D(0.0f, 0.0f, 1.0f);
    }
    normal.normalize();

    if (xAxis.lengthSquared() < 1e-8f) {
        xAxis = QVector3D::crossProduct(yAxis, normal);
    }
    if (xAxis.lengthSquared() < 1e-8f) {
        xAxis = (std::abs(normal.z()) < 0.9f) ? QVector3D::crossProduct(normal, QVector3D(0, 0, 1))
                                              : QVector3D::crossProduct(normal, QVector3D(0, 1, 0));
    }
    xAxis -= normal * QVector3D::dotProduct(normal, xAxis);
    if (xAxis.lengthSquared() < 1e-8f) {
        xAxis = (std::abs(normal.z()) < 0.9f) ? QVector3D::crossProduct(normal, QVector3D(0, 0, 1))
                                              : QVector3D::crossProduct(normal, QVector3D(0, 1, 0));
    }
    xAxis.normalize();

    QVector3D up = QVector3D::crossProduct(normal, xAxis).normalized();

    QVector3D target(plane.origin.x, plane.origin.y, plane.origin.z);
    float dist = m_camera->distance();
    m_camera->setTarget(target);
    m_camera->setUp(up);
    m_camera->setPosition(target - normal * dist);
    m_camera->setCameraAngle(0.0f);  // 0° = orthographic

    // Bind sketch to renderer
    if (m_sketchRenderer) {
        m_sketchRenderer->setSketch(sketch);
        m_sketchRenderer->updateGeometry();
    }

    // Initialize tool manager
    m_toolManager = std::make_unique<tools::SketchToolManager>(this);
    m_toolManager->setSketch(sketch);
    m_toolManager->setRenderer(m_sketchRenderer.get());

    // Connect tool signals
    connect(m_toolManager.get(), &tools::SketchToolManager::geometryCreated, this, [this]() {
        if (m_sketchRenderer) {
            m_sketchRenderer->updateGeometry();
        }
        update();
    });
    connect(m_toolManager.get(), &tools::SketchToolManager::updateRequested, this, [this]() {
        update();
    });

    update();
    emit sketchModeChanged(true);
}

void Viewport::exitSketchMode() {
    if (!m_inSketchMode) return;

    m_inSketchMode = false;
    m_activeSketch = nullptr;

    // Clean up tool manager
    if (m_toolManager) {
        m_toolManager->deactivateTool();
        m_toolManager.reset();
    }

    // Unbind sketch from renderer
    if (m_sketchRenderer) {
        m_sketchRenderer->setSketch(nullptr);
    }

    // Restore camera to previous state
    m_camera->setPosition(m_savedCameraPosition);
    m_camera->setTarget(m_savedCameraTarget);
    m_camera->setUp(m_savedCameraUp);
    m_camera->setCameraAngle(m_savedCameraAngle);

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
    // Forward to sketch tool if active
    if (m_inSketchMode && m_toolManager && m_toolManager->hasActiveTool()) {
        m_toolManager->handleKeyPress(static_cast<Qt::Key>(event->key()));
        event->accept();
        return;
    }

    QOpenGLWidget::keyPressEvent(event);
}

// Tool management
tools::SketchToolManager* Viewport::toolManager() const {
    return m_toolManager.get();
}

sketch::Vec2d Viewport::screenToSketch(const QPoint& screenPos) const {
    // Convert screen coordinates to sketch plane coordinates
    // This involves unprojecting from screen space through the camera
    // to the sketch plane

    if (!m_activeSketch || !m_camera) {
        return {0.0, 0.0};
    }

    const qreal ratio = devicePixelRatio();
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

    // Get sketch plane
    const auto& plane = m_activeSketch->getPlane();
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

void Viewport::activateLineTool() {
    if (m_toolManager) {
        m_toolManager->activateTool(tools::ToolType::Line);
    }
}

void Viewport::activateCircleTool() {
    if (m_toolManager) {
        m_toolManager->activateTool(tools::ToolType::Circle);
    }
}

void Viewport::activateRectangleTool() {
    if (m_toolManager) {
        m_toolManager->activateTool(tools::ToolType::Rectangle);
    }
}

void Viewport::deactivateTool() {
    if (m_toolManager) {
        m_toolManager->deactivateTool();
    }
}

} // namespace ui
} // namespace onecad
