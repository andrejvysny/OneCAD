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
#include "SketchConstraint.h"

#include <memory>
#include <vector>
#include <unordered_map>
#include <optional>

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
     */
    static SketchPlane XY() {
        return {{0, 0, 0}, {1, 0, 0}, {0, 1, 0}, {0, 0, 1}};
    }

    static SketchPlane XZ() {
        return {{0, 0, 0}, {1, 0, 0}, {0, 0, 1}, {0, -1, 0}};
    }

    static SketchPlane YZ() {
        return {{0, 0, 0}, {0, 1, 0}, {0, 0, 1}, {1, 0, 0}};
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
     */
    EntityID addArc(EntityID centerId, double radius,
                    double startAngle, double endAngle, bool construction = false);

    /**
     * @brief Add a circle
     * @param centerId ID of center point
     * @param radius Circle radius
     */
    EntityID addCircle(EntityID centerId, double radius, bool construction = false);

    /**
     * @brief Remove an entity and all constraints referencing it
     */
    bool removeEntity(EntityID id);

    /**
     * @brief Get entity by ID
     * @return Pointer to entity, or nullptr if not found
     */
    SketchEntity* getEntity(EntityID id);
    const SketchEntity* getEntity(EntityID id) const;

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
     * @brief Add radius constraint to arc or circle
     */
    ConstraintID addRadius(EntityID arcOrCircle, double radius);

    /**
     * @brief Add angle constraint between two lines
     */
    ConstraintID addAngle(EntityID line1, EntityID line2, double angleDegrees);

    /**
     * @brief Add fixed constraint to point
     */
    ConstraintID addFixed(EntityID point);

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
     * @brief Get total degrees of freedom
     *
     * Per SPECIFICATION.md §23.8:
     * DOF = Σ(entity DOF) - Σ(constraint DOF removed)
     */
    int getDegreesOfFreedom() const;

    /**
     * @brief Check if sketch is fully constrained (DOF == 0)
     */
    bool isFullyConstrained() const { return getDegreesOfFreedom() == 0; }

    /**
     * @brief Check if sketch is over-constrained (has conflicts)
     */
    bool isOverConstrained() const;

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

    // ========== Serialization ==========

    /**
     * @brief Serialize sketch to JSON
     */
    std::string toJson() const;

    /**
     * @brief Deserialize sketch from JSON
     * @return nullptr on malformed input
     */
    static std::unique_ptr<Sketch> fromJson(const std::string& json);

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

    std::vector<std::unique_ptr<SketchEntity>> entities_;
    std::vector<std::unique_ptr<SketchConstraint>> constraints_;

    // Fast lookup maps
    std::unordered_map<EntityID, size_t> entityIndex_;
    std::unordered_map<ConstraintID, size_t> constraintIndex_;

    // Solver (PlaneGCS wrapper)
    std::unique_ptr<ConstraintSolver> solver_;
    bool solverDirty_ = true;  // Needs rebuild if true

    // Cached DOF calculation
    mutable int cachedDOF_ = -1;
    mutable bool dofDirty_ = true;

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
};

} // namespace onecad::core::sketch

#endif // ONECAD_CORE_SKETCH_SKETCH_H
