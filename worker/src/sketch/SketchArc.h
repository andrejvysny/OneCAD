// Ported from OneCAD-CPP src/core/sketch/SketchArc.h @ b4ddcccc (2026-07-16)
/**
 * @file SketchArc.h
 * @brief Circular arc entity for sketch system
 *
 * Per SPECIFICATION.md §5.12 (B2):
 * Parameterization: Center + radius + start/end angles
 * DOF: 3 (radius, startAngle, endAngle) + 2 from center point = 5 total
 *
 * This parameterization is preferred because:
 * - Natural for radius constraints
 * - Efficient for tangency calculations
 * - Direct to OpenGL arc primitives
 */

#ifndef ONECAD_CORE_SKETCH_ARC_H
#define ONECAD_CORE_SKETCH_ARC_H

#include "SketchEntity.h"
#include <gp_Pnt2d.hxx>
#include <gp_Vec2d.hxx>
#include <cmath>

namespace onecad::core::sketch {

/**
 * @brief Circular arc defined by center point, radius, and angular extent
 *
 * Arc geometry:
 * - Center point (referenced by ID)
 * - Radius (positive value)
 * - Start angle (radians, measured from +X axis)
 * - End angle (radians, measured from +X axis)
 *
 * Arc direction is always counter-clockwise from start to end angle.
 */
class SketchArc : public SketchEntity {
public:
    //--------------------------------------------------------------------------
    // Construction
    //--------------------------------------------------------------------------

    /**
     * @brief Default constructor (invalid arc)
     */
    SketchArc();

    /**
     * @brief Construct arc with specified parameters
     * @param centerPointId ID of center point
     * @param radius Arc radius (mm)
     * @param startAngle Start angle (radians)
     * @param endAngle End angle (radians)
     */
    SketchArc(const PointID& centerPointId, double radius,
              double startAngle, double endAngle);

    ~SketchArc() override = default;

    //--------------------------------------------------------------------------
    // Geometry Access
    //--------------------------------------------------------------------------

    /**
     * @brief Get center point ID
     */
    const PointID& centerPointId() const { return m_centerPointId; }

    /**
     * @brief Set center point reference
     */
    void setCenterPointId(const PointID& pointId) { m_centerPointId = pointId; }

    //--------------------------------------------------------------------------
    // Endpoint points (W0b — DIVERGENCE FROM THE PORT)
    //--------------------------------------------------------------------------
    //
    // OneCAD-CPP's arc has NO endpoint points: start/end are always derived from
    // center + radius + angles. That made `<id>.start`/`<id>.end` unaddressable —
    // a `Coincident` welding a wall to a cap could not be expressed at all
    // (SCHEMA §7.3 promises it; §7.4 promises `"pointId": "e3.start"`).
    //
    // An arc MAY now carry two real `SketchPoint` ids for its endpoints. When
    // set, `ConstraintSolver::addArc` couples them to center+radius+angle with
    // PlaneGCS `addConstraintArcRules` (4 equations for the 4 new parameters —
    // DOF-neutral), so the endpoints are genuine solver handles that other
    // constraints can reference and a drag can move.
    //
    // EMPTY = legacy derived behaviour: no minted points, no ArcRules, endpoints
    // exist only as `startPoint()`/`endPoint()` computations. Every geometric
    // accessor below stays angle-derived either way — the points MIRROR the
    // parameterization, they do not replace it.

    /// Id of the arc's start `SketchPoint`, or empty when endpoints are derived.
    const PointID& startPointId() const { return m_startPointId; }
    /// Id of the arc's end `SketchPoint`, or empty when endpoints are derived.
    const PointID& endPointId() const { return m_endPointId; }
    /// True when both endpoint points exist (⇒ the solver adds arc rules).
    bool hasEndpointPoints() const { return !m_startPointId.empty() && !m_endPointId.empty(); }

    /// Bind (or clear, with two empty ids) the arc's endpoint points.
    void setEndpointPointIds(const PointID& startPointId, const PointID& endPointId) {
        m_startPointId = startPointId;
        m_endPointId = endPointId;
    }

    /**
     * @brief Get arc radius
     * @return Radius in mm
     */
    double radius() const { return m_radius; }

    /**
     * @brief Get mutable reference to radius (for solver binding)
     *
     * Solver callers must keep radius non-negative or clamp after solve.
     */
    double& radius() { return m_radius; }

    /**
     * @brief Set arc radius
     * @param radius Radius in mm (must be positive)
     */
    void setRadius(double radius) { m_radius = std::max(0.0, radius); }

    /**
     * @brief Get start angle
     * @return Angle in radians
     */
    double startAngle() const { return m_startAngle; }

    /**
     * @brief Get mutable reference to start angle (for solver binding)
     *
     * Solver callers must keep angles normalized to [-pi, pi] or call
     * normalizeAngle() after solve to enforce invariants.
     */
    double& startAngle() { return m_startAngle; }

