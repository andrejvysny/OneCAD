// Included inside test_sweep.cpp's fixture namespace.
void set_open_section(Fixture& fixture, double a, double b, double start, double end,
                      double angle = 0) {
    const double x = a * std::cos(start), y = b * std::sin(start);
    const double raw_dx = -a * std::sin(start), raw_dy = b * std::cos(start);
    const double dx = raw_dx * std::cos(angle) - raw_dy * std::sin(angle);
    const double dy = raw_dx * std::sin(angle) + raw_dy * std::cos(angle);
    const double length = std::hypot(dx, dy);
    auto& plane = fixture.sketches[0].second["plane"];
    plane["origin"] = {x * std::cos(angle) - y * std::sin(angle),
                       x * std::sin(angle) + y * std::cos(angle), 0};
    plane["xAxis"] = {0, 0, 1};
    plane["yAxis"] = {dy / length, -dx / length, 0};
    plane["normal"] = {dx / length, dy / length, 0};
    fixture.op["params"]["pathRange"] = {{"startRadians", start}, {"endRadians", end}};
}

double arc_length(double a, double b, double start, double end) {
    constexpr int steps = 4096;
    const double h = (end - start) / steps;
    double sum = 0;
    for (int i = 0; i <= steps; ++i) {
        const double t = start + h * i;
        const double weight = i == 0 || i == steps ? 1 : (i % 2 ? 4 : 2);
        sum += weight * std::hypot(a * std::sin(t), b * std::cos(t));
    }
    return sum * h / 3.0;
}

void check_open_caps(const TopoDS_Shape& shape, double a, double b, double start,
                     double end, bool rotated, double angle) {
    for (double t : {start, end}) {
        gp_Pnt center(a * std::cos(t), b * std::sin(t), 0);
        gp_Vec tangent(-a * std::sin(t), b * std::cos(t), 0);
        center = gp_Pnt(center.X() * std::cos(angle) - center.Y() * std::sin(angle),
                        center.X() * std::sin(angle) + center.Y() * std::cos(angle), 0);
        tangent = gp_Vec(tangent.X() * std::cos(angle) - tangent.Y() * std::sin(angle),
                         tangent.X() * std::sin(angle) + tangent.Y() * std::cos(angle), 0);
        if (rotated) {
            center = gp_Pnt(center.X() + 5, -center.Z() - 3, center.Y() + 2);
            tangent = gp_Vec(tangent.X(), -tangent.Z(), tangent.Y());
        }
        bool found = false;
        for (TopExp_Explorer e(shape, TopAbs_FACE); e.More(); e.Next()) {
            BRepAdaptor_Surface face(TopoDS::Face(e.Current()));
            if (face.GetType() != GeomAbs_Plane) continue;
            const gp_Pln plane = face.Plane();
            if (plane.Distance(center) <= 2e-6 &&
                plane.Axis().Direction().IsParallel(gp_Dir(tangent), Precision::Angular())) found = true;
        }
        check(found, "open sweep cap contains endpoint and is tangent-normal");
    }
}

void check_open_result(const Fixture& fixture, double a, double b, double radius,
                       double start, double end, bool rotated = false, double angle = 0) {
    const auto* body = fixture.bodies.get("body_sweep");
    check(body != nullptr, "open sweep creates body");
    if (!body) return;
    check(BRepCheck_Analyzer(body->geom).IsValid(), "open sweep BRep is valid");
    int solids = 0, shells = 0, caps = 0;
    for (TopExp_Explorer e(body->geom, TopAbs_SOLID); e.More(); e.Next()) ++solids;
    for (TopExp_Explorer e(body->geom, TopAbs_SHELL); e.More(); e.Next()) {
        ++shells;
        check(BRep_Tool::IsClosed(e.Current()), "open sweep shell is closed");
    }
    for (TopExp_Explorer e(body->geom, TopAbs_FACE); e.More(); e.Next()) {
        if (BRepAdaptor_Surface(TopoDS::Face(e.Current())).GetType() == GeomAbs_Plane) ++caps;
    }
    check(solids == 1 && shells == 1 && caps == 2, "open sweep has one solid and two plane caps");
    check_open_caps(body->geom, a, b, start, end, rotated, angle);
    BRepClass3d_SolidClassifier classifier(body->geom);
    classifier.PerformInfinitePoint(1e-9);
    check(classifier.State() == TopAbs_OUT, "open sweep is outward");
    const double expected = std::acos(-1.0) * radius * radius * arc_length(a, b, start, end);
    constexpr double kVolumeToleranceMm3 = 1e-6;
    check_near(session::shape_volume(body->geom), expected, kVolumeToleranceMm3,
               "open sweep analytic arc volume");
}

void open_arc_case(double a, double b, double radius, double start, double end, bool ellipse) {
    Fixture fixture(a, b, radius, ellipse, 3);
    set_open_section(fixture, a, b, start, end);
    const auto out = fixture.run();
    if (out.status != ops::OpOutcome::Status::Ok)
        std::fprintf(stderr, "open sweep: %s\n", out.error_message.c_str());
    check(out.status == ops::OpOutcome::Status::Ok, "open analytic sweep accepted");
    check_open_result(fixture, a, b, radius, start, end);
    check(out.body_events.size() == 1 && out.body_events[0].kind == "created", "open sweep creates one body");
}

