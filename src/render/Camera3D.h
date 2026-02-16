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
 *
 * Supports both orthographic and perspective projection:
 * - Camera angle 0° = orthographic
 * - Camera angle >0° = perspective with FOV = angle
 * - Seamless transitions preserve apparent scale (Shapr3D style)
 */
class Camera3D {
public:
    enum class ProjectionType {
        Orthographic,
        Perspective
    };

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
    void zoomByFactor(float factor);
    
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

    // Camera angle control (Shapr3D-style)
    // 0° = orthographic, >0° = perspective with FOV = angle
    void setCameraAngle(float degrees);
    float cameraAngle() const { return m_cameraAngle; }

    ProjectionType projectionType() const { return m_projectionType; }
    float orthoScale() const { return m_orthoScale; }
    void setOrthoScale(float scale) { m_orthoScale = scale; }

    // Matrix getters
    QMatrix4x4 viewMatrix() const;
    QMatrix4x4 projectionMatrix(float aspectRatio) const;

private:
    QVector3D m_position;
    QVector3D m_target;
    QVector3D m_up;

    // Projection parameters
    ProjectionType m_projectionType = ProjectionType::Perspective;
    float m_fov = 45.0f;           // Used in perspective mode
    float m_orthoScale = 1.0f;     // Used in orthographic mode (world units per viewport height)
    float m_cameraAngle = 45.0f;   // 0° = ortho, >0° = perspective with FOV=angle
    float m_nearPlane = 0.1f;
    float m_farPlane = 100000.0f;

    static constexpr float MIN_DISTANCE = 1.0f;
    static constexpr float MAX_DISTANCE = 50000.0f;
    static constexpr float MIN_PITCH = -89.0f;
    static constexpr float MAX_PITCH = 89.0f;
    static constexpr float MIN_PERSPECTIVE_FOV = 5.0f;  // Minimum FOV to avoid degeneracy
    static constexpr float MAX_PERSPECTIVE_FOV = 120.0f; // Maximum wide-angle FOV
};

} // namespace render
} // namespace onecad

#endif // ONECAD_RENDER_CAMERA3D_H
