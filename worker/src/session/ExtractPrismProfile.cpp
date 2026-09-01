// ExtractPrismProfile.cpp — see ExtractPrismProfile.h. SCHEMA §7.8.
#include "session/ExtractPrismProfile.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <optional>
#include <string>
#include <vector>

#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepGProp.hxx>
#include <BRepTools.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Vertex.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Mat.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_XYZ.hxx>

#include "io/BrepCodec.h"
#include "ops/OpCommon.h"
#include "session/BodyStore.h"
#include "util/Hashing.h"
#include "util/Log.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;

namespace {

// A face counts as an end cap when its orientation-corrected normal is within
// this angle (rad) of ±axis.
constexpr double kAxisParallelTolRad = 1.0e-6;
// Two candidate caps at the same offset along the axis belong to the same end.
constexpr double kEndGroupTolMm = 1.0e-6;
constexpr double kVolumeRelTol = 1.0e-6;
// DELIBERATELY the same margin as the volume gate. A tighter one has no
// engineering meaning here: the two caps of a real vendor extrusion are rounded
// independently by whatever wrote the STEP, and at 1e-9 a 10 pm taper across a
// 20 mm edge would refuse a stick that is a prism by every measure a machinist
// has. The volume gate still catches a taper that matters (a 1e-3 one is refused
// by volume long before area notices).
constexpr double kAreaRelTol = 1.0e-6;
// Below this the two in-plane principal moments are indistinguishable and the
// eigenvectors carry no information (a square, a circle, a regular polygon).
constexpr double kIsotropyRelTol = 1.0e-9;
// The tie-break quantum: the frame is chosen on vertex coordinates rounded to
// this, so a coordinate has to move by half a quantum — not by a last bit — for
// a different frame to win. That makes a flip overwhelmingly unlikely, not
// impossible: a vertex sitting exactly on a quantisation boundary can still tip.
constexpr double kQuantumMm = 1.0e-7;

Envelope fail(const Envelope& req, const char* code, const std::string& message,
              json detail = json()) {
    return Envelope::error_response(
        req.id, protocol::ErrorInfo{code, message, /*retriable=*/false,
                                    detail.is_null() ? std::optional<json>{}
                                                     : std::optional<json>{std::move(detail)}});
}

std::string get_str(const json& o, const char* key) {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return {};
}

double area_of(const TopoDS_Shape& s) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(s, props);
    return props.Mass();
}

// The longest axis-aligned bounding-box dimension, as an EXACT canonical basis
// vector. A tie falls to X, then Y, then Z: a cube has no distinguished axis and
// choosing one on a last-bit difference would be a guess dressed as a
// measurement.
gp_Dir longest_bbox_axis(const TopoDS_Shape& body) {
    Bnd_Box box;
    BRepBndLib::Add(body, box);
    if (box.IsVoid()) return gp_Dir(1.0, 0.0, 0.0);
    Standard_Real x0 = 0, y0 = 0, z0 = 0, x1 = 0, y1 = 0, z1 = 0;
    box.Get(x0, y0, z0, x1, y1, z1);
    const std::array<double, 3> extent{x1 - x0, y1 - y0, z1 - z0};
    std::size_t best = 0;
    for (std::size_t i = 1; i < 3; ++i) {
        if (extent[i] > extent[best] + kEndGroupTolMm) best = i;
    }
    return gp_Dir(best == 0 ? 1.0 : 0.0, best == 1 ? 1.0 : 0.0, best == 2 ? 1.0 : 0.0);
}

struct CapCandidate {
    int ordinal = 0;
    double offset = 0.0;  // signed plane offset along the axis
    double area = 0.0;
};

