#include "Camera3D.h"
#include <QtMath>

namespace onecad {
namespace render {

Camera3D::Camera3D() {
    reset();
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
    
    QVector3D rightVec = right();
    QVector3D upVec = m_up;
    
    QVector3D offset = rightVec * (-deltaX * scale) + upVec * (deltaY * scale);
    
    m_position += offset;
    m_target += offset;
}

void Camera3D::zoom(float delta) {
    float dist = distance();
    float newDist = dist - delta * dist * 0.001f;
    
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
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(0, -dist, 0);
    m_up = QVector3D(0, 0, 1);
}

void Camera3D::setBackView() {
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(0, dist, 0);
    m_up = QVector3D(0, 0, 1);
}

void Camera3D::setLeftView() {
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(-dist, 0, 0);
    m_up = QVector3D(0, 0, 1);
}

void Camera3D::setRightView() {
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(dist, 0, 0);
    m_up = QVector3D(0, 0, 1);
}

void Camera3D::setTopView() {
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(0, 0, dist);
    m_up = QVector3D(0, 1, 0);
}

void Camera3D::setBottomView() {
    float dist = distance();
    m_target = QVector3D(0, 0, 0);
    m_position = QVector3D(0, 0, -dist);
    m_up = QVector3D(0, -1, 0);
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

QMatrix4x4 Camera3D::projectionMatrix(float aspectRatio) const {
    QMatrix4x4 projection;
    projection.perspective(m_fov, aspectRatio, m_nearPlane, m_farPlane);
    return projection;
}

} // namespace render
} // namespace onecad
