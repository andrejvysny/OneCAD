#include <cassert>
#include <algorithm>
#include <cmath>
#include <limits>
#include <numbers>

#include "sketch/ResidualGate.h"
#include "sketch/Sketch.h"
#include "sketch/SketchPose.h"
#include "sketch/constraints/Constraints.h"

namespace sk = onecad::core::sketch;
namespace cs = sk::constraints;

sk::ResidualCheck assess(const sk::Sketch& sketch, const sk::SketchConstraint& c) {
    return sk::checkResiduals(sketch, {&c});
}

void length_boundary_is_inclusive() {
    for (const double residual : {0.00009999, 0.0001, 0.00010001}) {
        sk::Sketch sketch;
        const auto a = sketch.addPoint(0, 0);
        const auto b = sketch.addPoint(residual, 1);
        const cs::VerticalPointsConstraint c(a, b);
        const auto result = assess(sketch, c);
        assert(result.unassessed.empty());
        assert(result.violations.empty() == (residual <= 1e-4));
        assert(result.normalized_max == residual / 1e-4);
    }
}

void angular_budget_is_local() {
    sk::Sketch sketch;
    const auto a = sketch.addPoint(0, 0), b = sketch.addPoint(10, 0);
    const auto c = sketch.addPoint(0, 0);
    const auto d = sketch.addPoint(20 * std::sin(6e-6), 20 * std::cos(6e-6));
    const auto first = sketch.addLine(a, b), second = sketch.addLine(c, d);
    const cs::PerpendicularConstraint perpendicular(first, second);
    const auto before = assess(sketch, perpendicular);
    assert(before.unassessed.empty() && before.violations.size() == 1);
    assert(before.violations[0].dimension == sk::ResidualDimension::Angle);
    assert(std::abs(before.violations[0].tolerance - 5e-6) < 1e-18);
    const auto far_a = sketch.addPoint(0, 100), far_b = sketch.addPoint(10000, 100);
    sketch.addLine(far_a, far_b);
    assert(assess(sketch, perpendicular).normalized_max == before.normalized_max);
}

void all_twenty_kinds_have_a_typed_reader() {
    sk::Sketch sketch;
    const auto a = sketch.addPoint(0, 0), b = sketch.addPoint(10, 0);
    const auto c = sketch.addPoint(0, 10), d = sketch.addPoint(10, 10);
    const auto m = sketch.addPoint(5, 0), on = sketch.addPoint(20, 0);
    const auto line = sketch.addLine(a, b), parallel = sketch.addLine(c, d);
    const auto vertical = sketch.addLine(a, c);
    const auto circle = sketch.addCircle(c, 10), other = sketch.addCircle(c, 10);
    const cs::CoincidentConstraint coincident(a, a);
    const cs::HorizontalConstraint horizontal(line);
    const cs::VerticalConstraint v(vertical);
    const cs::FixedConstraint fixed(a, 0, 0);
    const cs::MidpointConstraint midpoint(m, line);
    const cs::PointOnCurveConstraint on_curve(on, line);
    const cs::ParallelConstraint p(line, parallel);
    const cs::PerpendicularConstraint perp(line, vertical);
    const cs::TangentConstraint tangent(line, circle);
    const cs::ConcentricConstraint concentric(circle, other);
    const cs::EqualConstraint equal(line, parallel);
    const cs::DistanceConstraint distance(a, b, 10);
    const cs::HorizontalDistanceConstraint hd(a, b, 10);
    const cs::VerticalDistanceConstraint vd(a, c, 10);
    const cs::AngleConstraint angle(line, vertical, std::numbers::pi / 2);
    const cs::RadiusConstraint radius(circle, 10);
    const cs::DiameterConstraint diameter(circle, 20);
    const cs::SymmetricConstraint symmetric(c, d, sketch.addLine(m, sketch.addPoint(5, 10)));
    const cs::HorizontalPointsConstraint hp(a, b);
    const cs::VerticalPointsConstraint vp(a, c);
    const std::vector<const sk::SketchConstraint*> constraints = {
        &coincident, &horizontal, &v, &fixed, &midpoint, &on_curve, &p, &perp, &tangent,
        &concentric, &equal, &distance, &hd, &vd, &angle, &radius, &diameter, &symmetric, &hp, &vp};
    const auto result = sk::checkResiduals(sketch, constraints);
    assert(constraints.size() == 20 && result.unassessed.empty() && result.violations.empty());
    // Arbitrary OnCurve uses the infinite supporting line, not its segment.
    assert(on_curve.getError(sketch) == 10);
    assert(assess(sketch, on_curve).normalized_max == 0);
}

