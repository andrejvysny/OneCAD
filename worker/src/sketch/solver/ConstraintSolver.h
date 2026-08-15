// Ported from OneCAD-CPP src/core/sketch/solver/ConstraintSolver.h @ b4ddcccc (2026-07-16)
/**
 * @file ConstraintSolver.h
 * @brief Constraint solver wrapper interface for PlaneGCS integration
 *
 * Per SPECIFICATION.md §23.3-23.8:
 * This class wraps the PlaneGCS library, providing a clean interface
 * for the sketch system while handling all PlaneGCS-specific details.
 *
 * IMPLEMENTATION STATUS: PlaneGCS integration in progress.
 *
 * Key design decisions from spec:
 * - Direct parameter binding (no copying) for performance
 * - DogLeg algorithm by default (LM fallback)
 * - 1e-4mm tolerance
 * - 30 FPS solve throttling
 * - Background threading for >100 entities
 */
#ifndef ONECAD_CORE_SKETCH_SOLVER_CONSTRAINT_SOLVER_H
#define ONECAD_CORE_SKETCH_SOLVER_CONSTRAINT_SOLVER_H

#include "../SketchTypes.h"
#include <atomic>
#include <chrono>
#include <deque>
#include <memory>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>
#include <functional>

// Forward declaration - PlaneGCS types
namespace GCS {
    class System;
}

namespace onecad::core::sketch {

// Forward declarations
class Sketch;
class SketchEntity;
class SketchPoint;
class SketchLine;
class SketchArc;
class SketchCircle;
class SketchConstraint;

/**
 * @brief Solver configuration options
 *
 * Per SPECIFICATION.md §23.4:
 * Default configuration uses DogLeg with 1e-4mm tolerance
 */
struct SolverConfig {
    /// Convergence tolerance in mm
    double tolerance = 1e-4;

    /// Maximum iterations per solve
    int maxIterations = 100;

    /// Algorithm selection
    enum class Algorithm {
        LevenbergMarquardt,  ///< Fallback nonlinear solver
        DogLeg,              ///< Default solver
        BFGS                 ///< Quasi-Newton method
    };
    Algorithm algorithm = Algorithm::DogLeg;

    /// Redundant constraint detection
    bool detectRedundant = true;

    /// Whether to apply results on partial solve
    bool applyPartialSolution = false;

    /// Timeout in milliseconds (0 = no timeout)
    int timeoutMs = 1000;
};

/**
 * @brief Result from a solve operation
 */
struct SolverResult {
    /// Overall success status
    bool success = false;

    /// Number of iterations used
    int iterations = 0;

    /// Final residual error
    double residual = 0.0;

    /// Time taken for solve
    std::chrono::microseconds solveTime{0};

    /// Status codes
    enum class Status {
        Uninitialized,    ///< Default state before solve
        Success,           ///< Fully converged
        PartialSuccess,    ///< Partially converged (some constraints satisfied)
        MaxIterations,     ///< Hit iteration limit
        Timeout,           ///< Hit time limit
        Diverged,          ///< Solution diverged
        Redundant,         ///< Redundant constraints detected
        Overconstrained,   ///< System is overconstrained
        Underconstrained,  ///< System is underconstrained (DOF > 0)
        InvalidInput,      ///< Invalid geometry or constraints
        InternalError      ///< PlaneGCS internal error
    };
    Status status = Status::Uninitialized;

    /// IDs of redundant constraints (if detectRedundant enabled)
    std::vector<ConstraintID> redundantConstraints;

    /// IDs of conflicting constraints
    std::vector<ConstraintID> conflictingConstraints;

    /// Human-readable error message
    std::string errorMessage;
};

/**
 * @brief Constraint solver wrapper for PlaneGCS
 *
 * This class manages the PlaneGCS solver instance and provides
 * translation between OneCAD sketch entities and PlaneGCS primitives.
 *
 * IMPLEMENTATION NOTE:
 * PlaneGCS uses direct parameter binding - we pass pointers to the
 * actual coordinate values in our entities, so when PlaneGCS modifies
 * them during solving, our entities are updated automatically.
 */
class ConstraintSolver {
public:
    /**
     * @brief Construct solver with default configuration
     */
    ConstraintSolver();