    /**
     * @brief Set start angle
     * @param angle Angle in radians
     */
    void setStartAngle(double angle) { m_startAngle = normalizeAngle(angle); }

    /**
     * @brief Get end angle
     * @return Angle in radians
     */
    double endAngle() const { return m_endAngle; }

    /**
     * @brief Get mutable reference to end angle (for solver binding)
     *
     * Solver callers must keep angles normalized to [-pi, pi] or call
     * normalizeAngle() after solve to enforce invariants.
     */
    double& endAngle() { return m_endAngle; }

    /**
     * @brief Set end angle
     * @param angle Angle in radians
     */
    void setEndAngle(double angle) { m_endAngle = normalizeAngle(angle); }

    //--------------------------------------------------------------------------
    // Derived Geometry
    //--------------------------------------------------------------------------

    /**
     * @brief Get angular extent of arc
     * @return Sweep angle in radians (always positive, CCW direction)
     */
    double sweepAngle() const;

    /**
     * @brief Get arc length
     * @return Length in mm
     */
    double arcLength() const;

    /**
     * @brief Calculate start point position
     * @param centerPos Center point position
     * @return Start point on arc circumference
     */
    gp_Pnt2d startPoint(const gp_Pnt2d& centerPos) const;

    /**
     * @brief Calculate end point position
     * @param centerPos Center point position
     * @return End point on arc circumference
     */
    gp_Pnt2d endPoint(const gp_Pnt2d& centerPos) const;

    /**
     * @brief Calculate midpoint of arc
     * @param centerPos Center point position
     * @return Point at middle of arc
     */
    gp_Pnt2d midpoint(const gp_Pnt2d& centerPos) const;

    /**
     * @brief Get tangent direction at start point
     * @return Unit tangent vector (CCW direction)
     */
    gp_Vec2d startTangent() const;

    /**
     * @brief Get tangent direction at end point
     * @return Unit tangent vector (CCW direction)
     */
    gp_Vec2d endTangent() const;

    /**
     * @brief Check if angle is within arc extent
     * @param angle Angle to test (radians)
     * @return true if angle is between start and end (CCW)
     */
    bool containsAngle(double angle) const;

    /**
     * @brief Calculate point on arc at given angle
     * @param centerPos Center position
     * @param angle Angle in radians
     * @return Point on arc (caller should check containsAngle() if needed)
     */
    gp_Pnt2d pointAtAngle(const gp_Pnt2d& centerPos, double angle) const;

    //--------------------------------------------------------------------------
    // SketchEntity Interface
    //--------------------------------------------------------------------------

    EntityType type() const override { return EntityType::Arc; }
    std::string typeName() const override { return "Arc"; }

    /**
     * @brief Base bounds requires center resolution (use boundsWithCenter via Sketch)
     */
    BoundingBox2d bounds() const override;

    /**
     * @brief Base hit test requires center resolution (use isNearWithCenter via Sketch)
     */
    bool isNear(const gp_Pnt2d& point, double tolerance) const override;

    /**
     * @brief Arc DOF: radius (1) + startAngle (1) + endAngle (1) = 3
     * @note Center point contributes its own 2 DOF separately
     * @note Minted ENDPOINT points likewise contribute their own 2 each, but the
     *       arc rules that couple them remove all 4 again. `Sketch::
     *       naiveDegreesOfFreedom` applies that −4 once per endpoint-bearing arc;
     *       this per-entity count stays 3 either way.
     */
    int degreesOfFreedom() const override { return 3; }


    //--------------------------------------------------------------------------
    // Bounds/hit test with center position (called by Sketch)
    //--------------------------------------------------------------------------

    BoundingBox2d boundsWithCenter(const gp_Pnt2d& centerPos) const;
    bool isNearWithCenter(const gp_Pnt2d& testPoint, const gp_Pnt2d& centerPos,
                          double tolerance) const;

    //--------------------------------------------------------------------------
    // Arc Dragging (per SPECIFICATION.md §5.19)
    //--------------------------------------------------------------------------

    /**
     * @brief Drag arc endpoint while keeping radius fixed
     * @param centerPos Center position
     * @param isDraggingStart true if dragging start point, false for end
     * @param newPos New desired position
     *
     * Per SPECIFICATION.md: "Arc endpoint dragging: Radius stays fixed"
     * The endpoint is projected onto the circle and the angle is updated.
     */
    void dragEndpoint(const gp_Pnt2d& centerPos, bool isDraggingStart,
                      const gp_Pnt2d& newPos);

private:
    PointID m_centerPointId;
    PointID m_startPointId;  // empty ⇒ derived (see setEndpointPointIds)
    PointID m_endPointId;    // empty ⇒ derived
    double m_radius = 0.0;
    double m_startAngle = 0.0;  // Radians
    double m_endAngle = 0.0;    // Radians

    /**
     * @brief Normalize angle to [-π, π] range
     */
    static double normalizeAngle(double angle);
};

} // namespace onecad::core::sketch

#endif // ONECAD_CORE_SKETCH_ARC_H