void undefined_and_nonfinite_operands_are_unassessed() {
    sk::Sketch sketch;
    const auto a = sketch.addPoint(0, 0), b = sketch.addPoint(10, 0);
    const auto zero = sketch.addLine(a, a), line = sketch.addLine(a, b);
    const cs::ParallelConstraint undefined(zero, line);
    assert(assess(sketch, undefined).unassessed == std::vector{undefined.id()});
    const cs::SymmetricConstraint symmetric(a, b, zero);
    assert(assess(sketch, symmetric).unassessed == std::vector{symmetric.id()});
    const cs::PointOnCurveConstraint zero_support(b, zero);
    assert(assess(sketch, zero_support).unassessed == std::vector{zero_support.id()});
    const cs::PointOnCurveConstraint endpoint(a, zero, sk::CurvePosition::Start);
    assert(assess(sketch, endpoint).unassessed.empty());
    const cs::DistanceConstraint missing(a, "ghost", 10);
    assert(assess(sketch, missing).unassessed == std::vector{missing.id()});
    const auto ellipse = sketch.addEllipse(a, 10, 5);
    const cs::PointOnCurveConstraint unsupported(b, ellipse);
    assert(assess(sketch, unsupported).unassessed == std::vector{unsupported.id()});
    const cs::FixedConstraint nan(a, std::numeric_limits<double>::quiet_NaN(), 0);
    assert(assess(sketch, nan).unassessed == std::vector{nan.id()});
    sketch.getEntityAs<sk::SketchPoint>(b)->setPosition(std::numeric_limits<double>::infinity(), 0);
    const cs::HorizontalConstraint infinite(line);
    assert(assess(sketch, infinite).unassessed == std::vector{infinite.id()});
}

void missing_owned_points_fail_even_without_public_constraints() {
    sk::Sketch sketch;
    const auto center = sketch.addPoint(0, 0);
    const auto circle = sketch.addCircle(center, 10);
    sketch.getEntityAs<sk::SketchCircle>(circle)->setCenterPointId("ghost");
    const auto result = sketch.solve();
    assert(!result.success && !result.truth.publishable());
    assert(result.truth.conflicting.empty() && !result.truth.issues.empty());
}

void welded_tangent_requires_both_length_and_angle() {
    for (const int weld : {0, 1, 2}) {
        sk::Sketch sketch;
        const auto center = sketch.addPoint(0, 0), start = sketch.addPoint(10, 0);
        const auto end = sketch.addPoint(0, 10);
        const auto arc = sketch.addArc(center, 10, 0, std::numbers::pi / 2);
        sketch.getEntityAs<sk::SketchArc>(arc)->setEndpointPointIds(start, end);
        const auto line_start = weld ? sketch.addPoint(10, 0) : start;
        if (weld == 1) sketch.addCoincident(start, line_start);
        const auto line_end = sketch.addPoint(10 + 10 * std::sin(.001), 10 * std::cos(.001));
        const auto line = sketch.addLine(line_start, line_end);
        if (weld == 2) sketch.addConstraint(std::make_unique<cs::PointOnCurveConstraint>(start, line, sk::CurvePosition::Start));
        const cs::TangentConstraint tangent(line, arc);
        assert(tangent.getError(sketch) < 1e-4);
        const auto result = assess(sketch, tangent);
        assert(result.unassessed.empty() && result.violations.size() == 1);
        assert(result.violations[0].component == "endpointTangency");
        assert(std::abs(result.violations[0].residual - .001) < 1e-12);
    }
}

