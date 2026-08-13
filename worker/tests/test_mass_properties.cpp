// test_mass_properties.cpp — WP-C1: the `QueryMassProperties` verb (SCHEMA §7.5).
//
// Every numeric expectation here is SELF-DERIVED: each case builds its own OCCT
// primitive from named dimensions, so the expected volume / area / centroid /
// principal moments are consequences of the construction written beside them.
// No corpus recording is involved (the oracle has no mass-properties entry point).
//
// The load-bearing properties, in order:
//   * EXACT on analytic solids — a 20x30x40 box is 24000 mm^3 and 5200 mm^2 to
//     the last digit, not "close"; a cylinder matches pi*r^2*h;
//   * the centroid is the VOLUME centre of mass, not the bbox centre;
//   * `principalMoments` are about the CENTROID (the contract's frame), which for
//     a box is V*(b^2+c^2)/12 — an about-ORIGIN answer would be ~10x larger for
//     an off-origin box, so this case is what pins the Huygens transfer;
//   * `principalAxes` are unit, right-handed, and sign-canonical;
//   * unknown bodyId is a recoverable REF_UNRESOLVED, never a zero reading;
//   * the verb is READ-ONLY — the head is identical before and after.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>
#include <gp_Pnt.hxx>

#include "nlohmann/json.hpp"
#include "session/HistoryHash.h"
#include "session/MassProperties.h"
#include "session/BodyTopology.h"
#include "session/ScratchJob.h"
#include "session/Session.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::session::Session;

