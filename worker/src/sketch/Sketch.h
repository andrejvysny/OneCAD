// Ported from OneCAD-CPP src/core/sketch/Sketch.h @ b4ddcccc (2026-07-16)
/**
 * @file Sketch.h
 * @brief Main sketch class managing entities, constraints, and solver interaction
 *
 * The Sketch class is the central coordinator for the 2D parametric sketch system.
 * It owns all geometry entities and constraints, manages the constraint solver,
 * and provides the interface for editing operations.
 *
 * Per SPECIFICATION.md §5.12:
 * - Manages entity lifecycle and ID generation
 * - Coordinates with PlaneGCS solver (§23.4)
 * - Handles DOF tracking (§23.8)
 * - Supports undo/redo through state serialization
 */
#ifndef ONECAD_CORE_SKETCH_SKETCH_H
#define ONECAD_CORE_SKETCH_SKETCH_H

#include "SketchTypes.h"
#include "SketchEntity.h"
#include "SketchPoint.h"
#include "SketchLine.h"
#include "SketchArc.h"
#include "SketchCircle.h"
#include "SketchEllipse.h"
#include "SketchConstraint.h"

#include <memory>
#include <vector>
#include <unordered_map>
#include <unordered_set>
#include <optional>
#include <string>

// Forward declarations
namespace GCS { class System; }

namespace onecad::core::sketch {

// Forward declaration
class ConstraintSolver;

/**
 * @brief Sketch plane definition in 3D space
 *
 * Per SPECIFICATION.md §5.17:
 * Sketches are defined on planes. The plane provides the transformation
 * between 2D sketch coordinates and 3D world coordinates.
 */
struct SketchPlane {
    Vec3d origin;   ///< Origin point of the sketch plane in 3D
    Vec3d xAxis;    ///< X direction (normalized)
    Vec3d yAxis;    ///< Y direction (normalized)
    Vec3d normal;   ///< Normal direction (xAxis × yAxis)

    /**
     * @brief Default XY plane at origin
     * 
     * @note NON-STANDARD COORDINATE SYSTEM
     * This plane maps 2D sketch coordinates to 3D world coordinates as follows:
     * - User X (Sketch X) -> World Y+ (0,1,0)
     * - User Y (Sketch Y) -> World X- (-1,0,0)
     * - Normal -> World Z+ (0,0,1)
     * 
     * This orientation is intentional to align with the application's specific
     * viewport conventions where "Front" is World X+ and "Right" is World Y+.
     */
    static SketchPlane XY() {
        return {{0, 0, 0}, {0, 1, 0}, {-1, 0, 0}, {0, 0, 1}};
    }

    /**
     * @brief XZ plane
     * User X = Geom Y+ (0,1,0)
     * User Z = Geom Z+ (0,0,1)
     */
    static SketchPlane XZ() {
        return {{0, 0, 0}, {0, 1, 0}, {0, 0, 1}, {1, 0, 0}};
    }

    /**
     * @brief YZ plane
     * User Y = Geom X- (-1,0,0)
     * User Z = Geom Z+ (0,0,1)
     */
    static SketchPlane YZ() {
        return {{0, 0, 0}, {-1, 0, 0}, {0, 0, 1}, {0, 1, 0}};
    }

    /**
     * @brief Convert 2D sketch point to 3D world coordinates
     */
    Vec3d toWorld(const Vec2d& p2d) const {
        return {
            origin.x + p2d.x * xAxis.x + p2d.y * yAxis.x,
            origin.y + p2d.x * xAxis.y + p2d.y * yAxis.y,
            origin.z + p2d.x * xAxis.z + p2d.y * yAxis.z
        };
    }