void welded_arc_arc_tangency_uses_the_larger_radius() {
    sk::Sketch sketch;
    const double delta = .001;
    const auto shared = sketch.addPoint(10, 0);
    const auto first_center = sketch.addPoint(0, 0);
    const auto first_end = sketch.addPoint(0, 10);
    const auto second_center = sketch.addPoint(10 + 20 * std::cos(delta), 20 * std::sin(delta));
    const double angle = std::numbers::pi + delta;
    const auto second_end = sketch.addPoint(10 + 20 * std::cos(delta) + 20 * std::cos(angle + std::numbers::pi / 2),
        20 * std::sin(delta) + 20 * std::sin(angle + std::numbers::pi / 2));
    const auto first = sketch.addArc(first_center, 10, 0, std::numbers::pi / 2);
    const auto second = sketch.addArc(second_center, 20, angle, angle + std::numbers::pi / 2);
    sketch.getEntityAs<sk::SketchArc>(first)->setEndpointPointIds(shared, first_end);
    sketch.getEntityAs<sk::SketchArc>(second)->setEndpointPointIds(shared, second_end);
    assert(sk::SketchPose(sketch).integrity(sketch).empty());
    const cs::TangentConstraint tangent(first, second);
    assert(tangent.getError(sketch) < 1e-4);
    const auto measured = assess(sketch, tangent);
    assert(measured.unassessed.empty() && measured.violations.size() == 1);
    assert(std::abs(measured.violations[0].tolerance - 5e-6) < 1e-18);
}

void proximity_is_not_a_weld() {
    sk::Sketch sketch;
    const auto center = sketch.addPoint(0, 0), start = sketch.addPoint(10, 0);
    const auto end = sketch.addPoint(0, 10);
    const auto arc = sketch.addArc(center, 10, 0, std::numbers::pi / 2);
    sketch.getEntityAs<sk::SketchArc>(arc)->setEndpointPointIds(start, end);
    const auto a = sketch.addPoint(10, 0), b = sketch.addPoint(10.01, 10);
    const cs::TangentConstraint tangent(sketch.addLine(a, b), arc);
    assert(assess(sketch, tangent).violations.empty());
}

void coupling_and_reference_locks_are_integrity_not_public_blame() {
    sk::Sketch sketch;
    const auto c = sketch.addPoint(0, 0), a = sketch.addPoint(10, 0), b = sketch.addPoint(0, 10);
    const auto arc_id = sketch.addArc(c, 10, 0, std::numbers::pi / 2);
    auto* arc = sketch.getEntityAs<sk::SketchArc>(arc_id);
    arc->setEndpointPointIds(a, b);
    const sk::SketchPose pose(sketch);
    for (double error : {0.00009999, 0.0001, 0.00010001}) {
        sketch.getEntityAs<sk::SketchPoint>(a)->setPosition(10, error);
        assert(pose.integrity(sketch).empty() == (error <= 1e-4));
    }
    pose.restore(sketch);
    sketch.setEntityReferenceLocked(arc_id, true);
    const sk::SketchPose locked(sketch);
    arc->radius() += 1e-12;
    assert(!locked.integrity(sketch).empty());
    locked.restore(sketch);
    assert(arc->radius() == 10 && locked.integrity(sketch).empty());
}

void classification_keeps_numerical_failure_separate() {
    sk::Sketch sketch;
    const auto unresolved = sk::classifyExact(sk::checkResiduals(sketch), false, 0, false, {});
    assert(!unresolved.state && unresolved.conflicting.empty());
    const auto under = sk::classifyExact({}, true, 2, false, {});
    assert(under.state == sk::ExactState::UnderConstrained && under.admissible());
    const auto over = sk::classifyExact({}, true, 0, true, {});
    assert(over.state == sk::ExactState::OverConstrained && over.admissible());
    sk::ResidualCheck residuals;
    residuals.violations.push_back({"near", sk::ResidualDimension::Length, .001, 1e-4, 10, "constraint"});
    const auto conflict = sk::classifyExact(residuals, true, 3, true, {"ten", "near"});
    assert(conflict.state == sk::ExactState::Conflicting && conflict.publishable() && !conflict.admissible());
    assert(conflict.conflicting == std::vector<sk::ConstraintID>({"near", "ten"}));
    assert(conflict.reasons.at("near") == sk::ConflictReason::ResidualViolation);
    assert(conflict.reasons.at("ten") == sk::ConflictReason::RankConflict);
    const auto incompatible_rank = sk::classifyExact({}, false, -2, false, {"fixedA", "fixedB"});
    assert(incompatible_rank.state == sk::ExactState::Conflicting && incompatible_rank.publishable());
    assert(incompatible_rank.dof == 0);
    assert(!sk::classifyExact({}, false, -1, false, {}).publishable());
}

