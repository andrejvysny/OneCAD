#include "Camera3D.h"
#include <QtMath>
#include <cmath>

namespace onecad {
namespace render {

Camera3D::Camera3D() {
    reset();

    // Initialize ortho scale based on initial perspective state
    // This ensures first transition to ortho preserves current view scale
    float halfFovRad = qDegreesToRadians(m_fov * 0.5f);
    m_orthoScale = 2.0f * distance() * qTan(halfFovRad);
}

void Camera3D::setPosition(const QVector3D& pos) {
    m_position = pos;
}

void Camera3D::setTarget(const QVector3D& target) {
    m_target = target;
}

void Camera3D::setUp(const QVector3D& up) {
    m_up = up.normalized();
}

QVector3D Camera3D::forward() const {
    return (m_target - m_position).normalized();
}

QVector3D Camera3D::right() const {
    return QVector3D::crossProduct(forward(), m_up).normalized();
}

float Camera3D::distance() const {
    return (m_target - m_position).length();
}

void Camera3D::orbit(float deltaYaw, float deltaPitch) {
    // Get current offset from target
    QVector3D offset = m_position - m_target;
    float dist = offset.length();
    
    // Convert to spherical coordinates
    float theta = qAtan2(offset.x(), offset.y()); // Azimuth (yaw)
    float phi = qAsin(qBound(-1.0f, offset.z() / dist, 1.0f)); // Elevation (pitch)
    
    // Apply rotation (convert degrees to radians)
    theta -= qDegreesToRadians(deltaYaw);
    phi += qDegreesToRadians(deltaPitch);
    
    // Clamp pitch to avoid gimbal lock
    phi = qBound(qDegreesToRadians(MIN_PITCH), phi, qDegreesToRadians(MAX_PITCH));
    
    // Convert back to Cartesian
    float cosPhi = qCos(phi);
    offset.setX(dist * cosPhi * qSin(theta));
    offset.setY(dist * cosPhi * qCos(theta));
    offset.setZ(dist * qSin(phi));
    
    m_position = m_target + offset;
}

void Camera3D::pan(float deltaX, float deltaY) {
    // Scale by distance for consistent feel
    float scale = distance() * 0.001f;
    if (m_projectionType == ProjectionType::Orthographic) {
        scale = m_orthoScale * 0.001f;
    }
    
    QVector3D rightVec = right();
    QVector3D upVec = m_up;
    
    QVector3D offset = rightVec * (-deltaX * scale) + upVec * (deltaY * scale);
    
    m_position += offset;
    m_target += offset;
}

void Camera3D::zoom(float delta) {
    zoomByFactor(1.0f - delta * 0.001f);
}

void Camera3D::zoomByFactor(float factor) {
    if (!std::isfinite(factor)) {
        return;
    }

    factor = qBound(0.01f, factor, 100.0f);

    if (m_projectionType == ProjectionType::Orthographic) {
        float newScale = m_orthoScale * factor;
        float minScale = 2.0f * MIN_DISTANCE *
            qTan(qDegreesToRadians(MIN_PERSPECTIVE_FOV * 0.5f));
        float maxScale = 2.0f * MAX_DISTANCE *
            qTan(qDegreesToRadians(MAX_PERSPECTIVE_FOV * 0.5f));
        m_orthoScale = qBound(minScale, newScale, maxScale);
        return;
    }

    float dist = distance();
    float newDist = dist * factor;

    // Clamp distance
    newDist = qBound(MIN_DISTANCE, newDist, MAX_DISTANCE);

    // Move camera along view direction
    QVector3D dir = forward();
    m_position = m_target - dir * newDist;
}

void Camera3D::reset() {
    setIsometricView();
}

void Camera3D::setFrontView() {
    // User Front = User -Y = Geom +X
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(dist, 0, 0);
    m_up = QVector3D(0, 0, 1);
}

void Camera3D::setBackView() {
    // User Back = User +Y = Geom -X
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(-dist, 0, 0);
    m_up = QVector3D(0, 0, 1);
}

void Camera3D::setLeftView() {
    // User Left = User -X = Geom -Y
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(0, -dist, 0);
    m_up = QVector3D(0, 0, 1);
}

void Camera3D::setRightView() {
    // User Right = User +X = Geom +Y
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(0, dist, 0);
    m_up = QVector3D(0, 0, 1);
}

void Camera3D::setTopView() {
    // User Top = +Z view
    // Screen right (User +X) maps to Geom +Y
    // Screen up (User +Y) maps to Geom -X
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(0, 0, dist);
    m_up = QVector3D(-1, 0, 0);
}

void Camera3D::setBottomView() {
    // User Bottom = -Z
    // Up Vector should be User -Y (Top of screen) -> Geom +X
    // (To mirror Top view flip)
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(0, 0, -dist);
    m_up = QVector3D(1, 0, 0);
}

void Camera3D::setIsometricView() {
    m_target = QVector3D(0, 0, 0);
    float dist = 500.0f; // Default viewing distance in mm
    float angle = qDegreesToRadians(45.0f);
    float elevation = qDegreesToRadians(35.264f); // arctan(1/sqrt(2))
    
    m_position = QVector3D(
        dist * qCos(elevation) * qSin(angle),
        dist * qCos(elevation) * qCos(angle),
        dist * qSin(elevation)
    );
    m_up = QVector3D(0, 0, 1);
}

QMatrix4x4 Camera3D::viewMatrix() const {
    QMatrix4x4 view;
    view.lookAt(m_position, m_target, m_up);
    return view;
}

void Camera3D::setCameraAngle(float degrees) {
    // Clamp to valid range [0, 120]
    degrees = qBound(0.0f, degrees, MAX_PERSPECTIVE_FOV);

    // Determine target projection type
    ProjectionType targetProjection = (degrees < 0.01f) ?
        ProjectionType::Orthographic :
        ProjectionType::Perspective;

    // Preserve apparent scale when changing projection or FOV
    if (targetProjection != m_projectionType) {
        // === PROJECTION TYPE CHANGE ===
        float currentDistance = distance();

        if (targetProjection == ProjectionType::Perspective) {
            // Ortho → Perspective transition
            // Compute new distance to preserve scale at matching plane
            // Formula: D_new = S_ortho / (2 * tan(θ/2))
            float targetFov = qMax(degrees, MIN_PERSPECTIVE_FOV);
            float halfFovRad = qDegreesToRadians(targetFov * 0.5f);
            float newDistance = m_orthoScale * 0.5f / qTan(halfFovRad);

            // Clamp to safe range
            newDistance = qBound(MIN_DISTANCE, newDistance, MAX_DISTANCE);

            // Move camera to new distance while keeping target fixed
            QVector3D dir = forward();
            m_position = m_target - dir * newDistance;

            m_fov = targetFov;
        } else {
            // Perspective → Ortho transition
            // Compute new ortho scale to preserve apparent size
            // Formula: S_new = 2 * D_curr * tan(θ/2)
            float halfFovRad = qDegreesToRadians(m_fov * 0.5f);
            m_orthoScale = 2.0f * currentDistance * qTan(halfFovRad);
        }

        m_projectionType = targetProjection;
    } else if (m_projectionType == ProjectionType::Perspective) {
        // === WITHIN PERSPECTIVE MODE: FOV CHANGE ===
        // CRITICAL: Must adjust distance to preserve apparent size!
        // Formula: D₂ = D₁ * tan(θ₁/2) / tan(θ₂/2)
        // This keeps H_world constant: 2*D*tan(θ/2) = constant

        float targetFov = qMax(degrees, MIN_PERSPECTIVE_FOV);

        if (qAbs(targetFov - m_fov) > 0.01f) {  // Only adjust if FOV actually changed
            float currentDistance = distance();
            float oldHalfFovRad = qDegreesToRadians(m_fov * 0.5f);
            float newHalfFovRad = qDegreesToRadians(targetFov * 0.5f);

            // Compute new distance to keep same visible world height
            float newDistance = currentDistance * qTan(oldHalfFovRad) / qTan(newHalfFovRad);

            // Clamp to safe range
            newDistance = qBound(MIN_DISTANCE, newDistance, MAX_DISTANCE);

            // Move camera along view direction
            QVector3D dir = forward();
            m_position = m_target - dir * newDistance;
        }

        m_fov = targetFov;
    }
    // Note: Ortho mode has no FOV, so no action needed when staying in ortho

    m_cameraAngle = degrees;
}

QMatrix4x4 Camera3D::projectionMatrix(float aspectRatio) const {
    QMatrix4x4 projection;

    if (m_projectionType == ProjectionType::Orthographic) {
        // Orthographic projection
        // Scale defines world units per viewport height
        float halfHeight = m_orthoScale * 0.5f;
        float halfWidth = halfHeight * aspectRatio;

        projection.ortho(-halfWidth, halfWidth,
                         -halfHeight, halfHeight,
                         m_nearPlane, m_farPlane);
    } else {
        // Perspective projection
        projection.perspective(m_fov, aspectRatio, m_nearPlane, m_farPlane);
    }

    return projection;
}

} // namespace render
} // namespace onecad