    /**
     * @brief Project 3D world point to 2D sketch coordinates
     */
    Vec2d toSketch(const Vec3d& p3d) const {
        Vec3d rel = {p3d.x - origin.x, p3d.y - origin.y, p3d.z - origin.z};
        return {
            rel.x * xAxis.x + rel.y * xAxis.y + rel.z * xAxis.z,
            rel.x * yAxis.x + rel.y * yAxis.y + rel.z * yAxis.z
        };
    }
};

struct HostFaceAttachment {
    std::string bodyId;
    std::string faceId;
    int projectedBoundaryVersion = 0;

    bool isValid() const {
        return !bodyId.empty() && !faceId.empty();
    }
};

/**
 * @brief Result of a constraint solve operation
 */
struct SolveResult {
    bool success = false;
    int iterations = 0;
    double residual = 0.0;
    std::vector<EntityID> movedEntities;
    std::vector<ConstraintID> conflictingConstraints;
    std::string errorMessage;
};

/**
 * @brief Sketch validation result
 */
struct ValidationResult {
    bool valid = true;
    std::vector<std::string> warnings;
    std::vector<std::string> errors;
    std::vector<EntityID> invalidEntities;
};

struct ConstraintSupportResult {
    bool supported = true;
    std::string reason;
};

/**
 * @brief The parameters the solver must hold still because their owning entity
 *        is reference-locked (host-face projected geometry).
 *
 * `referenceLocked` means "this mirrors the model face — a constraint may
 * REFERENCE it, nothing may MOVE it". Constraints against locked geometry are
 * legal (that is how a profile snaps to the boundary), so immobility has to come
 * from the solver: every parameter a locked entity owns is pinned to its current
 * value under the INTERNAL tag 0, exactly like the W0b arc rules.
 *
 * The pin set is the entity's OWN parameterization, never a derived quantity:
 *  - Line   → both endpoint points (its whole DOF).
 *  - Circle → center point + radius.
 *  - Arc    → center point + radius + start/end ANGLE. Its `.start`/`.end`
 *             points are then held by the arc rules; pinning those directly
 *             instead would be 2 redundant equations AND go singular on a
 *             180° arc (the two circle-circle intersections merge, so the
 *             Jacobian drops rank and PlaneGCS reports the sketch redundant).
 *  - Ellipse→ nothing: ellipses are not registered with PlaneGCS at all.
 *  - a bare locked Point → nothing; a locked point that no locked curve owns is
 *             pinned by an explicit `Fixed` constraint from the producer.
 *
 * A point that already carries a user `Fixed` constraint is EXCLUDED — that
 * constraint already pins it, and adding a second copy would be reported as
 * genuine redundancy (`hasRedundantConstraints` reads PlaneGCS's raw flag, which
 * tag 0 does not hide).
 */
struct ReferenceLockPins {
    std::vector<EntityID> points;     ///< pinned at their current (x, y) — 2 equations each
    std::vector<EntityID> radii;      ///< arcs/circles with a pinned radius — 1 equation each
    std::vector<EntityID> arcAngles;  ///< arcs with pinned start+end angle — 2 equations each

    /// Equations these pins add — the term `naiveDegreesOfFreedom` must subtract.
    int equationCount() const {
        return static_cast<int>(2 * points.size() + radii.size() + 2 * arcAngles.size());
    }

    bool empty() const { return points.empty() && radii.empty() && arcAngles.empty(); }
};

/**
 * @brief Main sketch class
 *
 * Manages all 2D geometry and constraints for a single sketch.
 * Each sketch exists on a plane and can be converted to 3D geometry.
 */
class Sketch {
public:
    /**
     * @brief Construct sketch on given plane
     */
    explicit Sketch(const SketchPlane& plane = SketchPlane::XY());

    ~Sketch();

    // Non-copyable, movable
    Sketch(const Sketch&) = delete;
    Sketch& operator=(const Sketch&) = delete;
    Sketch(Sketch&&) noexcept;
    Sketch& operator=(Sketch&&) noexcept;

    // ========== Entity Management ==========

    /**
     * @brief Add a point to the sketch
     * @return EntityID of the created point
     */
    EntityID addPoint(double x, double y, bool construction = false);

