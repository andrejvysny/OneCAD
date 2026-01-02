#ifndef ONECAD_RENDER_CAMERA3D_H
#define ONECAD_RENDER_CAMERA3D_H

#include <QVector3D>
#include <QMatrix4x4>

namespace onecad {
namespace render {

/**
 * @brief Orbit camera for 3D viewport navigation.
 * 
 * Uses target/position/up vectors for intuitive control:
 * - Orbit: rotate camera position around target
 * - Pan: move both camera and target in screen space
 * - Zoom: change distance from camera to target
 */
class Camera3D {
public:
    Camera3D();

    // Position and orientation
    void setPosition(const QVector3D& pos);
    void setTarget(const QVector3D& target);
    void setUp(const QVector3D& up);
    
    QVector3D position() const { return m_position; }
    QVector3D target() const { return m_target; }
    QVector3D up() const { return m_up; }
    
    // Derived vectors
    QVector3D forward() const;
    QVector3D right() const;
    float distance() const;

    // Navigation operations
    void orbit(float deltaYaw, float deltaPitch);
    void pan(float deltaX, float deltaY);
    void zoom(float delta);
    
    // Reset to default isometric view
    void reset();
    
    // Standard views
    void setFrontView();
    void setBackView();
    void setLeftView();
    void setRightView();
    void setTopView();
    void setBottomView();
    void setIsometricView();

    // Projection settings
    void setFov(float fov) { m_fov = fov; }
    void setNearPlane(float near) { m_nearPlane = near; }
    void setFarPlane(float far) { m_farPlane = far; }
    
    float fov() const { return m_fov; }
    float nearPlane() const { return m_nearPlane; }
    float farPlane() const { return m_farPlane; }

    // Matrix getters
    QMatrix4x4 viewMatrix() const;
    QMatrix4x4 projectionMatrix(float aspectRatio) const;

private:
    QVector3D m_position;
    QVector3D m_target;
    QVector3D m_up;
    
    float m_fov = 45.0f;
    float m_nearPlane = 0.1f;
    float m_farPlane = 100000.0f;
    
    static constexpr float MIN_DISTANCE = 1.0f;
    static constexpr float MAX_DISTANCE = 50000.0f;
    static constexpr float MIN_PITCH = -89.0f;
    static constexpr float MAX_PITCH = 89.0f;
};

} // namespace render
} // namespace onecad

#endif // ONECAD_RENDER_CAMERA3D_H