// Every planar face whose orientation-corrected normal is parallel or
// anti-parallel to `axis`, in FACE ORDINAL order.
std::vector<CapCandidate> collect_caps(const TopTools_IndexedMapOfShape& faces, const gp_Dir& axis) {
    std::vector<CapCandidate> caps;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const TopoDS_Face face = TopoDS::Face(faces(i));
        gp_Pln plane;
        gp_Dir normal;
        if (!ops::planar_face_plane_normal(face, plane, normal)) continue;
        if (!normal.IsParallel(axis, kAxisParallelTolRad)) continue;
        caps.push_back({i, plane.Location().XYZ().Dot(axis.XYZ()), area_of(face)});
    }
    return caps;
}

// The largest-area candidate within `kEndGroupTolMm` of `offset` — the end cap
// at that end. Ties on area fall to the LOWER face ordinal (candidates are in
// ordinal order and the comparison is strict).
const CapCandidate* cap_at(const std::vector<CapCandidate>& caps, double offset) {
    const CapCandidate* best = nullptr;
    for (const CapCandidate& c : caps) {
        if (std::abs(c.offset - offset) > kEndGroupTolMm) continue;
        if (best == nullptr || c.area > best->area) best = &c;
    }
    return best;
}

// `outerEdgeCount` / `innerWireCount` of the baked face. Both walks are indexed
// maps, so a seam edge shared by two wires is counted once.
void count_boundary(const TopoDS_Face& face, int& outer_edges, int& inner_wires) {
    TopTools_IndexedMapOfShape wires;
    TopExp::MapShapes(face, TopAbs_WIRE, wires);
    inner_wires = wires.Extent() > 0 ? wires.Extent() - 1 : 0;
    const TopoDS_Wire outer = BRepTools::OuterWire(face);
    if (outer.IsNull()) {
        outer_edges = 0;
        return;
    }
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(outer, TopAbs_EDGE, edges);
    outer_edges = edges.Extent();
}

std::string percent_message(double ratio) {
    char buf[128];
    std::snprintf(buf, sizeof(buf), "volume is %.1f%% of endCapArea x length", ratio * 100.0);
    return std::string(buf);
}

// A relative delta, in the message as well as on `refusal.areaDelta`: the number
// is what lets an ingest UI say "this stick tapers by 0.3%" instead of shrugging.
std::string ratio_message(double relative) {
    char buf[64];
    std::snprintf(buf, sizeof(buf), "%.3g relative", relative);
    return std::string(buf);
}

}  // namespace