    /**
     * @brief Add a line between two points
     * @param startId ID of start point
     * @param endId ID of end point
     * @return EntityID of the created line, or 0 if points don't exist
     */
    EntityID addLine(EntityID startId, EntityID endId, bool construction = false);

    /**
     * @brief Add a line, creating new points at given coordinates
     */
    EntityID addLine(double x1, double y1, double x2, double y2, bool construction = false);

    /**
     * @brief Add an arc
     * @param centerId ID of center point
     * @param radius Arc radius
     * @param startAngle Start angle in radians
     * @param endAngle End angle in radians
     * @param startPointId Optional existing point to bind as the arc's START
     * @param endPointId   Optional existing point to bind as the arc's END
     *
     * Passing both endpoint ids makes them REAL solver handles coupled to
     * center+radius+angle by PlaneGCS arc rules (W0b — see
     * `SketchArc::setEndpointPointIds`). Both must resolve to existing points or
     * neither is bound (endpoints stay derived). Omitting them is the legacy
     * OneCAD-CPP behaviour.
     */
    EntityID addArc(EntityID centerId, double radius,
                    double startAngle, double endAngle, bool construction = false,
                    EntityID startPointId = {}, EntityID endPointId = {});

    /**
     * @brief Add a circle
     * @param centerId ID of center point
     * @param radius Circle radius
     */
    EntityID addCircle(EntityID centerId, double radius, bool construction = false);

    /**
     * @brief Add a circle, creating a new center point at given coordinates
     */
    EntityID addCircle(double centerX, double centerY, double radius, bool construction = false);

    /**
     * @brief Add an ellipse
     * @param centerId ID of center point
     * @param majorRadius Major axis radius (distance from center to ellipse edge along major axis)
     * @param minorRadius Minor axis radius (distance from center to ellipse edge along minor axis)
     * @param rotation Rotation angle of major axis (radians)
     */
    EntityID addEllipse(EntityID centerId, double majorRadius, double minorRadius,
                        double rotation = 0.0, bool construction = false);

    /**
     * @brief Remove an entity and all constraints referencing it
     */
    bool removeEntity(EntityID id);

    /**
     * @brief Split a line at a point, creating two line segments
     * @param lineId ID of line to split
     * @param splitPoint Point where to split (must be on line)
     * @return IDs of two new line segments (empty if failed)
     *
     * Creates intermediate point at splitPoint, removes original line,
     * creates two new lines. Migrates constraints to new segments.
     */
    std::pair<EntityID, EntityID> splitLineAt(EntityID lineId, const Vec2d& splitPoint);

    /**
     * @brief Split an arc at an angle, creating two arc segments
     * @param arcId ID of arc to split
     * @param splitAngle Angle where to split (radians, must be within arc extent)
     * @return IDs of two new arc segments (empty if failed)
     *
     * Creates point on arc at splitAngle, removes original arc,
     * creates two new arcs sharing the split point.
     */
    std::pair<EntityID, EntityID> splitArcAt(EntityID arcId, double splitAngle);

    /**
     * @brief Get entity by ID
     * @return Pointer to entity, or nullptr if not found
     */
    SketchEntity* getEntity(EntityID id);
    const SketchEntity* getEntity(EntityID id) const;

    /**
     * @brief Check if a sketch entity is locked as host-face reference geometry.
     */
    bool isEntityReferenceLocked(EntityID id) const;

    /**
     * @brief Set or clear host-reference lock on a sketch entity.
     */
    bool setEntityReferenceLocked(EntityID id, bool locked);

    /**
     * @brief The solver pins implied by this sketch's reference-locked entities.
     *
     * Single source of truth for both consumers: `SolverAdapter` materializes
     * them as tag-0 PlaneGCS constraints, and `naiveDegreesOfFreedom` subtracts
     * `equationCount()` (those equations live nowhere in `constraints_`, so the
     * ellipse fallback would otherwise over-report DOF — the same fix-up the W0b
     * arc rules needed). Empty for every sketch with no locked geometry.
     */
    ReferenceLockPins referenceLockPins() const;