    /**
     * @brief Construct solver with custom configuration
     */
    explicit ConstraintSolver(const SolverConfig& config);

    ~ConstraintSolver();

    // Non-copyable
    ConstraintSolver(const ConstraintSolver&) = delete;
    ConstraintSolver& operator=(const ConstraintSolver&) = delete;
    ConstraintSolver(ConstraintSolver&&) = delete;
    ConstraintSolver& operator=(ConstraintSolver&&) = delete;

    /**
     * @brief Update configuration
     */
    void setConfig(const SolverConfig& config);
    const SolverConfig& getConfig() const { return config_; }

    // ========== System Building ==========

    /**
     * @brief Clear all entities and constraints from solver
     */
    void clear();

    /**
     * @brief Add a point to the solver
     * @param point Pointer to point entity (must remain valid)
     *
     * Per SPECIFICATION.md §23.5:
     * Uses direct parameter binding - stores pointer to point's coordinates
     *
     * Uses direct parameter binding to SketchPoint coordinates.
     */
    void addPoint(SketchPoint* point);

    /**
     * @brief Add a line to the solver
     *
     * Registers the line for constraint mapping (line uses point parameters).
     */
    void addLine(SketchLine* line);

    /**
     * @brief Add an arc to the solver
     *
     * Registers arc parameters (radius, start/end angles) for solving, and — for
     * an arc carrying endpoint points (W0b) — the internal arc rules that couple
     * those points to center+radius+angle.
     */
    void addArc(SketchArc* arc);

    /**
     * @brief Add a circle to the solver
     *
     * Registers circle radius for solving.
     */
    void addCircle(SketchCircle* circle);

    /**
     * @brief Add a constraint to the solver
     * @param constraint Constraint to add
     * @return true if constraint was added successfully
     *
     * Translates OneCAD constraint to PlaneGCS constraint(s).
     */
    bool addConstraint(SketchConstraint* constraint);

    // ===== Reference-lock pins (see `Sketch::referenceLockPins`) =====
    //
    // Freeze the parameters of host-face-projected geometry at their current
    // values. All three register under the INTERNAL tag 0, like the W0b arc
    // rules: structurally required, never blamed, and absent from
    // `gcsTagToConstraint_`, so a pin can never surface in
    // `conflicting[]`/`redundant[]`. `SolverAdapter` drives them AFTER entities
    // and user constraints, from the single pin set `Sketch` computes.

    /// Hold a point at its current (x, y). No-op for an unregistered id.
    void pinPointPosition(EntityID pointId);

    /// Hold an arc's / circle's radius at its current value.
    void pinRadius(EntityID arcOrCircleId);

    /// Hold an arc's start AND end angle at their current values.
    void pinArcAngles(EntityID arcId);

    /**
     * @brief Remove an entity from the solver
     */
    void removeEntity(EntityID id);

    /**
     * @brief Remove a constraint from the solver
     */
    void removeConstraint(ConstraintID id);

    // ========== Solving ==========

    /**
     * @brief Solve the constraint system
     *
     * Per SPECIFICATION.md §23.4:
     * 1. Calls PlaneGCS solve()
     * 2. If success, entity coordinates are already updated (direct binding)
     * 3. If failure, original coordinates preserved
     *
     * Calls PlaneGCS solve() and applies or reverts the solution.
     */
    SolverResult solve();

    /**
     * @brief Solve with a point being dragged
     * @param pointId Point being dragged by user
     * @param targetPos Target position
     * @param pointIdsToFix If empty, fixes all non-dragged points. Otherwise fixes only points in this set.
     *
     * Per SPECIFICATION.md §5.13:
     * Implements rubber-band dragging with spring resistance
     *
     * Current implementation adds temporary coordinate constraints for the dragged point.
     */
    SolverResult solveWithDrag(EntityID pointId, const Vec2d& targetPos,
                               const std::unordered_set<EntityID>& pointIdsToFix = {});

    SolverResult solveWithGroupDrag(const std::unordered_map<EntityID, Vec2d>& targetPositions);