PrismAnalysis analyze_prism(const TopoDS_Shape& body, const gp_Dir* axis_hint) {
    PrismAnalysis out;
    if (body.IsNull()) {
        out.refusal = "the body carries no geometry";
        return out;
    }
    out.axis = axis_hint != nullptr ? *axis_hint : longest_bbox_axis(body);

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    const std::vector<CapCandidate> caps = collect_caps(faces, out.axis);
    if (caps.size() < 2) {
        // ANALYTIC: the planarity test is `BRepAdaptor_Surface::GetType() ==
        // GeomAbs_Plane`, so a cap a translator stored as a degenerate B-spline
        // is not recognised. Naming that in the message is the honest V1 answer;
        // a `GeomLib_IsPlanarSurface` fallback would change which bodies ingest.
        out.refusal = "the body has fewer than two ANALYTIC planar faces perpendicular to the "
                      "axis (B-spline planes are not recognised in V1)";
        return out;
    }

    double lo = caps.front().offset;
    double hi = lo;
    for (const CapCandidate& c : caps) {
        lo = std::min(lo, c.offset);
        hi = std::max(hi, c.offset);
    }
    if (hi - lo <= kEndGroupTolMm) {
        out.refusal = "every face perpendicular to the axis lies in one plane";
        return out;
    }

    // STRUCTURAL, and it must run BEFORE the two measurements, because it is the
    // only one that catches a body they both accept: a 20x20 stick fused end to
    // end onto a 40x10 stick has equal cap areas (400 mm²) and a volume that is
    // exactly area×length, so it passes the volume gate and the equal-area gate
    // and would bake the 20x20 square as "the section". The step face at the
    // junction is the evidence. A true prism has no such face — its lateral
    // faces are PARALLEL to the axis and are never collected here — so this
    // cannot refuse a prism, while it also catches counterbores and internal
    // steps for free.
    for (const CapCandidate& c : caps) {
        if (std::abs(c.offset - lo) <= kEndGroupTolMm) continue;
        if (std::abs(c.offset - hi) <= kEndGroupTolMm) continue;
        out.refusal = "an axis-perpendicular planar face lies strictly between the end caps at "
                      "offset " +
                      std::to_string(c.offset) + " (the body is more than one prism)";
        return out;
    }

    const CapCandidate* low = cap_at(caps, lo);
    const CapCandidate* high = cap_at(caps, hi);

    // PLANE-TO-PLANE, deliberately: a `Bnd_Box` is inflated by the shape's
    // tolerance, and this number is multiplied into the prism-ness test below.
    out.length_mm = hi - lo;
    out.area_mm2 = low->area;
    out.end_cap_ordinal = low->ordinal;

    GProp_GProps volume_props;
    BRepGProp::VolumeProperties(body, volume_props);
    const double volume = volume_props.Mass();
    const double swept = out.area_mm2 * out.length_mm;
    out.volume_ratio = swept > 0.0 ? volume / swept : 0.0;

    if (std::abs(volume - swept) > kVolumeRelTol * std::abs(volume)) {
        out.refusal = percent_message(out.volume_ratio);
        return out;
    }
    const double area_span = std::max(low->area, high->area);
    out.area_delta = area_span > 0.0 ? std::abs(low->area - high->area) / area_span : 0.0;
    if (out.area_delta > kAreaRelTol) {
        out.refusal = "the two end caps differ in area by " + ratio_message(out.area_delta) +
                      ", so the section is not constant";
        return out;
    }

    const TopoDS_Face cap = TopoDS::Face(faces(out.end_cap_ordinal));
    count_boundary(cap, out.outer_edge_count, out.inner_wire_count);
    out.is_prism = true;
    return out;
}

namespace {

// `p -> ((p-origin)·ax, (p-origin)·ay, (p-origin)·az)`, built by hand rather
// than through `gp_Trsf::SetTransformation` so the direction of the mapping is
// unambiguous at the call site.
gp_Trsf world_to_frame(const gp_Pnt& origin, const gp_Dir& ax, const gp_Dir& ay, const gp_Dir& az) {
    const gp_XYZ o = origin.XYZ();
    gp_Trsf t;
    t.SetValues(ax.X(), ax.Y(), ax.Z(), -ax.XYZ().Dot(o),  //
                ay.X(), ay.Y(), ay.Z(), -ay.XYZ().Dot(o),  //
                az.X(), az.Y(), az.Z(), -az.XYZ().Dot(o));
    return t;
}

// The in-plane seed axis, derived from the SHAPE: the direction from the area
// centroid to the FIRST face vertex in `TopExp` ordinal order that is not on the
// centroid. It exists because the isotropic branch below has no eigenvector to
// fall back on — a square's moment tensor is a multiple of the identity, every
// frame diagonalises it, and a WORLD-seeded intermediate frame would make the
// canonical bytes a function of the input file's pose.
//
// There is NO world-space fallback, deliberately. §7.8 forbids a world seed by
// name, so a face that offers no usable vertex (unreachable for any real profile
// — even a full circle carries its seam vertex) FAILS THE BAKE rather than
// quietly canonicalising against `gp_Ax2`'s arbitrary XDirection. A refusal a
// caller can see beats a blob that stops deduplicating for reasons nothing
// records.
std::optional<gp_Dir> inplane_seed(const TopoDS_Face& face, const gp_Pnt& centroid,
                                   const gp_Dir& normal) {
    TopTools_IndexedMapOfShape vertices;
    TopExp::MapShapes(face, TopAbs_VERTEX, vertices);
    for (int i = 1; i <= vertices.Extent(); ++i) {
        const gp_XYZ r = BRep_Tool::Pnt(TopoDS::Vertex(vertices(i))).XYZ() - centroid.XYZ();
        const gp_XYZ in_plane = r - normal.XYZ() * r.Dot(normal.XYZ());
        if (in_plane.Modulus() > 1.0e-9) return gp_Dir(in_plane);
    }
    return std::nullopt;
}

// The principal in-plane angle of a face already sitting on the XY plane with
// its centroid at the origin. `0` when the two moments are indistinguishable —
// there the eigenvectors are arbitrary and the seed frame is the answer.
double principal_angle(const TopoDS_Face& face_at_origin) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(face_at_origin, props);
    const gp_Mat inertia = props.MatrixOfInertia();
    const double jxx = inertia.Value(1, 1);
    const double jyy = inertia.Value(2, 2);
    const double jxy = inertia.Value(1, 2);
    const double spread = std::sqrt((jxx - jyy) * (jxx - jyy) + 4.0 * jxy * jxy);
    if (spread <= kIsotropyRelTol * std::abs(jxx + jyy)) return 0.0;
    return 0.5 * std::atan2(2.0 * jxy, jxx - jyy);
}