    /**
     * @brief Get typed entity
     */
    template<typename T>
    T* getEntityAs(EntityID id) {
        auto* e = getEntity(id);
        return e ? dynamic_cast<T*>(e) : nullptr;
    }

    /**
     * @brief Get typed entity (const)
     */
    template<typename T>
    const T* getEntityAs(EntityID id) const {
        auto* e = getEntity(id);
        return e ? dynamic_cast<const T*>(e) : nullptr;
    }

    /**
     * @brief Get all entities of a specific type
     */
    std::vector<SketchEntity*> getEntitiesByType(EntityType type);

    /**
     * @brief Get all entities
     */
    const std::vector<std::unique_ptr<SketchEntity>>& getAllEntities() const { return entities_; }

    // ========== Constraint Management ==========

    /**
     * @brief Add a constraint to the sketch
     * @return ConstraintID, or 0 if constraint is invalid
     */
    ConstraintID addConstraint(std::unique_ptr<SketchConstraint> constraint);

    /**
     * @brief Validate that a constraint is implemented for its referenced entities.
     */
    ConstraintSupportResult validateConstraintSupport(const SketchConstraint& constraint) const;

    /**
     * @brief Add coincident constraint between two points
     */
    ConstraintID addCoincident(EntityID point1, EntityID point2);

    /**
     * @brief Add horizontal constraint to line or two points
     *
     * If point2 is empty, lineOrPoint1 is treated as a line ID.
     */
    ConstraintID addHorizontal(EntityID lineOrPoint1, EntityID point2 = {});

    /**
     * @brief Add vertical constraint to line or two points
     *
     * If point2 is empty, lineOrPoint1 is treated as a line ID.
     */
    ConstraintID addVertical(EntityID lineOrPoint1, EntityID point2 = {});

    /**
     * @brief Add parallel constraint between two lines
     */
    ConstraintID addParallel(EntityID line1, EntityID line2);

    /**
     * @brief Add perpendicular constraint between two lines
     */
    ConstraintID addPerpendicular(EntityID line1, EntityID line2);

    /**
     * @brief Add distance constraint
     */
    ConstraintID addDistance(EntityID entity1, EntityID entity2, double distance);

    /**
     * @brief Add signed horizontal distance constraint between two points (x2 - x1).
     */
    ConstraintID addHorizontalDistance(EntityID point1, EntityID point2, double distance);

    /**
     * @brief Add signed vertical distance constraint between two points (y2 - y1).
     */
    ConstraintID addVerticalDistance(EntityID point1, EntityID point2, double distance);

    /**
     * @brief Add radius constraint to arc or circle
     */
    ConstraintID addRadius(EntityID arcOrCircle, double radius);

    /**
     * @brief Add diameter constraint to arc or circle
     */
    ConstraintID addDiameter(EntityID arcOrCircle, double diameter);

    /**
     * @brief Add concentric constraint between two arcs/circles
     */
    ConstraintID addConcentric(EntityID entity1, EntityID entity2);

    /**
     * @brief Add symmetric constraint for two points about an axis line
     */
    ConstraintID addSymmetric(EntityID point1, EntityID point2, EntityID axisLine);

    /**
     * @brief Add angle constraint between two lines
     */
    ConstraintID addAngle(EntityID line1, EntityID line2, double angleDegrees);

    /**
     * @brief Add fixed constraint to point
     *
     * Only point entities are accepted. Returns empty ID if entity is not a point.
     */
    ConstraintID addFixed(EntityID point);

    /**
     * @brief Check if a point has a Fixed constraint
     */
    bool hasFixedConstraint(EntityID pointId) const;

    /**
     * @brief Add point-on-curve constraint
     * @param pointId ID of point to constrain
     * @param curveId ID of curve (Arc, Circle, Ellipse, or Line)
     * @param position Start/End/Arbitrary (auto-detected if Arbitrary)
     */
    ConstraintID addPointOnCurve(EntityID pointId, EntityID curveId,
                                  CurvePosition position = CurvePosition::Arbitrary);