void correctable_nonparallel_input_is_not_a_preflight_failure() {
    sk::Sketch sketch;
    const auto a = sketch.addPoint(0, 0), b = sketch.addPoint(10, 0);
    const auto c = sketch.addPoint(0, 10), d = sketch.addPoint(10, 11);
    const auto first = sketch.addLine(a, b), second = sketch.addLine(c, d);
    auto distance = std::make_unique<cs::DistanceConstraint>(first, second, 10);
    assert(!std::isfinite(distance->getError(sketch)));
    assert(!sketch.addConstraint(std::move(distance)).empty());
    assert(!sketch.addParallel(first, second).empty());
    assert(sk::checkResidualOperands(sketch).unassessed.empty());
    const auto solved = sketch.solve();
    assert(solved.success && solved.truth.admissible());
}

void fresh_redundancy_diagnosis_preserves_corrected_rectangle() {
    sk::Sketch sketch;
    const auto a = sketch.addPoint(3, 5), b = sketch.addPoint(83, 6);
    const auto c = sketch.addPoint(82, 65), d = sketch.addPoint(2, 64);
    const auto bottom = sketch.addLine(a, b), right = sketch.addLine(b, c);
    const auto top = sketch.addLine(c, d), left = sketch.addLine(d, a);
    sketch.addFixed(a);
    sketch.addHorizontal(bottom); sketch.addHorizontal(top);
    sketch.addVertical(left); sketch.addVertical(right);
    sketch.addConstraint(std::make_unique<cs::DistanceConstraint>(a, b, 80));
    sketch.addConstraint(std::make_unique<cs::DistanceConstraint>(b, c, 60));
    sketch.addPerpendicular(bottom, right); sketch.addPerpendicular(top, left);
    const auto solved = sketch.solve();
    assert(solved.success && solved.truth.admissible());
    assert(solved.truth.state == sk::ExactState::OverConstrained);
    assert(sketch.getEntityAs<sk::SketchPoint>(b)->y() == 5);
    assert(std::abs(sketch.getEntityAs<sk::SketchPoint>(b)->x() - 83) < 1e-4);
    assert(std::abs(sketch.getEntityAs<sk::SketchPoint>(c)->y() - 65) < 1e-4);
    const sk::SketchPose retained(sketch);
    const auto repeated = sketch.solve();
    assert(repeated.success && repeated.truth.admissible());
    assert(retained.integrity(sketch).empty());
    sketch.addEllipse(a, 10, 5);
    const auto unsupported_entity = sketch.solve();
    assert(unsupported_entity.success && unsupported_entity.truth.admissible());
    assert(!unsupported_entity.truth.redundant);
}

void explicit_fixed_targets_are_exact_only_after_solve() {
    for (const double initial : {0.0, 1e-12, 1e-5}) {
        sk::Sketch sketch;
        const auto point = sketch.addPoint(initial, 0);
        sketch.addConstraint(std::make_unique<cs::FixedConstraint>(point, 5e-11, 0));
        const sk::SketchPose before(sketch);
        assert(before.integrity(sketch, false).empty());
        const auto result = sketch.solve();
        const auto* retained = sketch.getEntityAs<sk::SketchPoint>(point);
        assert(retained->x() == 5e-11 && retained->y() == 0);
        assert(result.success && result.truth.admissible());
    }
    sk::Sketch authored;
    const auto point = authored.addPoint(3, 5);
    authored.addFixed(point);
    assert(authored.solve().truth.admissible());
    authored.addConstraint(std::make_unique<cs::FixedConstraint>(point, 3 + 5e-11, 5));
    const auto contradictory = authored.solve();
    assert(!contradictory.success && !contradictory.truth.publishable());
    assert(authored.getEntityAs<sk::SketchPoint>(point)->x() == 3);
    sk::Sketch locked;
    const auto locked_point = locked.addPoint(0, 0);
    locked.getEntityAs<sk::SketchPoint>(locked_point)->setReferenceLocked(true);
    locked.addConstraint(std::make_unique<cs::FixedConstraint>(locked_point, 5e-11, 0));
    const auto refused = locked.solve();
    assert(!refused.success && !refused.truth.publishable());
    assert(locked.getEntityAs<sk::SketchPoint>(locked_point)->x() == 0);
}

