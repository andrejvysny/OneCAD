// gear_dump.cpp — dump OneCAD gear-sampler output in a flat text form the
// cross-check harness can diff against the reference implementation.
//
// This is a DEVELOPER TOOL, not part of the worker. It is deliberately NOT
// registered in worker/CMakeLists.txt and never runs in CI (see README.md):
// its counterpart drives the GPL-3.0 `freecad.gears` reference, which must
// stay out of this repo's build and test lanes.
//
// It compiles with a bare g++ and no dependencies — no OCCT, no JSON library,
// no CMake — which is possible precisely because every sampler under
// worker/src/kernel/gear/ is pure arithmetic by construction. If this tool
// ever stops compiling standalone, a sampler has grown a kernel dependency it
// should not have.
//
// Protocol: read one case per line from stdin, write that case's values to
// stdout terminated by `END`. Keys are compared by name, so adding a value
// here does not invalidate an existing harness run.
//
//   in : <recipe> key=value key=value ...
//   out: CASE <line-number>
//        V <name> <double>
//        P <name> <x> <y>
//        ERR <message>          (a refusal — expected for invalid inputs)
//        END

#include <cmath>
#include <cstdio>
#include <iostream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

#include "kernel/gear/InvoluteMath.h"
#include "kernel/gear/LanternMath.h"
#include "kernel/gear/TimingMath.h"
#include "kernel/gear/TimingTMath.h"
#include "kernel/gear/WormMath.h"

namespace gear = onecad::kernel::gear;