    /**
     * @brief Remove a constraint
     */
    bool removeConstraint(ConstraintID id);

    /**
     * @brief Get constraint by ID
     */
    SketchConstraint* getConstraint(ConstraintID id);
    const SketchConstraint* getConstraint(ConstraintID id) const;

    /**
     * @brief Get all constraints
     */
    const std::vector<std::unique_ptr<SketchConstraint>>& getAllConstraints() const { return constraints_; }

    /**
     * @brief Get constraints referencing a specific entity
     */
    std::vector<SketchConstraint*> getConstraintsForEntity(EntityID entityId);

    // ========== Solver Interface ==========

    /**
     * @brief Solve all constraints
     * @return Result containing success status and diagnostics
     *
     * Per SPECIFICATION.md §23.4:
     * Uses PlaneGCS with LevenbergMarquardt algorithm
     * Tolerance: 1e-4mm
     */
    SolveResult solve();

    /**
     * @brief Solve constraints with a specific point being dragged
     * @param draggedPoint Point being moved by user
     * @param targetPos Target position for dragged point
     */
    SolveResult solveWithDrag(EntityID draggedPoint, const Vec2d& targetPos);

    /**
     * @brief Start a point-drag session and compute point-fixing strategy.
     */
    void beginPointDrag(EntityID draggedPoint);

    /**
     * @brief End active point-drag session.
     */
    void endPointDrag();

    /**
     * @brief Start a group-drag session for a selected point set.
     */
    void beginGroupDrag(const std::unordered_set<EntityID>& selectedPointIds);

    /**
     * @brief Solve constraints for an active group-drag session.
     * @param targetPositions Target positions keyed by dragged point IDs.
     */
    SolveResult solveWithGroupDrag(const std::unordered_map<EntityID, Vec2d>& targetPositions);

    /**
     * @brief End active group-drag session.
     */
    void endGroupDrag();

    /**
     * @brief Generalized direct-manipulation solve (SCHEMA §7.4 gesture kinds).
     * @param pointTargets Where each named point should go.
     * @param radiusTargets What each named Arc/Circle radius should become.
     * @param pinnedPoints Points held at their CURRENT position for this solve.
     *
     * The sibling of solveWithDrag for gestures that are not "one point to one
     * position": `radius` moves no point at all, `arcEnd` needs the sibling
     * endpoint FREE (pinning every other point over-determines the reshape), and
     * `entityBody` needs nothing pinned so coupled geometry can follow.
     *
     * Unlike solveWithGroupDrag this is NOT rigid-or-reject: the targets are
     * advisory tag(−1) drives, so a committed constraint that pulls the result
     * away from the target is a SUCCESS reporting the solved pose. The caller
     * owns the §7.4 MIN_GEOMETRY_SIZE radius floor and the MIN_ARC_SWEEP guard.
     */
    SolveResult solveWithTargets(const std::unordered_map<EntityID, Vec2d>& pointTargets,
                                 const std::unordered_map<EntityID, double>& radiusTargets,
                                 const std::unordered_set<EntityID>& pinnedPoints);

    /**
     * @brief Every point entity the given entity OWNS, in handle order.
     * @return center, start, end (Arc) · center (Circle/Ellipse) ·
     *         p0, p1 (Line) · itself (Point) · empty for anything else.
     *
     * Handle order is contractual: SCHEMA §7.4 anchors a grab-less `entityBody`
     * gesture on the FIRST id returned here. An arc with derived (non-entity)
     * endpoints contributes its center only.
     */
    std::vector<EntityID> entityPointIds(EntityID entityId) const;

    /**
     * @brief Get free direction(s) for a point for constrained drag (e.g. Shift-drag).
     * @param pointId Point entity ID (must be a SketchPoint).
     * @return Unit vectors along which the point can move; empty = fixed, 1 = line, 2 = plane.
     */
    std::vector<Vec2d> getPointFreeDirections(EntityID pointId) const;

