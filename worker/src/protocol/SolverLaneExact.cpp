#include "SolverLane.h"

#include "SketchTruth.h"

namespace onecad::protocol {

SolverLane::FinalGestureSolve SolverLane::finish_pose(Gesture& gesture,
                                                     const nlohmann::json& args) {
    using nlohmann::json;
    FinalGestureSolve final;
    bool target_failed = false;
    bool branch_refused = false;
    if (args.contains("commit") && args["commit"].is_object() &&
        args["commit"].contains("finalTarget")) {
        const auto& target = args["commit"]["finalTarget"];
        if (target.is_array() && target.size() >= 2 && target[0].is_number() && target[1].is_number()) {
            const auto preview = run_step(gesture, target[0].get<double>(), target[1].get<double>());
            target_failed = !preview.success;
            branch_refused = gesture.kind == DragKind::Point && preview.errorMessage == "branchBoundary";
        }
    }
    // Preview pins/drives must be absent before final rank and residual truth.
    if (gesture.kind == DragKind::Point) gesture.sketch->endPointDrag();
    final.result = gesture.sketch->solve();
    final.refused = target_failed || !final.result.truth.admissible();
    if (!final.refused) return final;
    final.refusal = exact_truth_diagnostics(final.result.truth, gesture.index);
    if (final.refusal.empty()) {
        final.refusal.push_back(json{{"code", "OP_FAILED"}, {"severity", "error"},
            {"reasonCode", branch_refused ? "branchBoundary" : "numericallyUnresolved"}, {"stage", "sketchSolve"},
            {"message", "Final gesture target was refused; retained accepted pose"},
            {"evidence", json{{"restoredAcceptedPose", true}}}});
    }
    if (gesture.accepted_pose) gesture.accepted_pose->restore(*gesture.sketch);
    final.result = gesture.sketch->solve();
    return final;
}

}  // namespace onecad::protocol