using QuantVertex = std::array<long long, 3>;

// Every vertex of `face_at_origin` expressed in the candidate frame `(cx, cy)`,
// quantised and sorted — the normative tie-break key. Lexicographically smallest
// wins; identical keys mean the candidate frames are related by a symmetry of
// the profile, so the choice between them cannot be observed.
std::vector<QuantVertex> frame_key(const std::vector<gp_Pnt>& vertices, double cx, double cy) {
    std::vector<QuantVertex> key;
    key.reserve(vertices.size());
    for (const gp_Pnt& p : vertices) {
        const double x = p.X() * cx + p.Y() * cy;
        const double y = -p.X() * cy + p.Y() * cx;
        key.push_back({std::llround(x / kQuantumMm), std::llround(y / kQuantumMm),
                       std::llround(p.Z() / kQuantumMm)});
    }
    std::sort(key.begin(), key.end());
    return key;
}

}  // namespace

std::string bake_canonical_profile(const TopoDS_Shape& body, const PrismAnalysis& analysis,
                                   std::vector<std::uint8_t>& bytes_out) {
    if (!analysis.is_prism) return "the body is not a prism";
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    if (analysis.end_cap_ordinal < 1 || analysis.end_cap_ordinal > faces.Extent()) {
        return "the end-cap ordinal does not address a face";
    }
    // FORWARD first: the canonical face stores orientation FORWARD, so the plane
    // normal it must carry is the one a FORWARD reading sees. A face's REGION is
    // its wires as stored (OCCT classifies with the face taken FORWARD); the
    // orientation flag only names the material side, which a standalone profile
    // does not have.
    const TopoDS_Face cap =
        TopoDS::Face(faces(analysis.end_cap_ordinal).Oriented(TopAbs_FORWARD));

    gp_Pln plane;
    gp_Dir normal;
    if (!ops::planar_face_plane_normal(cap, plane, normal)) return "the end cap is not planar";

    GProp_GProps props;
    BRepGProp::SurfaceProperties(cap, props);
    const gp_Pnt centroid = props.CentreOfMass();  // holes subtracted by construction
    const std::optional<gp_Dir> seed = inplane_seed(cap, centroid, normal);
    if (!seed) {
        return "the end cap offers no vertex off its centroid, so the canonical in-plane frame "
               "would have to be seeded from world space (forbidden: it would make the blob a "
               "function of the source file's pose)";
    }
    const gp_Trsf to_seed_frame =
        world_to_frame(centroid, *seed, normal.Crossed(*seed), normal);

    try {
        BRepBuilderAPI_Transform seeded(cap, to_seed_frame, /*Copy=*/Standard_True);
        if (!seeded.IsDone() || seeded.Shape().IsNull()) return "the seed transform failed";
        const TopoDS_Face face_at_origin = TopoDS::Face(seeded.Shape());

        const double angle = principal_angle(face_at_origin);
        TopTools_IndexedMapOfShape vertex_map;
        TopExp::MapShapes(face_at_origin, TopAbs_VERTEX, vertex_map);
        std::vector<gp_Pnt> vertices;
        vertices.reserve(static_cast<std::size_t>(vertex_map.Extent()));
        for (int i = 1; i <= vertex_map.Extent(); ++i) {
            vertices.push_back(BRep_Tool::Pnt(TopoDS::Vertex(vertex_map(i))));
        }

        // The four candidates: the principal axis and its perpendicular, each in
        // both signs, `y` derived right-handed. Exact quarter turns of (c, s) —
        // never `cos(angle + k*pi/2)`, which is only exact for k == 0.
        const double c = std::cos(angle);
        const double s = std::sin(angle);
        const std::array<std::array<double, 2>, 4> candidates{
            {{c, s}, {-s, c}, {-c, -s}, {s, -c}}};
        std::size_t best = 0;
        std::vector<QuantVertex> best_key = frame_key(vertices, candidates[0][0], candidates[0][1]);
        for (std::size_t k = 1; k < candidates.size(); ++k) {
            std::vector<QuantVertex> key = frame_key(vertices, candidates[k][0], candidates[k][1]);
            if (key < best_key) {
                best_key = std::move(key);
                best = k;
            }
        }

        gp_Trsf spin;
        spin.SetValues(candidates[best][0], candidates[best][1], 0.0, 0.0,   //
                       -candidates[best][1], candidates[best][0], 0.0, 0.0,  //
                       0.0, 0.0, 1.0, 0.0);
        BRepBuilderAPI_Transform canonical(cap, spin * to_seed_frame, /*Copy=*/Standard_True);
        if (!canonical.IsDone() || canonical.Shape().IsNull()) {
            return "the canonical transform failed";
        }
        return io::write_brep_shape(canonical.Shape(), bytes_out);
    } catch (const Standard_Failure& f) {
        return std::string("canonical bake raised: ") +
               (f.GetMessageString() != nullptr ? f.GetMessageString() : "OCCT failure");
    }
}