namespace {

int g_failures = 0;

void check(bool ok, const std::string& what) {
    if (!ok) {
        ++g_failures;
        std::fprintf(stderr, "FAIL: %s\n", what.c_str());
    }
}

void check_near(double got, double want, double tol, const std::string& what) {
    if (!(std::fabs(got - want) <= tol)) {
        ++g_failures;
        std::fprintf(stderr, "FAIL: %s (got %.10g, want %.10g, tol %g)\n", what.c_str(), got, want,
                     tol);
    }
}

/// Seed one body into the session HEAD through the REAL transaction — the same
/// route `test_face_projection.cpp` takes, because `Session` has no test-only
/// setter and inventing one would exercise a path production never takes.
void seed_head(Session& session, const std::string& body_id, const TopoDS_Shape& shape) {
    session.open("doc", /*documentRevision=*/1, /*workerEpoch=*/1, "determinism");
    onecad::session::FenceOutcome fence = session.fence_and_clone(
        /*job_id=*/1, /*document_revision=*/1, /*worker_epoch=*/1,
        onecad::session::kEmptyPrefixHash);
    onecad::session::ScratchJob job;
    job.job_id = 1;
    job.plan_document_revision = 1;
    job.bodies = std::move(fence.cloned_bodies);
    job.partition = std::move(fence.cloned_partition);
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    job.bodies.create(body_id, "op_seed", shape);
    session.store_prepared(std::move(job));
    session.accept_prepared(/*job_id=*/1, /*document_revision=*/1, /*worker_epoch=*/1);
}

Envelope mass_req(const std::string& body_id) {
    Envelope req;
    req.id = 11;
    req.verb = "QueryMassProperties";
    req.args = json{{"bodyId", body_id}};
    return req;
}

Envelope topology_req(const std::string& body_id) {
    Envelope req;
    req.id = 12;
    req.verb = "QueryBodyTopology";
    req.args = json{{"bodyId", body_id}};
    return req;
}

double at(const json& array, std::size_t i) {
    return array[i].get<double>();
}

// ═══════════════════════════════════════════════════════════════════════════
// A 20 x 30 x 40 box with its min corner at the origin.
//   volume    = 24000 mm^3               area = 2*(600 + 800 + 1200) = 5200 mm^2
//   centroid  = (10, 15, 20)
//   moments about the centroid = V*(b^2+c^2)/12 per axis:
//     about X: 24000*(900+1600)/12 = 5.0e6
//     about Y: 24000*(400+1600)/12 = 4.0e6
//     about Z: 24000*(400+ 900)/12 = 2.6e6
//   principal axes = the world axes (the box is axis-aligned).
// ═══════════════════════════════════════════════════════════════════════════
void test_box_exact() {
    Session session;
    seed_head(session, "body_1", BRepPrimAPI_MakeBox(20.0, 30.0, 40.0).Shape());

    const Envelope resp = onecad::session::handle_query_mass_properties(session, mass_req("body_1"));
    check(resp.ok.value_or(false), "box: ok");
    if (!resp.ok.value_or(false)) return;
    const json& r = resp.result;

    // EXACT, not approximate: OCCT integrates a planar-faced solid analytically.
    check_near(r["volume"].get<double>(), 24000.0, 1e-7, "box: volume 24000 mm^3");
    check_near(r["surfaceArea"].get<double>(), 5200.0, 1e-7, "box: area 5200 mm^2");
    check_near(at(r["centroid"], 0), 10.0, 1e-9, "box: centroid X 10");
    check_near(at(r["centroid"], 1), 15.0, 1e-9, "box: centroid Y 15");
    check_near(at(r["centroid"], 2), 20.0, 1e-9, "box: centroid Z 20");

    // The moments come back in the kernel's own order, so match them to the axis
    // each one is PAIRED with rather than assuming a sort.
    const json& moments = r["principalMoments"];
    const json& axes = r["principalAxes"];
    check(moments.size() == 3 && axes.size() == 3, "box: 3 moments + 3 axes");
    const double want[3] = {5.0e6, 4.0e6, 2.6e6};  // indexed by dominant axis
    for (std::size_t k = 0; k < 3; ++k) {
        const json& axis = axes[k];
        // Unit length.
        const double len = std::sqrt(at(axis, 0) * at(axis, 0) + at(axis, 1) * at(axis, 1) +
                                     at(axis, 2) * at(axis, 2));
        check_near(len, 1.0, 1e-12, "box: axis is a unit vector");
        // Sign-canonical: the dominant component is positive.
        std::size_t dominant = 0;
        for (std::size_t i = 1; i < 3; ++i) {
            if (std::fabs(at(axis, i)) > std::fabs(at(axis, dominant))) dominant = i;
        }
        // World-aligned box ⇒ each principal axis IS a world axis.
        check_near(std::fabs(at(axis, dominant)), 1.0, 1e-9, "box: principal axis is a world axis");
        check_near(at(moments, k), want[dominant], 1.0,
                   "box: moment about the centroid matches V*(b^2+c^2)/12");
    }
    // The third axis is a1 x a2, so the frame is right-handed by construction.
    const json& a1 = axes[0];
    const json& a2 = axes[1];
    const json& a3 = axes[2];
    const double cx = at(a1, 1) * at(a2, 2) - at(a1, 2) * at(a2, 1);
    const double cy = at(a1, 2) * at(a2, 0) - at(a1, 0) * at(a2, 2);
    const double cz = at(a1, 0) * at(a2, 1) - at(a1, 1) * at(a2, 0);
    check_near(cx * at(a3, 0) + cy * at(a3, 1) + cz * at(a3, 2), 1.0, 1e-9,
               "box: (a1 x a2) . a3 == +1 (right-handed)");
}

// ═══════════════════════════════════════════════════════════════════════════
// A cylinder r = 5, h = 10, axis +Z from the origin.
//   volume = pi*r^2*h = 250*pi      area = 2*pi*r*h + 2*pi*r^2 = 100*pi + 50*pi
//   centroid = (0, 0, 5)
// A cylinder is NOT integrated in closed form the way a plane-faced box is, so
// the tolerance is a relative one rather than machine epsilon.
// ═══════════════════════════════════════════════════════════════════════════
void test_cylinder() {
    Session session;
    seed_head(session, "body_cyl", BRepPrimAPI_MakeCylinder(5.0, 10.0).Shape());

    const Envelope resp =
        onecad::session::handle_query_mass_properties(session, mass_req("body_cyl"));
    check(resp.ok.value_or(false), "cylinder: ok");
    if (!resp.ok.value_or(false)) return;
    const json& r = resp.result;

    const double want_volume = M_PI * 25.0 * 10.0;
    const double want_area = 2.0 * M_PI * 5.0 * 10.0 + 2.0 * M_PI * 25.0;
    check_near(r["volume"].get<double>(), want_volume, want_volume * 1e-6,
               "cylinder: volume pi*r^2*h");
    check_near(r["surfaceArea"].get<double>(), want_area, want_area * 1e-6,
               "cylinder: area 2*pi*r*h + 2*pi*r^2");
    check_near(at(r["centroid"], 0), 0.0, 1e-6, "cylinder: centroid X 0");
    check_near(at(r["centroid"], 1), 0.0, 1e-6, "cylinder: centroid Y 0");
    check_near(at(r["centroid"], 2), 5.0, 1e-6, "cylinder: centroid Z h/2");
}

// An unknown body is a LOUD, recoverable refusal — never a zero reading, which a
// measurement UI would render as a real "0 mm^3" answer.
void test_unknown_body() {
    Session session;
    seed_head(session, "body_1", BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape());

    const Envelope missing =
        onecad::session::handle_query_mass_properties(session, mass_req("body_nope"));
    check(!missing.ok.value_or(true), "unknown bodyId: not ok");
    check(missing.error.has_value() && missing.error->code == "REF_UNRESOLVED",
          "unknown bodyId ⇒ REF_UNRESOLVED");

    const Envelope empty = onecad::session::handle_query_mass_properties(session, mass_req(""));
    check(!empty.ok.value_or(true) && empty.error->code == "REF_UNRESOLVED",
          "empty bodyId ⇒ REF_UNRESOLVED");
}

// READ-ONLY (SCHEMA §7.5): the head is untouched. This is the invariant that
// makes the verb safe to call from a hover/selection path at any time.
void test_read_only() {
    Session session;
    seed_head(session, "body_1", BRepPrimAPI_MakeBox(20.0, 30.0, 40.0).Shape());

    const onecad::session::WorkerHead before = session.head();
    (void)onecad::session::handle_query_mass_properties(session, mass_req("body_1"));
    (void)onecad::session::handle_query_mass_properties(session, mass_req("body_nope"));
    const onecad::session::WorkerHead after = session.head();

    check(before.document_revision == after.document_revision, "read-only: documentRevision held");
    check(before.worker_epoch == after.worker_epoch, "read-only: workerEpoch held");
    check(before.snapshot_id == after.snapshot_id, "read-only: snapshotId held");
    check(before.history_prefix_hash == after.history_prefix_hash,
          "read-only: historyPrefixHash held");
    check(before.has_scratch == after.has_scratch, "read-only: no scratch left behind");
}

// Same shape, repeated calls ⇒ byte-identical JSON (Invariant 5). The sign
// canonicalization is what makes this true for the axes.
void test_determinism() {
    Session session;
    seed_head(session, "body_1", BRepPrimAPI_MakeBox(20.0, 30.0, 40.0).Shape());

    const std::string first =
        onecad::session::handle_query_mass_properties(session, mass_req("body_1")).result.dump();
    for (int run = 0; run < 3; ++run) {
        const std::string again =
            onecad::session::handle_query_mass_properties(session, mass_req("body_1")).result.dump();
        check(again == first, "determinism: repeated reads are byte-identical");
    }
}

void test_body_topology() {
    Session session;
    seed_head(session, "body_box", BRepPrimAPI_MakeBox(20.0, 30.0, 40.0).Shape());
    const Envelope response = onecad::session::handle_query_body_topology(
        session, topology_req("body_box"));
    check(response.ok.value_or(false), "topology: ok");
    if (response.ok.value_or(false)) {
        check(response.result["solidCount"] == 1, "topology: box has one solid");
        check(response.result["faceCount"] == 6, "topology: box has six faces");
    }
    const Envelope missing = onecad::session::handle_query_body_topology(
        session, topology_req("body_nope"));
    check(!missing.ok.value_or(true) && missing.error->code == "REF_UNRESOLVED",
          "topology: unknown body REF_UNRESOLVED");
}

}  // namespace

int main() {
    test_box_exact();
    test_cylinder();
    test_unknown_body();
    test_read_only();
    test_determinism();
    test_body_topology();
    if (g_failures == 0) std::printf("test_mass_properties: all checks passed\n");
    return g_failures;
}
