#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <limits>

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <gp_Pnt.hxx>
#include <BRepBndLib.hxx>
#include <Bnd_Box.hxx>
#include <BRepTools.hxx>
#include <TopoDS.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp_Explorer.hxx>

#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "ops/SweepOp.h"
#include "ops/OpCommon.h"
#include "protocol/Envelope.h"
#include "session/HistoryHash.h"
#include "session/PlanExecutor.h"
#include "session/PreviewOp.h"
#include "session/ShapeMetrics.h"
#include "sketch/WireSketch.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace session = onecad::session;
namespace {
int failures = 0;
void check(bool ok, const char* text) {
    if (!ok) { std::fprintf(stderr, "FAIL: %s\n", text); ++failures; }
}

std::pair<std::array<double, 3>, std::array<double, 3>> exact_bounds(const TopoDS_Shape& shape) {
    Bnd_Box box;
    BRepBndLib::AddOptimal(shape, box, false, false);
    double xmin, ymin, zmin, xmax, ymax, zmax;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    return {{{xmin, ymin, zmin}}, {{xmax, ymax, zmax}}};
}

void check_near(double actual, double expected, double tolerance, const char* label) {
    if (std::abs(actual - expected) > tolerance) {
        std::fprintf(stderr, "FAIL: %s actual=%.17g expected=%.17g tolerance=%.17g\n",
                     label, actual, expected, tolerance);
        ++failures;
    }
}

void check_support(const TopoDS_Shape& shape, const std::array<double, 3>& center,
                   const std::array<double, 3>& major, const std::array<double, 3>& minor,
                   double a, double b, double radius) {
    constexpr double kDistanceTolerance = 2e-6;
    const auto bounds = session::compute_shape_metrics(shape);
    for (int axis = 0; axis < 3; ++axis) {
        const double path_extent = std::hypot(a * major[axis], b * minor[axis]);
        for (int sign : {-1, 1}) {
            std::array<double, 3> point = center;
            for (int coordinate = 0; coordinate < 3; ++coordinate) {
                point[coordinate] += path_extent > 0
                    ? sign * (a*a*major[axis]*major[coordinate] + b*b*minor[axis]*minor[coordinate]) / path_extent
                    : a * major[coordinate];
            }
            point[axis] += sign * radius;
            const auto vertex = BRepBuilderAPI_MakeVertex(gp_Pnt(point[0], point[1], point[2])).Vertex();
            BRepExtrema_DistShapeShape distance(vertex, shape);
            distance.Perform();
            check(distance.IsDone(), "support-point distance computed");
            if (distance.IsDone()) check_near(distance.Value(), 0, kDistanceTolerance, "analytic support point on solid");
            if (sign < 0) check(bounds.bbox_min[axis] <= point[axis] + kDistanceTolerance,
                                "production bbox encloses minimum support");
            else check(bounds.bbox_max[axis] >= point[axis] - kDistanceTolerance,
                       "production bbox encloses maximum support");
        }
    }
}

json curve_sketch(double a, double b, bool ellipse = true) {
    json curve = ellipse ? json{{"id", "curve"}, {"type", "Ellipse"}, {"center", {0, 0}},
                                {"majorR", a}, {"minorR", b}, {"rotation", 0.0}}
                         : json{{"id", "curve"}, {"type", "Circle"}, {"center", {0, 0}},
                                {"radius", a}};
    return {{"plane", {{"kind", "custom"}, {"origin", {0, 0, 0}}, {"xAxis", {1, 0, 0}},
                       {"yAxis", {0, 1, 0}}, {"normal", {0, 0, 1}}}},
            {"entities", json::array({curve})}, {"constraints", json::array()}};
}

json section_sketch(double a, double radius) {
    return {{"plane", {{"kind", "custom"}, {"origin", {a, 0, 0}}, {"xAxis", {0, 0, 1}},
                       {"yAxis", {1, 0, 0}}, {"normal", {0, 1, 0}}}},
            {"entities", json::array({{{"id", "section"}, {"type", "Circle"},
                                       {"center", {0, 0}}, {"radius", radius}}})},
            {"constraints", json::array()}};
}

std::string region_id(const json& params, int version) {
    auto translated = onecad::wire::translate(params);
    check(translated.ok, "profile fixture translates");
    if (!translated.ok || !translated.sketch) return {};
    const auto solved = translated.sketch->solve();
    check(solved.truth.admissible(), "profile fixture solves");
    onecad::core::loop::LoopDetector detector;
    auto config = onecad::core::loop::makeRegionDetectionConfig();
    config.exactAnalyticFragments = true;
    if (version == 3) config.curveRefinementPolicy = onecad::core::loop::CurveRefinementPolicy::V3PhysicalProximity;
    detector.setConfig(config);
    const auto detected = detector.detect(*translated.sketch);
    const auto table = onecad::core::loop::buildRegionTable(detected, [&](const auto& edge) {
        return translated.index.internal_edge_to_wire.at(edge);
    }, onecad::core::sketch::constants::COINCIDENCE_TOLERANCE,
       version == 3 ? onecad::core::loop::RegionIdentityVersion::V3 : onecad::core::loop::RegionIdentityVersion::V2);
    check(table.success && table.regions.size() == 1, "profile fixture has one region");
    if (!table.success || table.regions.size() != 1) return {};
    return table.regions.front().id;
}

struct Fixture {
    session::BodyStore bodies;
    onecad::elementmap::ElementMapPartition partition;
    std::vector<std::pair<std::string, json>> sketches;
    std::string last;
    onecad::CancelToken cancel;
    json op;

