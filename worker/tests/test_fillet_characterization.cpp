#include <sys/wait.h>
#include <unistd.h>

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepLib.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Solid.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/FilletBuilder.h"
#include "kernel/validation/ShapeAudit.h"
#include "session/PlanExecutor.h"
#include "session/Signatures.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;
namespace session = onecad::session;

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

TopoDS_Face face(const std::vector<gp_Pnt> &points) {
  BRepBuilderAPI_MakePolygon polygon;
  for (const gp_Pnt &point : points)
    polygon.Add(point);
  polygon.Close();
  return BRepBuilderAPI_MakeFace(polygon.Wire()).Face();
}

TopoDS_Shape square_pyramid() {
  const gp_Pnt a(-5, -5, 0), b(5, -5, 0), c(5, 5, 0), d(-5, 5, 0),
      top(0, 0, 10);
  BRepBuilderAPI_Sewing sewing;
  sewing.Add(face({a, d, c, b}));
  sewing.Add(face({a, b, top}));
  sewing.Add(face({b, c, top}));
  sewing.Add(face({c, d, top}));
  sewing.Add(face({d, a, top}));
  sewing.Perform();
  TopoDS_Shell shell;
  for (TopExp_Explorer ex(sewing.SewedShape(), TopAbs_SHELL); ex.More();
       ex.Next()) {
    shell = TopoDS::Shell(ex.Current());
    break;
  }
  TopoDS_Solid solid = BRepBuilderAPI_MakeSolid(shell).Solid();
  BRepLib::OrientClosedSolid(solid);
  return solid;
}

std::vector<TopoDS_Edge> incident_edges(const TopoDS_Shape &shape,
                                        const gp_Pnt &point) {
  std::vector<TopoDS_Edge> out;
  for (const TopoDS_Edge &edge : ft::all_edges(shape)) {
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if ((!first.IsNull() && BRep_Tool::Pnt(first).Distance(point) <= 1e-7) ||
        (!last.IsNull() && BRep_Tool::Pnt(last).Distance(point) <= 1e-7)) {
      out.push_back(edge);
    }
  }
  return out;
}

void test_analytic_radius() {
  const TopoDS_Shape body = ft::box();
  kf::FilletBuilder builder(
      body, {ft::resolved(body, ft::vertical_edges(body).front())}, 1.0);
  const kf::FilletBuildResult result = builder.build();
  check(result.ok, "analytic fillet succeeds");
  int cylinders = 0;
  for (TopExp_Explorer ex(result.shape, TopAbs_FACE); ex.More(); ex.Next()) {
    const BRepAdaptor_Surface surface(TopoDS::Face(ex.Current()));
    if (surface.GetType() == GeomAbs_Cylinder) {
      ++cylinders;
      check(std::abs(surface.Cylinder().Radius() - 1.0) <= 1e-9,
            "generated cylinder carries requested radius");
    }
  }
  check(cylinders > 0, "analytic fillet publishes cylindrical blend face");
}

void test_simulation_sections_are_characterization_only() {
  const TopoDS_Shape body = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(body).front();
  BRepFilletAPI_MakeFillet builder(body);
  builder.Add(1.0, edge);
  const int contour = builder.Contour(edge);
  check(contour > 0, "simulation contour exists");
  try {
    builder.Simulate(contour);
    const int surfaces = builder.NbSurf(contour);
    check(surfaces > 0, "Simulate publishes section surfaces");
    for (int i = 1; i <= surfaces; ++i) {
      (void)builder.Sect(contour, i);
    }
  } catch (...) {
    check(false, "Simulate/Sect characterization does not throw");
  }
}

void test_high_valence_vertices_refuse_safely() {
  const TopoDS_Shape box = ft::box();
  const std::vector<TopoDS_Edge> three = incident_edges(box, gp_Pnt(0, 0, 0));
  check(three.size() == 3, "box corner has three incident edges");
  kf::FilletBuilder three_builder(box, ft::resolved(box, three), 0.5);
  const kf::FilletBuildResult three_result = three_builder.build();
  check(three_result.ok ? three_result.output_audit.publishable()
                        : !three_result.diagnostics.empty(),
        "three-edge vertex classifies safely");

  const TopoDS_Shape pyramid = square_pyramid();
  check(onecad::kernel::validation::audit_shape(pyramid).publishable(),
        "pyramid input passes shape audit");
  const std::vector<TopoDS_Edge> four =
      incident_edges(pyramid, gp_Pnt(0, 0, 10));
  check(four.size() == 4, "pyramid apex has four incident edges");
  kf::FilletBuilder four_builder(pyramid, ft::resolved(pyramid, four), 0.5);
  const kf::FilletBuildResult four_result = four_builder.build();
  check(four_result.ok ? four_result.output_audit.publishable()
                       : !four_result.diagnostics.empty(),
        "four-edge vertex classifies safely");
}

std::string replay_digest() {
  session::ScratchJob job;
  const TopoDS_Shape body = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(body).front();
  job.bodies.create("body_box", "op_box", body);
  std::string last_sketch;
  const session::CandidateResult candidate =
      session::execute_candidate_op(job, ft::op(body, {edge}, 1.0), "op_fillet",
                                    last_sketch, onecad::CancelToken{});
  if (candidate.status != session::CandidateResult::Status::Ok)
    return "candidate-failed";
  return std::to_string(static_cast<int>(candidate.status)) + "|" +
         session::geometry_signature(job.bodies) + "|" +
         candidate.delta.to_json().dump();
}

std::string child_digest(const char *executable) {
  int fds[2];
  if (pipe(fds) != 0)
    return "pipe-failed";
  const pid_t pid = fork();
  if (pid < 0) {
    close(fds[0]);
    close(fds[1]);
    return "fork-failed";
  }
  if (pid == 0) {
    if (dup2(fds[1], STDOUT_FILENO) < 0)
      _exit(126);
    close(fds[0]);
    close(fds[1]);
    execl(executable, executable, "digest", nullptr);
    _exit(127);
  }
  close(fds[1]);
  std::string output;
  char buffer[512];
  ssize_t count = 0;
  while ((count = read(fds[0], buffer, sizeof(buffer))) > 0)
    output.append(buffer, count);
  close(fds[0]);
  int status = 0;
  const pid_t waited = waitpid(pid, &status, 0);
  return count == 0 && waited == pid && WIFEXITED(status) &&
                 WEXITSTATUS(status) == 0 && !output.empty()
             ? output
             : "child-failed";
}

void test_fresh_process_replay(const char *executable) {
  const std::string first = child_digest(executable);
  const std::string second = child_digest(executable);
  check(!first.empty() && first == second && first != "child-failed" &&
            first != "pipe-failed" && first != "fork-failed" &&
            first.find("candidate-failed") == std::string::npos,
        "fresh-process replay digest stable");
}

} // namespace

int main(int argc, char **argv) {
  if (argc == 2 && std::string(argv[1]) == "digest") {
    std::printf("%s\n", replay_digest().c_str());
    return 0;
  }
  test_analytic_radius();
  test_simulation_sections_are_characterization_only();
  test_high_valence_vertices_refuse_safely();
  test_fresh_process_replay(argv[0]);
  return failures;
}