void negative_solved_radius_restores_complete_pose() {
    sk::Sketch sketch;
    const auto center = sketch.addPoint(3, 5);
    const auto circle = sketch.addCircle(center, 4);
    sketch.addConstraint(std::make_unique<cs::RadiusConstraint>(circle, -1));
    const auto result = sketch.solve();
    assert(!result.success && !result.truth.publishable());
    assert(result.truth.issues[0].reason == "internalCouplingFailure");
    assert(sketch.getEntityAs<sk::SketchCircle>(circle)->radius() == 4);
    assert(sketch.getEntityAs<sk::SketchPoint>(center)->x() == 3);
    assert(sketch.getEntityAs<sk::SketchPoint>(center)->y() == 5);
}

void immutable_reference_normalization_preserves_the_free_candidate() {
    sk::Sketch sketch;
    const auto center = sketch.addPoint(0, 0), start = sketch.addPoint(8, 0);
    const auto end = sketch.addPoint(-8, 8 * std::sin(std::numbers::pi));
    const auto free = sketch.addPoint(30, 30);
    const auto arc_id = sketch.addArc(center, 8, 0, std::numbers::pi);
    auto* arc = sketch.getEntityAs<sk::SketchArc>(arc_id);
    arc->setEndpointPointIds(start, end);
    for (const auto& id : {arc_id, start, end}) sketch.setEntityReferenceLocked(id, true);
    sketch.addCoincident(free, start);
    const sk::SketchPose before(sketch);
    auto* endpoint = sketch.getEntityAs<sk::SketchPoint>(end);
    endpoint->setPosition(-8, 9.7144514654701197e-16);
    arc->startAngle() = 6.9388939039072161e-18;
    sketch.getEntityAs<sk::SketchPoint>(free)->setPosition(8, 0);
    assert(!before.assessCandidate(sketch, false).empty());
    assert(endpoint->y() == 9.7144514654701197e-16);
    assert(arc->startAngle() == 6.9388939039072161e-18);
    assert(before.assessCandidate(sketch, true).empty());
    assert(endpoint->y() == before.points.at(end)[1]);
    assert(arc->radius() == 8 && arc->startAngle() == 0 && arc->endAngle() == std::numbers::pi);
    assert(sketch.getEntityAs<sk::SketchPoint>(free)->x() == 8);
    assert(sketch.getEntityAs<sk::SketchPoint>(free)->y() == 0);
    assert(sk::checkResiduals(sketch).violations.empty());
    sketch.addConstraint(std::make_unique<cs::FixedConstraint>(end, -8, 1));
    assert(!before.assessCandidate(sketch, true).empty());
    assert(endpoint->y() == before.points.at(end)[1]);
}

void reference_normalization_cannot_mask_nonfinite_or_negative_geometry() {
    sk::Sketch sketch;
    const auto center = sketch.addPoint(0, 0);
    const auto circle_id = sketch.addCircle(center, 8);
    sketch.setEntityReferenceLocked(circle_id, true);
    const sk::SketchPose before(sketch);
    auto* circle = sketch.getEntityAs<sk::SketchCircle>(circle_id);
    for (const double radius : {-1.0, std::numeric_limits<double>::infinity(),
                                std::numeric_limits<double>::quiet_NaN()}) {
        circle->radius() = radius;
        assert(!before.assessCandidate(sketch, true).empty());
        assert(circle->radius() == radius || std::isnan(circle->radius()));
        before.restore(sketch);
        assert(circle->radius() == 8 && before.integrity(sketch).empty());
    }
}

int main() {
    immutable_reference_normalization_preserves_the_free_candidate();
    reference_normalization_cannot_mask_nonfinite_or_negative_geometry();
    explicit_fixed_targets_are_exact_only_after_solve();
    negative_solved_radius_restores_complete_pose();
    fresh_redundancy_diagnosis_preserves_corrected_rectangle();
    length_boundary_is_inclusive();
    angular_budget_is_local();
    all_twenty_kinds_have_a_typed_reader();
    undefined_and_nonfinite_operands_are_unassessed();
    missing_owned_points_fail_even_without_public_constraints();
    welded_tangent_requires_both_length_and_angle();
    welded_arc_arc_tangency_uses_the_larger_radius();
    proximity_is_not_a_weld();
    coupling_and_reference_locks_are_integrity_not_public_blame();
    classification_keeps_numerical_failure_separate();
    correctable_nonparallel_input_is_not_a_preflight_failure();
}