    /**
     * @brief Get total degrees of freedom
     *
     * PlaneGCS diagnosis (redundant-aware) when the sketch is fully
     * solver-representable; naive Σ(entity DOF) − Σ(constraint DOF removed)
     * fallback for sketches containing solver-unsupported entities (ellipses).
     */
    int getDegreesOfFreedom() const;

    /// Naive static DOF count (see getDegreesOfFreedom); may go negative.
    int naiveDegreesOfFreedom() const;

    /// True when an entity type not registered with PlaneGCS is present.
    bool hasSolverUnsupportedEntities() const;

    /**
     * @brief True when some entity carries an INTERNAL solver coupling — an arc
     *        whose endpoints are real points bound by arc rules (W0b), or a
     *        reference-locked entity whose parameters are pinned.
     *
     * The solve/drag fast paths short-circuit on `constraints_.empty()` and just
     * teleport the geometry. That is only sound while every relationship in the
     * sketch is a user constraint; an endpoint-bearing arc has four equations
     * that live nowhere in `constraints_`, so a sketch holding one must always go
     * through PlaneGCS or a drag would tear the endpoint off its own circle. Pins
     * are the same kind of hidden equation — teleporting past them would let a
     * group drag walk locked host-face geometry off the face.
     */
    bool hasInternalCouplings() const;

    /**
     * @brief Check if sketch is fully constrained (DOF == 0)
     */
    bool isFullyConstrained() const { return getDegreesOfFreedom() == 0; }

    /**
     * @brief Check if sketch is over-constrained (has conflicts)
     */
    bool isOverConstrained() const;

    /**
     * @brief Check if the sketch carries benign, DOF-preserving redundant
     *        constraints (e.g. a duplicate dimension) per PlaneGCS diagnosis.
     *
     * Distinct from isOverConstrained() (genuine conflicts, no solution): a
     * redundant sketch still has a valid solution — the extra constraint removes
     * no degree of freedom. Mirrors isOverConstrained()'s diagnosis path; returns
     * false when the sketch is not fully solver-representable (ellipses).
     */
    bool hasRedundantConstraints() const;

    /**
     * @brief Get list of conflicting constraints if over-constrained
     */
    std::vector<ConstraintID> getConflictingConstraints() const;

    // ========== Plane & Coordinate System ==========

    /**
     * @brief Get the sketch plane
     */
    const SketchPlane& getPlane() const { return plane_; }

    /**
     * @brief Set the sketch plane
     */
    void setPlane(const SketchPlane& plane) {
        plane_ = plane;
        invalidateSolver();
    }

    /**
     * @brief Translate the sketch plane origin by a delta in sketch coordinates.
     * Entity coordinates and constraint values are unchanged; only plane.origin moves in world space.
     */
    void translatePlaneInSketch(const Vec2d& deltaSketch);

    /**
     * @brief Translate all sketch geometry by (dx, dy) in sketch coordinates.
     * Updates every point position and every Fixed constraint's stored (x,y).
     * @return false — mutating NOTHING — if any entity is reference-locked.
     */
    bool translateSketch(double dx, double dy);

    /**
     * @brief Translate only the points (and their Fixed constraints) that belong to the given region.
     * @return false — mutating NOTHING — for an empty/unknown region or if any
     *         member of the region is reference-locked.
     */
    bool translateSketchRegion(const std::string& regionId, double dx, double dy);

    // ========== Optional Host Attachment ==========

    const std::optional<HostFaceAttachment>& hostFaceAttachment() const { return hostFaceAttachment_; }

    bool hasHostFaceAttachment() const { return hostFaceAttachment_.has_value(); }

    void setHostFaceAttachment(const std::string& bodyId, const std::string& faceId);

    bool hasProjectedHostBoundaries() const {
        return hostFaceAttachment_.has_value() && hostFaceAttachment_->projectedBoundaryVersion > 0;
    }

