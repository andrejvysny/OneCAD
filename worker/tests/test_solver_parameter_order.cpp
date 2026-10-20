// ONECAD-305: storage layout must not select a different valid free-Arc pose.
#include <Constraints.h>
#include <GCS.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <utility>
#include <vector>

namespace {
constexpr int kParameters = 13;
using Pose = std::array<double, kParameters>;

int failures = 0;

void check(bool condition, const char* message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        ++failures;
    }
}

struct Layout {
    Pose storage{};
    std::array<double*, kParameters> semantic{};

    explicit Layout(const std::array<int, 4>& blocks) {
        int slot = 0;
        for (int block : blocks) {
            const int first = block == 3 ? 6 : 2 * block;
            const int count = block == 3 ? 3 : 2;
            for (int j = 0; j < count; ++j) semantic[first + j] = &storage[slot++];
        }
        for (int i = 9; i < kParameters; ++i) semantic[i] = &storage[i];
    }

    void reset() {
        // The end's derived X differs from the placed construction point's X.
        // Their structural weld must not make pointer order choose an alias.
        const Pose initial{0., 0., 10., 0., 10. * std::cos(M_PI / 2.), 10., 10.,
                           0., M_PI / 2., 10., 0., 0., 10.};
        for (int i = 0; i < kParameters; ++i) *semantic[i] = initial[i];
    }

    Pose pose() const {
        Pose value{};
        for (int i = 0; i < kParameters; ++i) value[i] = *semantic[i];
        return value;
    }
};

struct Solved {
    Pose pose;
    int status;
    int dof;
    double residual;

    bool operator==(const Solved&) const = default;
};

GCS::Arc arc_of(const Layout& layout) {
    const auto& p = layout.semantic;
    GCS::Arc arc;
    arc.center = GCS::Point(p[0], p[1]);
    arc.start = GCS::Point(p[2], p[3]);
    arc.end = GCS::Point(p[4], p[5]);
    arc.rad = p[6];
    arc.startAngle = p[7];
    arc.endAngle = p[8];
    return arc;
}

std::vector<GCS::Constraint*> constraints_for(GCS::Arc& arc, double* radius) {
    return {
        new GCS::ConstraintCurveValue(arc.start, arc.start.x, arc, arc.startAngle),
        new GCS::ConstraintCurveValue(arc.start, arc.start.y, arc, arc.startAngle),
        new GCS::ConstraintCurveValue(arc.end, arc.end.x, arc, arc.endAngle),
        new GCS::ConstraintCurveValue(arc.end, arc.end.y, arc, arc.endAngle),
        new GCS::ConstraintEqual(arc.rad, radius),
    };
}

void add_control(std::vector<GCS::Constraint*>& constraints, Layout& layout,
                 double* zero_x, double* zero_y) {
    const auto& p = layout.semantic;
    for (auto pair : {std::pair{9, 2}, std::pair{10, 3}, std::pair{11, 4}, std::pair{12, 5},
                      std::pair{1, 10}, std::pair{0, 11}}) {
        constraints.push_back(new GCS::ConstraintEqual(p[pair.first], p[pair.second]));
    }
    constraints.push_back(new GCS::ConstraintEqual(p[0], zero_x));
    constraints.push_back(new GCS::ConstraintEqual(p[1], zero_y));
}

GCS::VEC_pD declared(const Layout& layout, bool control) {
    const auto& p = layout.semantic;
    if (!control) return GCS::VEC_pD(p.begin(), p.begin() + 9);
    GCS::VEC_pD parameters;
    for (int index : {0, 1, 9, 10, 11, 12, 2, 3, 4, 5, 6, 7, 8}) {
        parameters.push_back(p[index]);
    }
    return parameters;
}

void configure(GCS::System& system) {
    system.debugMode = GCS::NoDebug;
    system.setConvergence(1e-4);
    system.setConvergenceRedundant(1e-4);
    system.setMaxIterations(100);
    system.setMaxIterationsRedundant(100);
}