namespace {

constexpr double kPi = 3.14159265358979323846;

void emit_value(const char* name, double v) { std::printf("V %s %.17g\n", name, v); }

void emit_point(const std::string& name, const gear::Point2d& p) {
  std::printf("P %s %.17g %.17g\n", name.c_str(), p.x, p.y);
}

void emit_error(const std::string& msg) { std::printf("ERR %s\n", msg.c_str()); }

void emit_segments(const std::vector<gear::ProfileSegment>& segs) {
  for (std::size_t i = 0; i < segs.size(); ++i) {
    const std::string base = "seg" + std::to_string(i);
    emit_value((base + ".kind").c_str(), static_cast<double>(segs[i].kind));
    emit_point(base + ".start", segs[i].start);
    emit_point(base + ".end", segs[i].end);
    if (segs[i].kind == gear::SegmentKind::Arc) emit_point(base + ".mid", segs[i].mid);
    for (std::size_t k = 0; k < segs[i].points.size(); ++k) {
      emit_point(base + ".p" + std::to_string(k), segs[i].points[k]);
    }
  }
}

using Args = std::map<std::string, double>;

double arg(const Args& a, const char* key, double fallback) {
  const auto it = a.find(key);
  return it == a.end() ? fallback : it->second;
}

double deg2rad(double d) { return d * kPi / 180.0; }

void run_involute(const Args& a) {
  gear::InvoluteToothParams p;
  p.module = arg(a, "module", 5.0);
  p.numTeeth = static_cast<int>(arg(a, "teeth", 15));
  p.pressureAngle = deg2rad(arg(a, "pressureAngleDeg", 20.0));
  p.clearance = arg(a, "clearance", 0.25);
  p.shift = arg(a, "shift", 0.0);
  p.helixAngle = deg2rad(arg(a, "helixAngleDeg", 0.0));
  p.undercut = arg(a, "undercut", 0.0) != 0.0;
  p.backlash = arg(a, "backlash", 0.0);
  p.head = arg(a, "head", 0.0);
  p.propertiesFromTool = arg(a, "propertiesFromTool", 0.0) != 0.0;
  const int n = static_cast<int>(arg(a, "sampleCount", 7));

  const auto fr = gear::compute_involute_factors(p);
  if (!fr.ok) {
    emit_error(fr.error);
    return;
  }
  const auto& f = fr.factors;
  emit_value("pressureAngleTransverse", f.pressureAngleTransverse);
  emit_value("transverseModule", f.transverseModule);
  emit_value("transversePitch", f.transversePitch);
  emit_value("clearanceAbs", f.clearanceAbs);
  emit_value("referenceDiameter", f.referenceDiameter);
  emit_value("workingDiameter", f.workingDiameter);
  emit_value("addendumDiameter", f.addendumDiameter);
  emit_value("rootDiameter", f.rootDiameter);
  emit_value("baseDiameter", f.baseDiameter);
  emit_value("angularPitch", f.angularPitch);
  emit_value("undercutEnd", f.undercutEnd);
  emit_value("undercutRot", f.undercutRot);
  emit_value("involuteEnd", f.involuteEnd);
  emit_value("involuteRot1", f.involuteRot1);
  emit_value("involuteRot2", f.involuteRot2);
  emit_value("involuteRot", f.involuteRot);
  emit_value("angularBacklash", f.angularBacklash);
  emit_value("involuteStart", f.involuteStart);

  const auto inv = gear::involute_points(f, n);
  for (std::size_t i = 0; i < inv.size(); ++i) emit_point("involute." + std::to_string(i), inv[i]);
  const auto unc = gear::undercut_points(f, n);
  for (std::size_t i = 0; i < unc.size(); ++i) emit_point("undercut." + std::to_string(i), unc[i]);

  const auto tooth = gear::one_tooth_profile(p, f, n);
  emit_value("toothSegmentCount", static_cast<double>(tooth.segments.size()));
  for (std::size_t s = 0; s < tooth.segments.size(); ++s) {
    emit_value(("tooth" + std::to_string(s) + ".count").c_str(),
               static_cast<double>(tooth.segments[s].size()));
    for (std::size_t i = 0; i < tooth.segments[s].size(); ++i) {
      emit_point("tooth" + std::to_string(s) + "." + std::to_string(i), tooth.segments[s][i]);
    }
  }
}

void run_timing(const Args&, const std::string& profileName, int numTeeth) {
  gear::TimingProfile prof{};
  if (!gear::timing_profile_from_name(profileName, prof)) {
    emit_error("unknown timing profile " + profileName);
    return;
  }
  gear::TimingToothParams p;
  p.profile = prof;
  p.numTeeth = numTeeth;
  const auto fr = gear::compute_timing_factors(p);
  if (!fr.ok) {
    emit_error(fr.error);
    return;
  }
  emit_value("rp", fr.factors.rp);
  emit_value("pitchRadiusTrue", fr.factors.pitchRadiusTrue);
  emit_value("outerRadius", fr.factors.outerRadius);
  emit_value("halfToothAngle", fr.factors.halfToothAngle);
  const auto tr = gear::one_tooth_arcs(p, fr.factors);
  if (!tr.ok) {
    emit_error(tr.error);
    return;
  }
  emit_value("arcCount", static_cast<double>(tr.arcs.size()));
  for (std::size_t i = 0; i < tr.arcs.size(); ++i) {
    const std::string b = "arc" + std::to_string(i);
    emit_point(b + ".start", tr.arcs[i].start);
    emit_point(b + ".mid", tr.arcs[i].mid);
    emit_point(b + ".end", tr.arcs[i].end);
  }
}

void run_timing_t(const Args& a) {
  gear::TimingTParams p;
  p.pitch = arg(a, "pitch", 5.0);
  p.numTeeth = static_cast<int>(arg(a, "teeth", 15));
  p.toothHeight = arg(a, "toothHeight", 1.2);
  p.pitchLineOffset = arg(a, "pitchLineOffset", 0.6);
  p.flankAngle = deg2rad(arg(a, "flankAngleDeg", 40.0));
  p.backlash = arg(a, "backlash", 0.0);
  const auto fr = gear::compute_timing_t_factors(p);
  if (!fr.ok) {
    emit_error(fr.error);
    return;
  }
  emit_value("pitchRadius", fr.factors.pitchRadius);
  emit_value("angularPitch", fr.factors.angularPitch);
  emit_value("halfToothAngle", fr.factors.halfToothAngle);
  emit_value("tipRadius", fr.factors.tipRadius);
  emit_value("rootRadius", fr.factors.rootRadius);
  const auto tr = gear::one_tooth_segments(p, fr.factors);
  if (!tr.ok) {
    emit_error(tr.error);
    return;
  }
  emit_segments(tr.segments);
}

void run_lantern(const Args& a) {
  gear::LanternParams p;
  p.module = arg(a, "module", 1.0);
  p.numTeeth = static_cast<int>(arg(a, "teeth", 15));
  p.rollerRadius = arg(a, "rollerRadius", 1.0);
  p.head = arg(a, "head", 0.0);
  p.sampleCount = static_cast<int>(arg(a, "sampleCount", 10));
  const auto fr = gear::compute_lantern_factors(p);
  if (!fr.ok) {
    emit_error(fr.error);
    return;
  }
  emit_value("pitchRadius", fr.factors.pitchRadius);
  emit_value("maxRadius", fr.factors.maxRadius);
  emit_value("phiMin", fr.factors.phiMin);
  emit_value("phiMax", fr.factors.phiMax);
  const auto tr = gear::one_tooth_segments(p, fr.factors);
  if (!tr.ok) {
    emit_error(tr.error);
    return;
  }
  emit_segments(tr.segments);
}

void run_worm(const Args& a) {
  gear::WormParams p;
  p.module = arg(a, "module", 1.0);
  p.numThreads = static_cast<int>(arg(a, "threads", 3));
  p.diameter = arg(a, "diameter", 5.0);
  p.pressureAngle = deg2rad(arg(a, "pressureAngleDeg", 20.0));
  p.clearance = arg(a, "clearance", 0.25);
  p.head = arg(a, "head", 0.0);
  p.sampleCount = static_cast<int>(arg(a, "sampleCount", 10));
  p.reversePitch = arg(a, "reversePitch", 0.0) != 0.0;
  const auto fr = gear::compute_worm_factors(p);
  if (!fr.ok) {
    emit_error(fr.error);
    return;
  }
  const auto& f = fr.factors;
  emit_value("leadAngle", f.leadAngle);
  emit_value("rootRadius", f.rootRadius);
  emit_value("tipRadius", f.tipRadius);
  emit_value("z0", f.z0);
  emit_value("z1", f.z1);
  emit_value("z2", f.z2);
  emit_value("z3", f.z3);
  emit_value("z4", f.z4);
  emit_value("phi0", f.phi0);
  emit_value("phi1", f.phi1);
  emit_value("phi2", f.phi2);
  emit_value("phi3", f.phi3);
  emit_value("phi4", f.phi4);
  const auto tr = gear::one_tooth_segments(p, f);
  if (!tr.ok) {
    emit_error(tr.error);
    return;
  }
  emit_segments(tr.segments);
}

}  // namespace

int main() {
  std::string line;
  int lineNo = 0;
  while (std::getline(std::cin, line)) {
    ++lineNo;
    if (line.empty() || line[0] == '#') continue;
    std::istringstream is(line);
    std::string recipe;
    is >> recipe;
    Args args;
    std::string profileName;
    std::string tok;
    while (is >> tok) {
      const auto eq = tok.find('=');
      if (eq == std::string::npos) continue;
      const std::string key = tok.substr(0, eq);
      const std::string val = tok.substr(eq + 1);
      if (key == "profile") {
        profileName = val;
      } else {
        args[key] = std::stod(val);
      }
    }
    std::printf("CASE %d\n", lineNo);
    if (recipe == "involute") {
      run_involute(args);
    } else if (recipe == "timing") {
      run_timing(args, profileName, static_cast<int>(arg(args, "teeth", 15)));
    } else if (recipe == "timingT") {
      run_timing_t(args);
    } else if (recipe == "lantern") {
      run_lantern(args);
    } else if (recipe == "worm") {
      run_worm(args);
    } else {
      emit_error("unknown recipe " + recipe);
    }
    std::printf("END\n");
  }
  return 0;
}