    int projectedHostBoundariesVersion() const {
        return hostFaceAttachment_ ? hostFaceAttachment_->projectedBoundaryVersion : 0;
    }

    void setProjectedHostBoundariesVersion(int version);

    void clearHostFaceAttachment() { hostFaceAttachment_.reset(); }

    /**
     * @brief Convert 2D sketch point to 3D world
     */
    Vec3d toWorld(const Vec2d& p2d) const { return plane_.toWorld(p2d); }

    /**
     * @brief Convert 3D world point to 2D sketch
     */
    Vec2d toSketch(const Vec3d& p3d) const { return plane_.toSketch(p3d); }

    // ========== Validation ==========

    /**
     * @brief Validate sketch geometry
     *
     * Per SPECIFICATION.md §5.20:
     * - Checks for degenerate geometry (zero-length lines, zero-radius arcs)
     * - Verifies coordinate precision
     * - Detects orphaned points
     */
    ValidationResult validate() const;

    // W-WP3a: toJson/fromJson removed (Rust-owned serialization).

    // ========== Query & Hit Testing ==========

    /**
     * @brief Find entity nearest to a point within tolerance
     * @param pos Query position in sketch coordinates
     * @param tolerance Maximum distance to consider
     * @param filter Optional type filter
     * @return EntityID of nearest entity, or 0 if none found
     */
    EntityID findNearest(const Vec2d& pos, double tolerance,
                         std::optional<EntityType> filter = std::nullopt) const;

    /**
     * @brief Find all entities within a rectangular region
     */
    std::vector<EntityID> findInRect(const Vec2d& min, const Vec2d& max) const;

    // ========== Statistics ==========

    size_t getEntityCount() const { return entities_.size(); }
    size_t getConstraintCount() const { return constraints_.size(); }

private:
    SketchPlane plane_;
    std::optional<HostFaceAttachment> hostFaceAttachment_;

    std::vector<std::unique_ptr<SketchEntity>> entities_;
    std::vector<std::unique_ptr<SketchConstraint>> constraints_;

    // Fast lookup maps
    std::unordered_map<EntityID, size_t> entityIndex_;
    std::unordered_map<ConstraintID, size_t> constraintIndex_;

    // Solver (PlaneGCS wrapper)
    std::unique_ptr<ConstraintSolver> solver_;
    bool solverDirty_ = true;  // Needs rebuild if true
    std::vector<ConstraintID> lastConflictingConstraints_;

    // Cached DOF calculation
    mutable int cachedDOF_ = -1;
    mutable bool dofDirty_ = true;

    // Active point-drag session state
    std::unordered_set<EntityID> activeDragFixedPoints_;
    bool isDraggingPoint_ = false;
    std::unordered_map<EntityID, Vec2d> dragStartPositions_;
    bool dragSessionHadFailure_ = false;

    // Active group-drag session scaffold state
    std::unordered_set<EntityID> activeGroupDragPoints_;
    std::unordered_map<EntityID, Vec2d> groupDragStartPose_;
    bool isDraggingGroup_ = false;

    void restoreGroupDragStartPose();

    /**
     * @brief Mark solver as needing rebuild
     */
    void invalidateSolver();

    /**
     * @brief Rebuild solver from current entities/constraints
     */
    void rebuildSolver();

    /**
     * @brief Update entity index map after removal
     */
    void rebuildEntityIndex();

    /**
     * @brief Update constraint index map after removal
     */
    void rebuildConstraintIndex();

    /**
     * @brief Auto-detect curve position (Start/End/Arbitrary) for arc
     * @param pointId Point to check
     * @param arcId Arc to check against
     * @return Detected position based on proximity to arc endpoints
     */
    CurvePosition detectArcPosition(EntityID pointId, EntityID arcId) const;
};

} // namespace onecad::core::sketch

#endif // ONECAD_CORE_SKETCH_SKETCH_H