    /**
     * @brief One drag step's worth of intent: where points should go, what radii
     *        should become, and which points must not move.
     *
     * `pinnedPoints` is held at its CURRENT position — the caller chooses the
     * pin set per gesture kind, because there is no set that is right for all of
     * them (pinning every other point over-determines an arc-endpoint reshape,
     * pinning nothing lets a body drag translate the whole sketch).
     * A point that is both a target and a pin is TARGETED: the target is the
     * user's intent, the pin is a default.
     */
    struct DragTargets {
        std::unordered_map<EntityID, Vec2d> points;
        std::unordered_map<EntityID, double> radii;
        std::unordered_set<EntityID> pinnedPoints;
    };

    /**
     * @brief Generalized drag solve: drive point positions AND curve radii
     *        toward `targets` while holding `targets.pinnedPoints`.
     *
     * The point-only cases (`solveWithDrag`, `solveWithGroupDrag`) stay as they
     * are; this is the superset the SCHEMA §7.4 gesture kinds need — `radius`
     * moves no point at all, and `arcEnd` needs a pin set neither of the others
     * can express.
     *
     * Targets are ADVISORY: they are temporary tag(−1) drives, so any committed
     * constraint outranks them and a converged solve that lands away from the
     * target is still a success. Substepped like the other drag entries; a
     * substep that fails restores the pre-drag pose and returns.
     *
     * The caller owns the SCHEMA §7.4 `MIN_GEOMETRY_SIZE` radius floor — a
     * radius target is handed to PlaneGCS verbatim.
     */
    SolverResult solveWithTargets(const DragTargets& targets);

    /**
     * @brief Apply solution from last successful solve
     *
     * Used when applyPartialSolution is false but caller wants
     * to manually apply after checking result.
     */
    void applySolution();

    /**
     * @brief Revert to state before last solve
     *
     * Per SPECIFICATION.md §23.7:
     * If solve fails, revert to pre-solve geometry
     */
    void revertSolution();

    // ========== DOF & Analysis ==========

    /**
     * @brief Analyze constraint system for redundancies
     *
     * Uses PlaneGCS redundancy analysis to find:
     * - Redundant constraints (remove without changing solution)
     * - Conflicting constraints (no solution exists)
     *
     * Uses PlaneGCS redundancy analysis results (if available).
     */
    std::vector<ConstraintID> findRedundantConstraints() const;

    /**
     * @brief Check if system is solvable
     */
    bool isSolvable() const;

    /**
     * @brief PlaneGCS diagnosis of the declared system.
     * @return True degrees of freedom (redundant constraints do not reduce
     *         DOF, unlike a naive constraint count), or -1 when unavailable.
     */
    int diagnose();

    /// True when the last solve/diagnose found genuinely conflicting constraints.
    bool hasConflicting() const;

    /// True when the last solve/diagnose found redundant constraints.
    bool hasRedundant() const;

private:
    struct DragSolveSnapshot {
        std::unordered_map<EntityID, Vec2d> pointPositions;
        struct ArcState {
            double radius = 0.0;
            double startAngle = 0.0;
            double endAngle = 0.0;
        };
        std::unordered_map<EntityID, ArcState> arcStates;
        std::unordered_map<EntityID, double> circleRadii;
    };

    SolverConfig config_;

    /// PlaneGCS system instance
    std::unique_ptr<GCS::System> gcsSystem_;

    /// Mapping from OneCAD entity IDs to PlaneGCS internal IDs
    std::unordered_map<EntityID, int> entityToGcsId_;

    /// Mapping from OneCAD constraint IDs to PlaneGCS constraint tags
    std::unordered_map<ConstraintID, int> constraintToGcsTag_;
    std::unordered_map<int, ConstraintID> gcsTagToConstraint_;

    /// Direct pointers to entity parameters for backup/restore
    struct ParameterBackup {
        EntityID entityId;
        EntityType type = EntityType::Point;
        std::vector<double> values;
    };
    std::vector<ParameterBackup> parameterBackup_;

    std::unordered_map<EntityID, SketchPoint*> pointsById_;
    std::unordered_map<EntityID, SketchLine*> linesById_;
    std::unordered_map<EntityID, SketchArc*> arcsById_;
    std::unordered_map<EntityID, SketchCircle*> circlesById_;
    std::vector<SketchConstraint*> constraints_;