void open_arc_rotated(bool ellipse) {
    constexpr double start = -0.4, end = 0.3;
    const double b = ellipse ? 2 : 4;
    Fixture fixture(4, b, .2, ellipse);
    set_open_section(fixture, 4, b, start, end);
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
    check(out.status == ops::OpOutcome::Status::Ok, "rotated open sweep accepted");
    check_open_result(fixture, 4, b, .2, start, end, true);
}

void open_arc_rotated_ellipse_basis() {
    constexpr double start = -.3, end = .45, angle = .4;
    Fixture fixture(4, 2, .2, true);
    fixture.sketches[1].second["entities"][0]["rotation"] = angle;
    set_open_section(fixture, 4, 2, start, end, angle);
    const auto out = fixture.run();
    check(out.status == ops::OpOutcome::Status::Ok, "rotated ellipse basis accepted");
    check_open_result(fixture, 4, 2, .2, start, end, false, angle);
}

void open_arc_controls() {
    open_arc_case(4, 4, .5, .25, 1.25, false);
    open_arc_case(1.0816653826343816, .6, .3, 2.156580469758751e-12, .14936698838903525, true);
    open_arc_case(1.0816653826343816, .6, .3, -.15494271314769836, 2.1560531138236046e-12, true);
    open_arc_case(4, 2, .2, -.4, .3, true);
    open_arc_rotated(true);
    open_arc_rotated(false);
    open_arc_rotated_ellipse_basis();
}

void open_arc_refusals() {
    const double turn = 2 * std::acos(-1.0);
    const std::vector<json> bad = {
        nullptr, json::array(), json::object(), {{"startRadians", 0}},
        {{"startRadians", 0}, {"endRadians", 1}, {"unknown", 2}},
        {{"startRadians", "bad"}, {"endRadians", 1}},
        {{"startRadians", 0}, {"endRadians", 0}},
        {{"startRadians", 1}, {"endRadians", 0}},
        {{"startRadians", 0}, {"endRadians", turn}},
        {{"startRadians", -turn}, {"endRadians", 0}},
        {{"startRadians", -turn - 1}, {"endRadians", -turn}},
        {{"startRadians", 0}, {"endRadians", turn + 1}},
        {{"startRadians", std::numeric_limits<double>::infinity()}, {"endRadians", 1}},
        {{"startRadians", std::numeric_limits<double>::quiet_NaN()}, {"endRadians", 1}},
    };
    for (const auto& value : bad) {
        Fixture fixture(4, 4, .5, false);
        const TopoDS_Shape existing = BRepPrimAPI_MakeBox(2, 3, 4).Shape();
        fixture.bodies.create("body_existing", "existing", existing);
        fixture.op["params"]["pathRange"] = value;
        const auto out = fixture.run();
        check(out.status != ops::OpOutcome::Status::Ok, "invalid open path range refused");
        check(fixture.bodies.all().size() == 1 && fixture.bodies.get("body_existing")->geom.IsEqual(existing),
              "invalid path range preserves head");
        check(out.body_events.empty() && out.topology_history.empty(), "invalid range publishes nothing");
    }
    Fixture near_full(4, 4, .5, false);
    set_open_section(near_full, 4, 4, 0, turn - .1);
    const auto near_full_out = near_full.run();
    check(near_full_out.status == ops::OpOutcome::Status::Ok, "separated near-full arc accepted");
    check_open_result(near_full, 4, 4, .5, 0, turn - .1);
    if (const auto* body = near_full.bodies.get("body_sweep")) {
        std::vector<TopoDS_Face> caps;
        for (TopExp_Explorer e(body->geom, TopAbs_FACE); e.More(); e.Next()) {
            const auto face = TopoDS::Face(e.Current());
            if (BRepAdaptor_Surface(face).GetType() == GeomAbs_Plane) caps.push_back(face);
        }
        if (caps.size() == 2) {
            BRepExtrema_DistShapeShape distance(caps[0], caps[1]);
            distance.Perform();
            check(distance.IsDone(), "near-full cap separation computed");
            if (distance.IsDone()) check_near(distance.Value(), 2 * (4 - .5) * std::sin(.05),
                                              1e-6, "near-full caps have analytic separation");
        } else check(false, "near-full arc has two caps");
    }
    Fixture cancelled(4, 4, .5, false);
    set_open_section(cancelled, 4, 4, 0, .5);
    cancelled.cancel.cancel();
    check(cancelled.run().status == ops::OpOutcome::Status::Cancelled && cancelled.bodies.all().empty(),
          "cancelled open sweep publishes nothing");
    Fixture wrong_start(4, 4, .5, false);
    wrong_start.op["params"]["pathRange"] = {{"startRadians", .3}, {"endRadians", .6}};
    check(wrong_start.run().status != ops::OpOutcome::Status::Ok, "profile must match path start");
}
