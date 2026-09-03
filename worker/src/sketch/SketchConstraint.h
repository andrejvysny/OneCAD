// Ported from OneCAD-CPP src/core/sketch/SketchConstraint.h @ b4ddcccc (2026-07-16)
/**
 * @file SketchConstraint.h
 * @brief Base class for all sketch constraints
 *
 * Constraints define geometric relationships between entities.
 * They reduce degrees of freedom and drive the constraint solver.
 */

#ifndef ONECAD_CORE_SKETCH_CONSTRAINT_H
#define ONECAD_CORE_SKETCH_CONSTRAINT_H

#include "SketchTypes.h"

#include <gp_Pnt2d.hxx>
#include <cmath>
#include <string>
#include <vector>
#include <memory>

namespace onecad::core::sketch {

// Forward declaration
class Sketch;

/**
 * @brief Abstract base class for all sketch constraints
 *
 * Constraints represent geometric relationships:
 * - Positional: Coincident, Horizontal, Vertical, Midpoint
 * - Relational: Parallel, Perpendicular, Tangent, Equal
 * - Dimensional: Distance, Angle, Radius
 *
 * Each constraint removes degrees of freedom from the sketch.
 */
class SketchConstraint {
public:
    virtual ~SketchConstraint() = default;

    //--------------------------------------------------------------------------
    // Identification
    //--------------------------------------------------------------------------

    /**
     * @brief Get unique constraint identifier
     */
    ConstraintID id() const { return m_id; }

    /**
     * @brief Get constraint type
     */
    virtual ConstraintType type() const = 0;

    /**
     * @brief Get human-readable type name
     */
    virtual std::string typeName() const = 0;

    /**
     * @brief Get display string (e.g., "Distance: 25.0 mm")
     */
    virtual std::string toString() const = 0;

    //--------------------------------------------------------------------------
    // Entity References
    //--------------------------------------------------------------------------

    /**
     * @brief Get list of entity IDs this constraint references
     */
    virtual std::vector<EntityID> referencedEntities() const = 0;

    /**
     * @brief Check if this constraint references a specific entity
     */
    bool references(const EntityID& entityId) const;

    //--------------------------------------------------------------------------
    // DOF Contribution
    //--------------------------------------------------------------------------

    /**
     * @brief Get number of degrees of freedom removed by this constraint
     * @return DOF removed (typically 1 or 2)
     */
    virtual int degreesRemoved() const = 0;

    //--------------------------------------------------------------------------
    // Validation
    //--------------------------------------------------------------------------

    /**
     * @brief Check if constraint is currently satisfied
     * @param sketch Sketch context for entity lookup
     * @param tolerance Satisfaction tolerance
     * @return true if within tolerance
     */
    virtual bool isSatisfied(const Sketch& sketch, double tolerance) const = 0;

    /**
     * @brief Get current error/residual of constraint
     * @param sketch Sketch context
     * @return Error value (0 = perfectly satisfied)
     */
    virtual double getError(const Sketch& sketch) const = 0;

    //--------------------------------------------------------------------------
    // Visualization (per SPECIFICATION.md §5.16)
    //--------------------------------------------------------------------------

    /**
     * @brief Get icon position for constraint visualization
     * @param sketch Sketch context
     * @return Position for constraint icon in sketch coordinates
     */
    virtual gp_Pnt2d getIconPosition(const Sketch& sketch) const = 0;

    /**
     * @brief Get dimension text position (for dimensional constraints)
     * @param sketch Sketch context
     * @return Position for dimension text
     */
    virtual gp_Pnt2d getDimensionTextPosition(const Sketch& sketch) const;

protected:
    SketchConstraint();
    explicit SketchConstraint(const ConstraintID& id);

    static ConstraintID generateId();

    ConstraintID m_id;
};

//==============================================================================
// Dimensional Constraint Base
//==============================================================================

/**
 * @brief Base class for dimensional constraints (Distance, Angle, Radius)
 *
 * Dimensional constraints have a numeric value that can be edited.
 */
class DimensionalConstraint : public SketchConstraint {
public:
    /**
     * @brief Get dimension value
     */
    double value() const { return m_value; }

    /**
     * @brief Get mutable pointer to dimension value (for solver binding)
     *
     * Unsafe: the pointer is valid only while the owning constraint exists and is unchanged.
     * Mutations bypass validation/notifications and are not thread-safe; callers must re-validate
     * and notify after edits (or use a setter for safe updates).
     */
    double* valuePtr() { return &m_value; }

    /**
     * @brief Get pointer to dimension value (const)
     *
     * Pointer lifetime is tied to the owning constraint instance.
     */
    const double* valuePtr() const { return &m_value; }

    /**
     * @brief Set dimension value
     * @param value New value (interpretation depends on subclass)
     */
    virtual void setValue(double value) { m_value = value; }

    /**
     * @brief Get units string for display
     * @return Units (e.g., "mm", "°")
     */
    virtual std::string units() const = 0;

protected:
    explicit DimensionalConstraint(double value = 0.0);
    DimensionalConstraint(const ConstraintID& id, double value);

    double m_value = 0.0;
};

// W-WP3a: ConstraintFactory (JSON-driven constraint deserialization) removed —
// serialization is Rust-owned; the worker constructs constraints programmatically
// via the concrete constraint constructors in constraints/Constraints.h.

/**
 * @brief SCHEMA §7.4 `maxResidual` (REPORTING ONLY) — the largest residual over
 *        `constraints`, each measured in ITS OWN dimension.
 *
 * The dimension is whatever `getError` returns for that kind: mm for a
 * length-valued constraint (Distance, Radius, Diameter, Tangent, Coincident, …),
 * radians for an angle-valued one (Angle, Parallel, Perpendicular). A single
 * number therefore MIXES dimensions — a reader displays it and never compares it
 * across kinds or against a tolerance of its own.
 *
 * A constraint whose entities cannot be read yields `+infinity` from `getError`;
 * it is SKIPPED here rather than winning the maximum, because an unreadable
 * constraint has no residual and "infinitely wrong" would be a lie about a
 * sketch that is merely incomplete. A non-finite value can therefore never reach
 * the wire, where it is rejected outright (SCHEMA §4). An empty — or wholly
 * unreadable — set measures 0.0: nothing is violated.
 *
 * Purely a measurement: nothing reads the answer back into a solve, so it
 * changes no decision.
 *
 * @param constraints any range of pointer-like handles (raw or `unique_ptr`);
 *        null entries are skipped.
 */
template <typename ConstraintPtrRange>
double maxConstraintResidual(const Sketch& sketch, const ConstraintPtrRange& constraints) {
    double worst = 0.0;
    for (const auto& constraint : constraints) {
        if (!constraint) continue;
        const double error = std::abs(constraint->getError(sketch));
        if (!std::isfinite(error)) continue;
        if (error > worst) worst = error;
    }
    return worst;
}

} // namespace onecad::core::sketch

#endif // ONECAD_CORE_SKETCH_CONSTRAINT_H