    /// Parameter pointers used for direct binding
    std::vector<double*> parameters_;
    std::vector<double*> drivenParameters_;

    /// Frozen target values for reference-lock pins. PlaneGCS keeps the raw
    /// `double*` it is handed, so this must be POINTER-STABLE for the solver's
    /// whole life — a deque never invalidates on push_back, a vector would.
    /// These are deliberately NOT in `parameters_`: they are constants, not
    /// unknowns.
    std::deque<double> pinValues_;

    int nextEntityTag_ = 1;
    int nextConstraintTag_ = 1;

    /// Async solve state

    /// Statistics
    int totalSolves_ = 0;
    int successfulSolves_ = 0;
    std::chrono::microseconds totalSolveTime_{0};

    /**
     * @brief Backup all parameter values before solve
     */
    void backupParameters();

    /**
     * @brief Restore parameters from backup
     */
    void restoreParameters();

    /**
     * @brief Translate OneCAD constraint to PlaneGCS constraint
     */
    bool translateConstraint(SketchConstraint* constraint, int tagId);

    /**
     * @brief Couple an endpoint-bearing arc's start/end points to its
     *        center+radius+angle parameterization (PlaneGCS `ArcRules`).
     *
     * DOF-neutral (4 new parameters, 4 independent equations) and registered
     * under the INTERNAL tag 0, so it is never reported as a user constraint.
     * No-op for an arc with derived endpoints.
     */
    void addArcRules(SketchArc* arc);

    void configureSystem();

    DragSolveSnapshot captureDragSolveSnapshot() const;
    void restoreDragSolveSnapshot(const DragSolveSnapshot& snapshot);
    SolverResult solveWithDragSingleStep(EntityID pointId, const Vec2d& targetPos,
                                         const std::unordered_set<EntityID>& pointIdsToFix);
    SolverResult solveWithGroupDragSingleStep(const std::unordered_map<EntityID, Vec2d>& targetPositions);
    SolverResult solveWithTargetsSingleStep(const DragTargets& targets);
};

// ========== DOF Calculation Table ==========
/**
 * Per SPECIFICATION.md §23.8 - DOF removed by each constraint type:
 *
 * | Constraint      | DOF Removed |
 * |-----------------|-------------|
 * | Coincident      | 2           |
 * | Horizontal      | 1           |
 * | Vertical        | 1           |
 * | Parallel        | 1           |
 * | Perpendicular   | 1           |
 * | Tangent         | 1           |
 * | Concentric      | 2           |
 * | Equal           | 1           |
 * | Distance        | 1           |
 * | Angle           | 1           |
 * | Radius          | 1           |
 * | Diameter        | 1           |
 * | Fixed           | 2           |
 * | Midpoint        | 2           |
 * | Symmetric       | 2           |
 */
inline int getConstraintDOFReduction(ConstraintType type) {
    switch (type) {
        case ConstraintType::Coincident:    return 2;
        case ConstraintType::Horizontal:    return 1;
        case ConstraintType::Vertical:      return 1;
        case ConstraintType::OnCurve:       return 1;
        case ConstraintType::Parallel:      return 1;
        case ConstraintType::Perpendicular: return 1;
        case ConstraintType::Tangent:       return 1;
        case ConstraintType::Concentric:    return 2;
        case ConstraintType::Equal:         return 1;
        case ConstraintType::Distance:      return 1;
        case ConstraintType::HorizontalDistance: return 1;
        case ConstraintType::VerticalDistance:   return 1;
        case ConstraintType::Angle:         return 1;
        case ConstraintType::Radius:        return 1;
        case ConstraintType::Diameter:      return 1;
        case ConstraintType::Fixed:         return 2;
        case ConstraintType::Midpoint:      return 2;
        case ConstraintType::Symmetric:     return 2;
        // SNAP P3: one shared coordinate, so one DOF — the same reduction the
        // line-only forms make, because PlaneGCS lowers all four to the same
        // primitive.
        case ConstraintType::HorizontalPoints: return 1;
        case ConstraintType::VerticalPoints:   return 1;
    }
    return 0;
}

} // namespace onecad::core::sketch

#endif // ONECAD_CORE_SKETCH_SOLVER_CONSTRAINT_SOLVER_H