    Fixture(double a, double b, double radius, bool ellipse = true, int version = 2) {
        auto profile = section_sketch(a, radius);
        op = {{"opType", "Sweep"}, {"opId", "sweep"},
              {"params", {{"sketchId", "profile"}, {"regionId", region_id(profile, version)},
                          {"regionIdentityVersion", version}, {"pathSketchId", "path"},
                          {"booleanMode", "NewBody"}}}};
        sketches.emplace_back("profile", std::move(profile));
        sketches.emplace_back("path", curve_sketch(a, b, ellipse));
    }
    ops::OpOutcome run() {
        ops::OpContext ctx{bodies, &sketches, partition, &last, false, json::object(), &cancel};
        return ops::execute_sweep(ctx, op, "sweep");
    }
};

void set_standard_xy(Fixture& fixture, double major) {
    fixture.sketches[1].second["plane"] = {{"kind", "XY"}, {"origin", {0,0,0}},
        {"xAxis", {0,1,0}}, {"yAxis", {-1,0,0}}, {"normal", {0,0,1}}};
    auto& profile = fixture.sketches[0].second["plane"];
    profile["origin"] = {0,major,0};
    profile["xAxis"] = {0,0,1}; profile["yAxis"] = {0,1,0}; profile["normal"] = {-1,0,0};
}

double ellipse_length(double a, double b) {
    constexpr int steps = 4096;
    const double h = 0.5 * std::acos(-1.0) / steps;
    double sum = 0;
    for (int i = 0; i <= steps; ++i) {
        const double angle = h * i;
        const double x = a * std::sin(angle), y = b * std::cos(angle);
        sum += (i == 0 || i == steps ? 1 : (i % 2 ? 4 : 2)) * std::hypot(x, y);
    }
    return 4.0 * sum * h / 3.0;
}

void accepted(double a, double b, double radius, bool ellipse, int version) {
    Fixture fixture(a, b, radius, ellipse, version);
    const auto out = fixture.run();
    check(out.status == ops::OpOutcome::Status::Ok, out.error_message.c_str());
    const auto* body = fixture.bodies.get("body_sweep");
    check(body != nullptr, "successful sweep creates body");
    if (!body) return;
    const auto metric = session::compute_shape_metrics(body->geom);
    const double pi = std::acos(-1.0);
    const double expected = ellipse ? pi * radius * radius * ellipse_length(a, b)
                                    : 2.0 * pi * pi * a * radius * radius;
    // The tolerance follows the 1e-7 mm construction bound and the model scale;
    // the independent Simpson path integration uses 4096 intervals per quadrant.
    const double volume_budget = 128.0 * (a + radius) * (a + radius) * 1e-7;
    check(std::abs(metric.volume - expected) <= volume_budget, "sweep volume vs analytic path length");
    check_support(body->geom, {0,0,0}, {1,0,0}, {0,1,0}, a, b, radius);
    check(out.body_events.size() == 1 && out.body_events[0].kind == "created", "created body event");
    check(out.topology_history.size() == 1 && out.topology_history[0].survivors.empty() &&
          out.topology_history[0].successors.empty() && !out.topology_history[0].births.empty(),
          "new body history has births only");
}

void circle_seam_and_alternate_start() {
    auto path = curve_sketch(4, 4, false);
    path["plane"] = {{"kind", "XY"}, {"origin", {0,0,0}}, {"xAxis", {0,1,0}},
                     {"yAxis", {-1,0,0}}, {"normal", {0,0,1}}};
    std::string error;
    const auto face = ops::build_profile_face(path, "", std::nullopt, error);
    check(face.has_value(), "production XY path face built");
    if (face) {
        const auto wire = BRepTools::OuterWire(*face);
        TopExp_Explorer explorer(wire, TopAbs_EDGE);
        check(explorer.More(), "production XY path edge exists");
        if (explorer.More()) {
            BRepAdaptor_Curve curve(TopoDS::Edge(explorer.Current()));
            gp_Pnt start; gp_Vec tangent;
            curve.D1(curve.FirstParameter(), start, tangent);
            std::fprintf(stderr, "production XY circle initial seam: (%.17g,%.17g,%.17g) tangent=(%.17g,%.17g,%.17g)\n",
                         start.X(), start.Y(), start.Z(), tangent.X(), tangent.Y(), tangent.Z());
            check(start.Distance(gp_Pnt(4,0,0)) < 1e-9, "production circle actual seam is +X");
        }
    }
    Fixture fixture(4,4,.5,false);
    auto& plane = fixture.sketches[0].second["plane"];
    plane["origin"] = {0,4,0};
    plane["xAxis"] = {0,0,1}; plane["yAxis"] = {0,1,0}; plane["normal"] = {-1,0,0};
    const auto out = fixture.run();
    check(out.status == ops::OpOutcome::Status::Ok, "authored alternate circle seam accepted");
    const auto* body = fixture.bodies.get("body_sweep");
    if (body) {
        check_support(body->geom, {0,0,0}, {1,0,0}, {0,1,0}, 4, 4, .5);
        const double expected = 2*std::acos(-1.0)*std::acos(-1.0)*4*.5*.5;
        check_near(session::shape_volume(body->geom), expected, 128.0*4.5*4.5*1e-7,
                   "alternate seam analytic volume");
    } else check(false, "alternate circle seam creates body");
}

void production_standard_planes() {
    constexpr double a = 4, b = 2, radius = .2;
    const std::array<std::string, 3> kinds = {"XY", "XZ", "YZ"};
    const std::array<std::array<double, 3>, 3> major = {{{0,1,0},{0,1,0},{-1,0,0}}};
    const std::array<std::array<double, 3>, 3> minor = {{{-1,0,0},{0,0,1},{0,0,1}}};
    const std::array<std::array<double, 3>, 3> normal = {{{0,0,1},{1,0,0},{0,1,0}}};
    for (std::size_t k = 0; k < kinds.size(); ++k) {
        Fixture fixture(a, b, radius, true, 3);
        fixture.sketches[1].second["plane"] = {{"kind", kinds[k]}, {"origin", {0,0,0}},
            {"xAxis", major[k]}, {"yAxis", minor[k]}, {"normal", normal[k]}};
        auto& profile = fixture.sketches[0].second["plane"];
        profile["origin"] = {a*major[k][0], a*major[k][1], a*major[k][2]};
        if (k == 0) {
            profile["xAxis"] = {0,0,1}; profile["yAxis"] = {0,1,0}; profile["normal"] = {-1,0,0};
        } else {
            profile["xAxis"] = {1,0,0}; profile["yAxis"] = {0,1,0}; profile["normal"] = {0,0,1};
        }
        const auto out = fixture.run();
        check(out.status == ops::OpOutcome::Status::Ok, "production wire standard plane accepted");
        const auto* body = fixture.bodies.get("body_sweep");
        if (!body) { check(false, "standard plane body exists"); continue; }
        const auto metrics = session::compute_shape_metrics(body->geom);
        const auto [minimum, maximum] = exact_bounds(body->geom);
        if (const char* directory = std::getenv("ONECAD_SWEEP_DEBUG_DIR")) {
            const std::string path = std::string(directory) + "/sweep-standard-" + std::to_string(k) + ".brep";
            check(BRepTools::Write(body->geom, path.c_str()), "diagnostic BRep written");
        }
        std::fprintf(stderr, "standard %s bounds [%g,%g,%g] [%g,%g,%g]\n", kinds[k].c_str(),
                     minimum[0], minimum[1], minimum[2], maximum[0], maximum[1], maximum[2]);
        const double pi = std::acos(-1.0);
        const double volume = pi * radius * radius * ellipse_length(a, b);
        check(std::abs(metrics.volume-volume) <= 128.0*(a+radius)*(a+radius)*1e-7,
              "standard plane analytic tube volume");
        check_support(body->geom, {0,0,0}, major[k], minor[k], a, b, radius);
    }
}

void opposite_profile_normal() {
    Fixture fixture(4, 4, .5, false);
    auto& plane = fixture.sketches[0].second["plane"];
    plane["yAxis"] = {-1,0,0};
    plane["normal"] = {0,-1,0};
    const auto out = fixture.run();
    check(out.status == ops::OpOutcome::Status::Ok, "opposite profile normal accepted");
    const auto* body = fixture.bodies.get("body_sweep");
    check(body && BRepCheck_Analyzer(body->geom).IsValid() && session::shape_volume(body->geom) > 0,
          "opposite normal yields valid outward solid");
}

void anchor_rebinding() {
    Fixture fixture(4, 4, .5, false, 3);
    fixture.op["params"]["regionId"] = "stale";
    fixture.op["params"]["regionAnchor"] = {0, 0};
    const auto out = fixture.run();
    check(out.status == ops::OpOutcome::Status::Ok, "profile anchor resolves unique region");
    check(std::any_of(out.diagnostics.begin(), out.diagnostics.end(), [](const auto& d) {
        return d.value("code", std::string{}) == "REGION_REBOUND_BY_ANCHOR";
    }), "profile anchor rebind is diagnosed");
}

void rotated_axis_frame() {
    constexpr double a = 4.0, radius = .5;
    Fixture fixture(a, a, radius, false);
    for (auto& [id, sketch] : fixture.sketches) {
        auto& plane = sketch["plane"];
        for (const char* key : {"origin", "xAxis", "yAxis", "normal"}) {
            const auto v = plane[key];
            const double x = v[0].get<double>(), y = v[1].get<double>(), z = v[2].get<double>();
            plane[key] = {x, -z, y};
        }
        plane["origin"][0] = plane["origin"][0].get<double>() + 5.0;
        plane["origin"][1] = plane["origin"][1].get<double>() - 3.0;
        plane["origin"][2] = plane["origin"][2].get<double>() + 2.0;
    }
    const auto out = fixture.run();
    if (out.status != ops::OpOutcome::Status::Ok)
        std::fprintf(stderr, "rotated sweep: %s diagnostics=%s\n", out.error_message.c_str(), json(out.diagnostics).dump().c_str());
    check(out.status == ops::OpOutcome::Status::Ok, "rotated translated sketch frames accepted");
    const auto* body = fixture.bodies.get("body_sweep");
    if (!body) { check(false, "rotated sweep body exists"); return; }
    check_support(body->geom, {5,-3,2}, {1,0,0}, {0,0,1}, a, a, radius);
}

void refusals() {
    auto refused = [](Fixture& fixture, const char* label) {
        const TopoDS_Shape existing = BRepPrimAPI_MakeBox(2, 3, 4).Shape();
        fixture.bodies.create("body_existing", "existing", existing);
        const auto out = fixture.run();
        check(out.status != ops::OpOutcome::Status::Ok, label);
        check(fixture.bodies.all().size() == 1 && fixture.bodies.get("body_existing")->geom.IsEqual(existing),
              "refused sweep preserves prior body");
        check(out.body_events.empty() && out.topology_history.empty(), "refusal has no published event or history");
    };
    { Fixture f(4,4,.5,false); f.op["params"]["pathEdgeId"] = "edge"; refused(f,"path edge refused"); }
    { Fixture f(4,4,.5,false); f.op["params"]["booleanMode"] = "Cut"; refused(f,"boolean refused"); }
    { Fixture f(4,4,.5,false); f.op["params"]["twist"] = 1; refused(f,"twist refused"); }
    { Fixture f(4,4,.5,false); f.op["inputs"] = json::array({{{"kind", "face"}}}); refused(f,"semantic input refused"); }
    { Fixture f(4,4,.5,false); f.sketches[1].second["plane"]["xAxis"] = {2,0,0}; refused(f,"nonrigid plane refused"); }
    { Fixture f(4,4,.5,false); f.sketches[1].second["plane"] = {{"kind","XY"},{"origin",{1,0,0}},
        {"xAxis",{0,1,0}},{"yAxis",{-1,0,0}},{"normal",{0,0,1}}};
      refused(f,"standard plane with ignored nonzero origin refused"); }
    { Fixture f(4,4,.5,false); f.sketches[0].second["entities"].push_back({{"id","hole"},{"type","Circle"},{"center",{.1,0}},{"radius",.1}}); refused(f,"profile hole refused"); }
    { Fixture f(4,4,.5,false); f.op["params"]["regionIdentityVersion"] = 1; refused(f,"legacy profile refused"); }
    { Fixture f(4,4,.5,false); f.op["params"]["regionAnchor"] = {0,"bad"}; refused(f,"malformed region anchor refused"); }
    { Fixture f(4,4,.5,false); f.op["params"]["regionId"] = "stale"; refused(f,"missing exact region refused"); }
    { Fixture f(4,4,.5,false); f.sketches[0].second["plane"]["origin"][0] = 4.1; refused(f,"off-path profile refused"); }
    { Fixture f(4,4,.5,false); auto& p = f.sketches[0].second["plane"];
      p["xAxis"] = {1,0,0}; p["yAxis"] = {0,1,0}; p["normal"] = {0,0,1};
      refused(f,"rigid profile normal not parallel to tangent refused"); }
    { Fixture f(1.08166538263,.6,.34); refused(f,"curvature overlap refused"); }
    { Fixture f(1.08166538263,.6,.3); auto& p = f.sketches[0].second["plane"];
      p["origin"] = {0,.6,0}; p["xAxis"] = {0,0,1}; p["yAxis"] = {0,1,0}; p["normal"] = {-1,0,0};
      const auto out = f.run();
      check(out.status == ops::OpOutcome::Status::Unsupported && out.error_code == "UNSUPPORTED" &&
            out.error_message.find("positive major-axis") != std::string::npos,
            "non-major ellipse seam explicitly unsupported");
      check(f.bodies.all().empty(), "unsupported ellipse seam publishes no body"); }
    { Fixture f(4,4,.5,false); f.sketches[1].second["entities"][0]["radius"] = 0.0; refused(f,"zero path radius refused"); }
    { Fixture f(4,4,.5,false); f.sketches[1].second["entities"][0]["construction"] = true; refused(f,"construction path refused"); }
    { Fixture f(4,4,.5,false); f.sketches[1].second["entities"].push_back({{"id","line"},{"type","Line"},{"p0",{0,0}},{"p1",{1,1}}}); refused(f,"open path segment refused"); }
    { Fixture f(4,4,.5,false); f.sketches[1].second["entities"].push_back(f.sketches[1].second["entities"][0]); refused(f,"multiple paths refused"); }
    { Fixture f(4,4,.5,false); f.sketches[1].second["entities"][0]["radius"] = std::numeric_limits<double>::infinity(); refused(f,"nonfinite path refused"); }
    { Fixture f(4,4,.5,false); f.cancel.cancel(); const auto out = f.run();
      check(out.status == ops::OpOutcome::Status::Cancelled && f.bodies.all().empty(), "cancellation publishes nothing"); }
}
void transaction_fences() {
    Fixture fixture(4, 2, .2, true);
    set_standard_xy(fixture, 4);
    json operations = json::array();
    int index = 0;
    for (const auto& [id, data] : fixture.sketches) {
        auto params = data;
        params["sketchId"] = id;
        operations.push_back({{"opType", "Sketch"}, {"opId", "sketch" + std::to_string(index)},
                              {"stepIndex", index++}, {"params", params}});
    }
    auto operation = fixture.op;
    operation["stepIndex"] = index;
    operations.push_back(operation);
    json args = {{"jobId", 1}, {"documentRevision", 1}, {"workerEpoch", 3},
                 {"expectedBaseHash", session::kEmptyPrefixHash}, {"prefixHashes", {"a", "b", "c"}},
                 {"targetStep", index}, {"ops", operations}};
    session::Session state;
    state.open("sweep", 0, 3, "determinism");
    onecad::CancelToken cancel;
    onecad::protocol::HandlerContext context{cancel, [](int) {}, [](onecad::protocol::Envelope&) {}, {}};
    const auto prepared = session::handle_execute_plan(state,
        onecad::protocol::Envelope::request(1, "ExecutePlan", args), context);
    check(prepared.ok.value_or(false) && prepared.result["lastValidStep"] == 2, "sweep plan prepared");
    check(state.bodies_copy().all().empty(), "prepared sweep does not mutate head");
    auto rejected = session::handle_accept_prepared(state, onecad::protocol::Envelope::request(2,
        "AcceptPrepared", {{"jobId", 1}, {"documentRevision", 1}, {"workerEpoch", 4}}));
    check(rejected.error.has_value(), "stale sweep epoch refused");
    auto accepted = session::handle_accept_prepared(state, onecad::protocol::Envelope::request(3,
        "AcceptPrepared", {{"jobId", 1}, {"documentRevision", 1}, {"workerEpoch", 3}}));
    check(accepted.ok.value_or(false) && state.bodies_copy().contains("body_sweep"), "accepted sweep published");
}

void previews() {
    Fixture fixture(4, 2, .2, true);
    set_standard_xy(fixture, 4);
    session::Session state;
    state.open("sweep-preview", 0, 3, "determinism");
    for (const auto& [id, params] : fixture.sketches) state.sketches().upsert(id, params);
    const auto request = [&](const json& op, std::uint64_t snapshot) {
        return onecad::protocol::Envelope::request(1, "PreviewOp", {{"op", op}, {"expectedSnapshotId", snapshot}});
    };
    auto result = session::handle_preview_op(state, request(fixture.op, 0));
    check(result.ok.value_or(false), "preview loads profile and path from session");
    if (result.ok.value_or(false)) {
        check(result.result["changedBodies"] == json::array({"body_sweep"}), "preview includes sweep body");
        check(result.result["meshes"].size() == 1, "preview includes checked mesh");
    }
    auto missing = fixture.op;
    missing["params"]["pathSketchId"] = "missing";
    result = session::handle_preview_op(state, request(missing, 0));
    check(result.error.has_value() && result.error->code == "REF_UNRESOLVED", "preview missing path refused");
    auto invalid = fixture.op;
    invalid["params"]["pathEdgeId"] = "edge";
    result = session::handle_preview_op(state, request(invalid, 0));
    check(result.error.has_value() && result.error->code == "UNSUPPORTED", "preview path edge refused");
    result = session::handle_preview_op(state, request(fixture.op, 1));
    check(result.error.has_value() && result.error->code == "STALE_PREVIEW", "preview stale snapshot refused");
    onecad::CancelToken cancel;
    cancel.cancel();
    result = session::handle_preview_op(state, request(fixture.op, 0), cancel);
    check(result.error.has_value() && result.error->code == "CANCELLED", "cancelled preview refused");
    check(state.current_snapshot_id() == 0 && state.bodies_copy().all().empty(), "preview never mutates head");
}

}  // namespace

int main() {
    accepted(4, 4, .5, false, 2);
    accepted(1.08166538263, .6, .3, true, 3);
    rotated_axis_frame();
    production_standard_planes();
    circle_seam_and_alternate_start();
    opposite_profile_normal();
    anchor_rebinding();
    refusals();
    transaction_fences();
    previews();
    return failures;
}