namespace {

json prism_json(const PrismAnalysis& a) {
    return json{{"axis", json::array({a.axis.X(), a.axis.Y(), a.axis.Z()})},
                {"endCap", json{{"topoKey", "f:" + std::to_string(a.end_cap_ordinal)}}},
                {"lengthMm", a.length_mm},
                {"areaMm2", a.area_mm2},
                {"volumeRatio", a.volume_ratio},
                {"outerEdgeCount", a.outer_edge_count},
                {"innerWireCount", a.inner_wire_count}};
}

// One shell for all three branches (prism+write, prism without `path`, refusal)
// so a reader parses one key set.
json response(json prism, bool written, std::uint64_t bytes, json sha256, json refusal) {
    return json{{"prism", std::move(prism)},
                {"written", written},
                {"bytes", bytes},
                {"codec", "brep"},
                {"format", io::kBrepFormatVersion},
                {"sha256", std::move(sha256)},
                {"refusal", std::move(refusal)}};
}

// `axisHint` is optional; a present one must be a finite, non-degenerate 3-array.
// Returns false with `err` set on a malformed value.
bool read_axis_hint(const json& args, std::optional<gp_Dir>& hint, std::string& err) {
    if (!args.is_object() || !args.contains("axisHint")) return true;
    const json& v = args["axisHint"];
    if (!v.is_array() || v.size() != 3) {
        err = "ExtractPrismProfile: axisHint must be a 3-array";
        return false;
    }
    double xyz[3] = {0.0, 0.0, 0.0};
    for (std::size_t i = 0; i < 3; ++i) {
        if (!v[i].is_number()) {
            err = "ExtractPrismProfile: axisHint must contain only numbers";
            return false;
        }
        xyz[i] = v[i].get<double>();
        if (!std::isfinite(xyz[i])) {
            err = "ExtractPrismProfile: axisHint must be finite";
            return false;
        }
    }
    if (gp_XYZ(xyz[0], xyz[1], xyz[2]).Modulus() < 1.0e-9) {
        err = "ExtractPrismProfile: axisHint is degenerate";
        return false;
    }
    hint = gp_Dir(xyz[0], xyz[1], xyz[2]);
    return true;
}

}  // namespace