Solved solve(Layout& layout, double radius, bool control, bool auxiliary) {
    GCS::Arc arc = arc_of(layout);
    double zero_x = 0., zero_y = 0., target_x = 1., target_y = 15.;
    auto constraints = constraints_for(arc, &radius);
    if (control) add_control(constraints, layout, &zero_x, &zero_y);
    GCS::System system;
    configure(system);
    for (std::size_t index = 0; index < constraints.size(); ++index) {
        constraints[index]->setTag(index < 4 ? 0 : static_cast<int>(index) - 3);
        constraints[index]->setDriving(true);
        system.addConstraint(constraints[index]);
    }
    if (auxiliary) {
        system.addConstraintCoordinateX(arc.end, &target_x, -1, true);
        system.addConstraintCoordinateY(arc.end, &target_y, -1, true);
    }
    auto parameters = declared(layout, control);
    system.declareUnknowns(parameters);
    GCS::VEC_pD driven;
    system.declareDrivenParams(driven);
    system.initSolution(GCS::DogLeg);
    const int status = system.solve(true, GCS::DogLeg, false);
    if (status == GCS::Success || status == GCS::Converged) system.applySolution();
    else system.undoSolution();
    double residual = 0.;
    for (auto* constraint : constraints) residual = std::max(residual, std::abs(constraint->error()));
    if (auxiliary) {
        residual = std::max({residual, std::abs(*arc.end.x - target_x),
                             std::abs(*arc.end.y - target_y)});
    }
    return {layout.pose(), status, system.dofsNumber(), residual};
}

void arc_layouts(bool control, bool auxiliary) {
    std::array<int, 4> blocks{0, 1, 2, 3};
    Solved first{};
    bool baseline = true;
    do {
        Layout layout(blocks);
        for (int repeat = 0; repeat < 3; ++repeat) {
            layout.reset();
            const auto accepted = solve(layout, 10., control, false);
            check(accepted.status == GCS::Success, "accepted radius10 solve");
            check(accepted.pose[6] == 10., "accepted radius10 retained");
            const auto edited = solve(layout, 14., control, auxiliary);
            check(edited.status == GCS::Success || edited.status == GCS::Converged,
                  "finite edit converges");
            check(edited.residual <= 1e-4, "all equations and auxiliary target satisfied");
            check(std::abs(edited.pose[6] - 14.) <= 1e-6, "radius14 satisfied");
            check(edited.dof == (control ? 0 : 4), "released structural DOF retained");
            if (baseline) { first = edited; baseline = false; }
            check(edited == first, "identical semantic input yields exact pose/residual/DOF across layouts");
        }
    } while (std::next_permutation(blocks.begin(), blocks.end()));
}

void declared_membership() {
    double storage[4]{2., 3., 5., 7.};
    for (bool reverse : {false, true}) {
        auto* first = &storage[reverse ? 3 : 0];
        auto* second = &storage[reverse ? 0 : 3];
        double constant = 0.;
        GCS::ConstraintEqual a(first, &constant), b(second, &constant);
        std::vector<GCS::Constraint*> constraints{&a, &b};
        GCS::VEC_pD parameters{second, &storage[1], first, second};
        GCS::SubSystem subsystem(constraints, parameters);
        GCS::VEC_pD columns;
        subsystem.getParamList(columns);
        check(columns == GCS::VEC_pD{second, first}, "membership preserves declared order and deduplication");
    }
}

void unequal_reduced_aliases() {
    double storage[2]{};
    for (bool reverse : {false, true}) {
        double* representative = &storage[reverse ? 1 : 0];
        double* alias = &storage[reverse ? 0 : 1];
        *representative = 2.;
        *alias = 7.;
        double target = 2.;
        GCS::ConstraintEqual a(representative, &target), b(alias, &target);
        std::vector<GCS::Constraint*> constraints{&a, &b};
        GCS::VEC_pD parameters{representative, alias};
        GCS::MAP_pD_pD reduction{{alias, representative}};
        GCS::SubSystem subsystem(constraints, parameters, reduction);
        subsystem.redirectParams();
        Eigen::VectorXd values;
        subsystem.getParams(values);
        check(values.size() == 1 && values[0] == 2., "chosen representative initializes reduced slot once");
        subsystem.applySolution();
        subsystem.revertParams();
        check(*representative == 2. && *alias == 2., "solved value publishes to every reduced alias");
    }
}
} // namespace

int main() {
    declared_membership();
    unequal_reduced_aliases();
    arc_layouts(false, false);
    arc_layouts(true, false);
    arc_layouts(false, true);
    if (failures == 0) std::fprintf(stderr, "solver parameter order: membership, aliases, 216 Arc cases, 432 solves passed\n");
    return failures == 0 ? 0 : 1;
}
