// Ported from OneCAD-CPP src/core/sketch/solver/SolverAdapter.cpp @ b4ddcccc (2026-07-16)
#include "SolverAdapter.h"

#include "../Sketch.h"
#include "ConstraintSolver.h"
#include "util/Log.h"


namespace onecad::core::sketch {

bool SolverAdapter::populateSolver(Sketch& sketch, ConstraintSolver& solver) {
    solver.clear();

    for (const auto& entity : sketch.getAllEntities()) {
        if (entity && entity->type() == EntityType::Point) {
            solver.addPoint(dynamic_cast<SketchPoint*>(entity.get()));
        }
    }

    for (const auto& entity : sketch.getAllEntities()) {
        if (!entity) {
            continue;
        }

        switch (entity->type()) {
            case EntityType::Line:
                solver.addLine(dynamic_cast<SketchLine*>(entity.get()));
                break;
            case EntityType::Arc:
                solver.addArc(dynamic_cast<SketchArc*>(entity.get()));
                break;
            case EntityType::Circle:
                solver.addCircle(dynamic_cast<SketchCircle*>(entity.get()));
                break;
            default:
                break;
        }
    }

    bool ok = true;
    for (const auto& constraint : sketch.getAllConstraints()) {
        if (!addConstraintToSolver(constraint.get(), solver)) {
            ok = false;
            WLOG_WARN("%s", "populateSolver:constraint-translation-failed");
        }
    }

    // LAST: reference-lock pins. They must follow the user constraints because
    // the pin set skips points a `Fixed` already holds — pinning those twice
    // would be reported as genuine redundancy.
    const ReferenceLockPins pins = sketch.referenceLockPins();
    for (const auto& pointId : pins.points) {
        solver.pinPointPosition(pointId);
    }
    for (const auto& entityId : pins.radii) {
        solver.pinRadius(entityId);
    }
    for (const auto& arcId : pins.arcAngles) {
        solver.pinArcAngles(arcId);
    }

    return ok;
}

bool SolverAdapter::addConstraintToSolver(SketchConstraint* constraint, ConstraintSolver& solver) {
    if (!constraint) {
        return false;
    }
    return solver.addConstraint(constraint);
}

} // namespace onecad::core::sketch