Envelope handle_extract_prism_profile(Session& session, const Envelope& req) {
    const json& args = req.args;
    const std::uint64_t head_snapshot = session.current_snapshot_id();

    // The fence is MANDATORY: this answer is about to be frozen into a package.
    if (!args.is_object() || !args.contains("snapshotId") ||
        !args["snapshotId"].is_number_unsigned()) {
        return fail(req, "PROTOCOL_ERROR", "ExtractPrismProfile: snapshotId (u64) is required");
    }
    const std::uint64_t requested = args["snapshotId"].get<std::uint64_t>();
    if (requested != head_snapshot) {
        return fail(req, "STALE_PREVIEW",
                    "ExtractPrismProfile: the body was picked against snapshot " +
                        std::to_string(requested) + "; head is " +
                        std::to_string(head_snapshot) + " — re-pick",
                    json{{"requested", requested}, {"head", head_snapshot}});
    }

    const std::string body_id = get_str(args, "bodyId");
    const BodyStore bodies = session.bodies_copy();  // a COPY of the head; nothing below mutates
    const BodyRecord* rec = body_id.empty() ? nullptr : bodies.get(body_id);
    if (rec == nullptr || rec->geom.IsNull()) {
        return fail(req, "REF_UNRESOLVED",
                    "ExtractPrismProfile: unknown bodyId '" + body_id + "'");
    }

    std::optional<gp_Dir> hint;
    std::string err;
    if (!read_axis_hint(args, hint, err)) return fail(req, "PROTOCOL_ERROR", err);
    if (args.contains("path") && !args["path"].is_string()) {
        return fail(req, "PROTOCOL_ERROR", "ExtractPrismProfile: path must be a string");
    }
    const std::string path = get_str(args, "path");

    const PrismAnalysis analysis = analyze_prism(rec->geom, hint ? &*hint : nullptr);
    if (!analysis.is_prism) {
        return Envelope::ok_response(
            req.id, response(json(), /*written=*/false, 0, json(),
                             json{{"code", "notAPrism"},
                                  {"message", analysis.refusal},
                                  {"volumeRatio", analysis.volume_ratio},
                                  {"areaDelta", analysis.area_delta}}));
    }
    if (path.empty()) {
        return Envelope::ok_response(
            req.id, response(prism_json(analysis), /*written=*/false, 0, json(), json()));
    }

    std::vector<std::uint8_t> bytes;
    const std::string bake_error = bake_canonical_profile(rec->geom, analysis, bytes);
    if (!bake_error.empty()) {
        return fail(req, "OP_FAILED", "ExtractPrismProfile: " + bake_error);
    }
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    if (out) {
        out.write(reinterpret_cast<const char*>(bytes.data()),
                  static_cast<std::streamsize>(bytes.size()));
        out.close();
    }
    if (!out) {
        return fail(req, "OP_FAILED", "ExtractPrismProfile: write to " + path + " failed");
    }

    WLOG_INFO("ExtractPrismProfile: %s cap %s, %.6f mm long, %zu bytes", body_id.c_str(),
              ("f:" + std::to_string(analysis.end_cap_ordinal)).c_str(), analysis.length_mm,
              bytes.size());
    return Envelope::ok_response(
        req.id, response(prism_json(analysis), /*written=*/true,
                         static_cast<std::uint64_t>(bytes.size()),
                         json(hashing::sha256_hex(bytes.data(), bytes.size())), json()));
}

}  // namespace onecad::session
