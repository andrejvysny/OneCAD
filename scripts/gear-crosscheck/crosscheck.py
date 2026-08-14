#!/usr/bin/env python3
"""Cross-check OneCAD's gear samplers against the freecad.gears reference.

This is the Gear Generator plan's §10.1 leg 2: equivalence evidence against a
shipping implementation, to complement the closed-form analytic oracles (leg 1)
and the kernel-level geometry checks (leg 3).

LOCAL ONLY.  This script drives the GPL-3.0 `freecad.gears` reference, which is
deliberately never vendored into this repo and never wired into CI — see
README.md for the licensing and determinism reasons.  Nothing in the OneCAD
build or test lanes depends on this file.

Usage:
    python3 crosscheck.py --pygears /path/to/freecad.gears

It compiles scripts/gear-crosscheck/gear_dump.cpp with a bare g++ (the gear
samplers have no OCCT dependency by construction), feeds it a parameter sweep,
and diffs every emitted value against the reference computed in-process.

Exit status is 0 only when every case agrees within tolerance.
"""

from __future__ import annotations

import argparse
import math
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
WORKER_SRC = os.path.join(REPO_ROOT, "worker", "src")
DUMP_SRC = os.path.join(os.path.dirname(__file__), "gear_dump.cpp")

# Tolerances are per-quantity because the recipes differ in how they are solved.
# `timingT` and `lantern` are looser against the REFERENCE only: this port
# solves their 1-D problems in closed form / with a bracketed solver, while
# upstream uses scipy minimisers that converge to ~1e-6..1e-8 (measured).  The
# analytic oracles in the C++ unit tests hold those same quantities to 1e-9,
# so the loose bound here is upstream's error budget, not this port's.
TOLERANCE = {
    "involute": 1e-9,
    "timing": 1e-9,
    "timingT": 5e-5,
    "lantern": 1e-6,
    "worm": 1e-9,
}


@dataclass
class Case:
    recipe: str
    params: dict
    def line(self) -> str:
        parts = [self.recipe]
        for k, v in self.params.items():
            parts.append(f"{k}={v}")
        return " ".join(parts)


@dataclass
class Result:
    values: dict = field(default_factory=dict)
    points: dict = field(default_factory=dict)
    error: str | None = None


# --------------------------------------------------------------------------
# Sweep definition
# --------------------------------------------------------------------------

def build_sweep() -> list[Case]:
    cases: list[Case] = []

    # involute: the widest sweep — this is the recipe with the most branches
    # (undercut on/off, dg vs df, propertiesFromTool, shift, backlash, head).
    for module in (0.5, 1.0, 2.0, 3.0):
        for teeth in (8, 12, 15, 20, 30, 60):
            for alpha in (14.5, 20.0, 25.0):
                for shift in (-0.3, 0.0, 0.5, 1.0):
                    for undercut in (0, 1):
                        for head in (0.0, 0.3):
                            for backlash in (0.0, 0.1):
                                cases.append(Case("involute", dict(
                                    module=module, teeth=teeth, pressureAngleDeg=alpha,
                                    shift=shift, undercut=undercut, head=head,
                                    backlash=backlash, clearance=0.25, sampleCount=7)))
    # propertiesFromTool branch (transverse recompute from a helix angle).
    for beta in (10.0, 25.0, 40.0):
        for teeth in (12, 20, 40):
            cases.append(Case("involute", dict(
                module=2.0, teeth=teeth, pressureAngleDeg=20.0, shift=0.2,
                helixAngleDeg=beta, propertiesFromTool=1, clearance=0.25, sampleCount=7)))

    # timing: every belt section over a range of tooth counts.
    for profile in ("gt2", "gt3", "gt5", "gt8", "htd3", "htd5", "htd8"):
        for teeth in (10, 15, 20, 24, 36, 60):
            cases.append(Case("timing", dict(profile=profile, teeth=teeth)))

    # timingT
    for pitch in (2.5, 5.0, 10.0):
        for teeth in (10, 15, 30, 60):
            for alpha in (20.0, 40.0, 50.0):
                for backlash in (0.0, 0.1):
                    cases.append(Case("timingT", dict(
                        pitch=pitch, teeth=teeth, flankAngleDeg=alpha, backlash=backlash,
                        toothHeight=pitch * 0.24, pitchLineOffset=pitch * 0.12)))

    # lantern
    for module in (0.5, 1.0, 2.0):
        for teeth in (8, 15, 30):
            for roller in (0.3, 1.0, 1.5):
                for head in (0.0, 0.2):
                    cases.append(Case("lantern", dict(
                        module=module, teeth=teeth, rollerRadius=roller, head=head,
                        sampleCount=8)))

    # worm
    for module in (0.5, 1.0, 2.0):
        for threads in (1, 2, 3, 5):
            for diameter in (5.0, 12.0, 25.0):
                for alpha in (14.5, 20.0, 30.0):
                    for head in (0.0, 0.1):
                        cases.append(Case("worm", dict(
                            module=module, threads=threads, diameter=diameter,
                            pressureAngleDeg=alpha, head=head, clearance=0.25,
                            sampleCount=8)))
    return cases


# --------------------------------------------------------------------------
# Reference side
# --------------------------------------------------------------------------

def reference_involute(p, np, InvoluteTooth) -> Result:
    r = Result()
    t = InvoluteTooth(
        m=p["module"], num_teeth=int(p["teeth"]),
        pressure_angle=math.radians(p["pressureAngleDeg"]),
        clearance=p.get("clearance", 0.25), shift=p.get("shift", 0.0),
        beta=math.radians(p.get("helixAngleDeg", 0.0)),
        undercut=bool(p.get("undercut", 0)), backlash=p.get("backlash", 0.0),
        head=p.get("head", 0.0),
        properties_from_tool=bool(p.get("propertiesFromTool", 0)))
    mapping = {
        "pressureAngleTransverse": "pressure_angle_t", "transverseModule": "m",
        "transversePitch": "pitch", "clearanceAbs": "c", "referenceDiameter": "d",
        "workingDiameter": "dw", "addendumDiameter": "da", "rootDiameter": "df",
        "baseDiameter": "dg", "angularPitch": "phipart", "undercutEnd": "undercut_end",
        "undercutRot": "undercut_rot", "involuteEnd": "involute_end",
        "involuteRot1": "involute_rot1", "involuteRot2": "involute_rot2",
        "involuteRot": "involute_rot", "angularBacklash": "angular_backlash",
        "involuteStart": "involute_start",
    }
    for ours, theirs in mapping.items():
        r.values[ours] = float(getattr(t, theirs))
    n = int(p.get("sampleCount", 7))
    for i, q in enumerate(t.involute_points(num=n)):
        r.points[f"involute.{i}"] = (float(q[0]), float(q[1]))
    for i, q in enumerate(t.undercut_points(num=n)):
        r.points[f"undercut.{i}"] = (float(q[0]), float(q[1]))
    tooth = t.points(num=n)
    r.values["toothSegmentCount"] = float(len(tooth))
    for s, seg in enumerate(tooth):
        pts = [list(map(float, q)) for q in seg]
        r.values[f"tooth{s}.count"] = float(len(pts))
        for i, q in enumerate(pts):
            r.points[f"tooth{s}.{i}"] = (q[0], q[1])
    return r


TIMING_DATA = {
    "gt2":  dict(pitch=2.0, u=0.254, h=0.75, r0=0.555, r1=1.0, rs=0.15, offset=0.40),
    "gt3":  dict(pitch=3.0, u=0.381, h=1.14, r0=0.85, r1=1.52, rs=0.25, offset=0.61),
    "gt5":  dict(pitch=5.0, u=0.5715, h=1.93, r0=1.44, r1=2.57, rs=0.416, offset=1.03),
    "gt8":  dict(pitch=8.0, u=0.9144, h=3.088, r0=2.304, r1=4.112, rs=0.6656, offset=1.648),
    "htd3": dict(pitch=3.0, u=0.381, h=1.21, r0=0.89, r1=0.89, rs=0.26, offset=0.0),
    "htd5": dict(pitch=5.0, u=0.5715, h=2.06, r0=1.49, r1=1.49, rs=0.43, offset=0.0),
    "htd8": dict(pitch=8.0, u=0.686, h=3.45, r0=2.46, r1=2.46, rs=0.70, offset=0.0),
}


def reference_timing(p, np, reflection, arc_from_points_and_center) -> Result:
    """Transcribed from freecad/gears/timinggear.py generate_gear_shape.

    Reproduced here rather than imported because upstream's version is welded
    to FreeCAD's Part API (it builds shapes, not numbers); only the arithmetic
    is portable.  Keeping it in the harness rather than in the C++ test is the
    point — this file is the one place the reference lives.
    """
    r = Result()
    gt = TIMING_DATA[p["profile"]]
    teeth = int(p["teeth"])
    pitch, h, u = gt["pitch"], gt["h"], gt["u"]
    r_12, r_23, r_34, offset = gt["r0"], gt["r1"], gt["rs"], gt["offset"]
    arcs = []
    if offset == 0.0:
        phi5 = np.pi / teeth
        ref = reflection(-phi5 - np.pi / 2.0)
        rp = pitch * teeth / np.pi / 2.0 - u
        m_34 = np.array([-(r_12 + r_34), rp - h + r_12])
        x2 = np.array([-r_12, m_34[1]])
        x4 = np.array([m_34[0], m_34[1] + r_34])
        x6 = ref(x4)
        mir = np.array([-1.0, 1.0])
        xn2, xn4, mn_34 = mir * x2, mir * x4, mir * m_34
        arcs.append(arc_from_points_and_center(xn4, xn2, mn_34))
        arcs.append((xn2, np.array([0.0, rp - h]), x2))
        arcs.append(arc_from_points_and_center(x2, x4, m_34))
        arcs.append(arc_from_points_and_center(x4, x6, np.array([0.0, 0.0])))
    else:
        phi_12 = np.arctan(np.sqrt(1.0 / (((r_12 - r_23) / offset) ** 2 - 1)))
        rp = pitch * teeth / np.pi / 2.0
        r4 = r5 = rp - u
        m_12 = np.array([0.0, r5 - h + r_12])
        m_23 = np.array([offset, offset / np.tan(phi_12) + m_12[1]])
        y = m_23[1]
        phi4 = 2 * np.arctan(
            (-2 * offset * r5 + 2 * offset * r_34 + np.sqrt(
                -(y ** 4) - 2 * y ** 2 * offset ** 2 + 2 * y ** 2 * r5 ** 2
                - 4 * y ** 2 * r5 * r_34 + 2 * y ** 2 * r_23 ** 2
                + 4 * y ** 2 * r_23 * r_34 + 4 * y ** 2 * r_34 ** 2
                - offset ** 4 + 2 * offset ** 2 * r5 ** 2 - 4 * offset ** 2 * r5 * r_34
                + 2 * offset ** 2 * r_23 ** 2 + 4 * offset ** 2 * r_23 * r_34
                + 4 * offset ** 2 * r_34 ** 2 - r5 ** 4 + 4 * r5 ** 3 * r_34
                + 2 * r5 ** 2 * r_23 ** 2 + 4 * r5 ** 2 * r_23 * r_34
                - 4 * r5 ** 2 * r_34 ** 2 - 4 * r5 * r_23 ** 2 * r_34
                - 8 * r5 * r_23 * r_34 ** 2 - r_23 ** 4 - 4 * r_23 ** 3 * r_34
                - 4 * r_23 ** 2 * r_34 ** 2))
            / (y ** 2 + 2 * y * r5 - 2 * y * r_34 + offset ** 2 + r5 ** 2
               - 2 * r5 * r_34 - r_23 ** 2 - 2 * r_23 * r_34))
        phi5 = np.pi / teeth
        m_34 = (r5 - r_34) * np.array([-np.sin(phi4), np.cos(phi4)])
        x2 = np.array([-r_12 * np.sin(phi_12), m_12[1] - r_12 * np.cos(phi_12)])
        x3 = m_34 + r_34 / (r_34 + r_23) * (m_23 - m_34)
        x4 = r4 * np.array([-np.sin(phi4), np.cos(phi4)])
        ref = reflection(-phi5 - np.pi / 2)
        x6 = ref(x4)
        mir = np.array([-1.0, 1.0])
        xn2, xn3, xn4 = mir * x2, mir * x3, mir * x4
        mn_34, mn_23 = mir * m_34, mir * m_23
        arcs.append(arc_from_points_and_center(xn4, xn3, mn_34))
        arcs.append(arc_from_points_and_center(xn3, xn2, mn_23))
        arcs.append(arc_from_points_and_center(xn2, x2, m_12))
        arcs.append(arc_from_points_and_center(x2, x3, m_23))
        arcs.append(arc_from_points_and_center(x3, x4, m_34))
        arcs.append(arc_from_points_and_center(x4, x6, np.array([0.0, 0.0])))
    r.values["rp"] = float(rp)
    r.values["arcCount"] = float(len(arcs))
    for i, a in enumerate(arcs):
        for label, q in zip(("start", "mid", "end"), a):
            r.points[f"arc{i}.{label}"] = (float(q[0]), float(q[1]))
    return r


def reference_timing_t(p, np, sp, reflection, rotation) -> Result:
    """Transcribed from freecad/gears/timinggear_t.py generate_gear_shape."""
    r = Result()
    pitch = p["pitch"]
    teeth = int(p["teeth"])
    u = p["pitchLineOffset"]
    tooth_height = p["toothHeight"]
    alpha = math.radians(p["flankAngleDeg"])
    backlash = p.get("backlash", 0.0)
    r_p = pitch * teeth / 2.0 / np.pi
    gamma_0 = pitch / r_p
    gamma_1 = gamma_0 / 4 - backlash / r_p
    p_A = np.array([np.cos(-gamma_1), np.sin(-gamma_1)]) * (r_p - u - tooth_height / 2)

    def line(s):
        return p_A + np.array([np.cos(alpha / 2 - gamma_1), np.sin(alpha / 2 - gamma_1)]) * s

    s1 = sp.optimize.minimize(
        lambda s: (np.linalg.norm(line(s)) - (r_p - u - tooth_height)) ** 2, 0.0).x
    s2 = sp.optimize.minimize(lambda s: (np.linalg.norm(line(s)) - (r_p - u)) ** 2, 0.0).x
    p_1, p_2 = line(s1), line(s2)
    p_3, p_4 = reflection(0.0)(np.array([p_2, p_1]))
    p_5 = rotation(gamma_0)(np.array([p_1]))[0]
    r.values["pitchRadius"] = float(r_p)
    r.values["angularPitch"] = float(gamma_0)
    r.values["halfToothAngle"] = float(gamma_1)
    r.values["tipRadius"] = float(r_p - u)
    r.values["rootRadius"] = float(r_p - u - tooth_height)
    for name, q in (("seg0.start", p_1), ("seg0.end", p_2), ("seg2.start", p_3),
                    ("seg2.end", p_4), ("seg3.end", p_5)):
        r.points[name] = (float(q[0]), float(q[1]))
    return r


def reference_lantern(p, np, sp) -> Result:
    """Transcribed from freecad/gears/lanterngear.py generate_gear_shape."""
    r = Result()
    m, teeth, r_r = p["module"], int(p["teeth"]), p["rollerRadius"]
    head = p.get("head", 0.0)
    r_0 = m * teeth / 2
    r_max = r_0 + r_r + head * m
    phi_max = (r_r + np.sqrt(r_max ** 2 - r_0 ** 2)) / r_0

    def f(phi):
        return r_0 * (phi ** 2 * r_0 - 2 * phi * r_0 * np.sin(phi) - 2 * phi * r_r
                      - 2 * r_0 * np.cos(phi) + 2 * r_0 + 2 * r_r * np.sin(phi))

    phi_min = sp.optimize.root(f, (phi_max + r_r / r_0 * 4) / 5).x[0]
    n = int(p.get("sampleCount", 10))
    phi = np.linspace(phi_min, phi_max, n)
    x = r_0 * (np.cos(phi) + phi * np.sin(phi)) - r_r * np.sin(phi)
    y = r_0 * (np.sin(phi) - phi * np.cos(phi)) + r_r * np.cos(phi)
    r.values["pitchRadius"] = float(r_0)
    r.values["maxRadius"] = float(r_max)
    r.values["phiMin"] = float(phi_min)
    r.values["phiMax"] = float(phi_max)
    for i in range(n):
        r.points[f"seg2.p{i}"] = (float(x[i]), float(y[i]))
    return r


def reference_worm(p, np) -> Result:
    """Transcribed from freecad/gears/wormgear.py generate_gear_shape."""
    r = Result()
    m, d, t = p["module"], p["diameter"], int(p["threads"])
    clearance, head = p.get("clearance", 0.25), p.get("head", 0.0)
    alpha = p["pressureAngleDeg"]
    tan_a = np.tan(np.deg2rad(alpha))
    r_1 = (d - (2 + 2 * clearance) * m) / 2
    r_2 = (d + (2 + 2 * head) * m) / 2
    z_a = (2 + head + clearance) * m * tan_a
    z_b = (m * np.pi - 4 * m * tan_a) / 2
    z_0 = clearance * m * tan_a
    z_1 = z_b - z_0
    z_2 = z_1 + z_a
    z_3 = z_2 + z_b - 2 * head * m * tan_a
    z_4 = z_3 + z_a
    r.values["leadAngle"] = float(np.arctan(m * t / d))
    r.values["rootRadius"] = float(r_1)
    r.values["tipRadius"] = float(r_2)
    for i, z in enumerate((z_0, z_1, z_2, z_3, z_4)):
        r.values[f"z{i}"] = float(z)
        r.values[f"phi{i}"] = float(2 * z / m / t)
    n = int(p.get("sampleCount", 10))
    for seg, (rf, rt, zf, zt) in ((1, (r_1, r_2, z_1, z_2)), (3, (r_2, r_1, z_3, z_4))):
        rv = np.linspace(rf, rt, n)
        zv = np.linspace(zf, zt, n)
        phi = 2 * zv / m / t
        for i in range(n):
            r.points[f"seg{seg}.p{i}"] = (float(rv[i] * np.cos(phi[i])),
                                          float(rv[i] * np.sin(phi[i])))
    return r


# --------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------

def parse_dump(text: str) -> list[Result]:
    out: list[Result] = []
    cur: Result | None = None
    for line in text.splitlines():
        if line.startswith("CASE "):
            cur = Result()
        elif line == "END":
            out.append(cur if cur is not None else Result())
            cur = None
        elif cur is None:
            continue
        elif line.startswith("V "):
            _, name, val = line.split(" ", 2)
            cur.values[name] = float(val)
        elif line.startswith("P "):
            _, name, x, y = line.split(" ", 3)
            cur.points[name] = (float(x), float(y))
        elif line.startswith("ERR "):
            cur.error = line[4:]
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pygears", required=True,
                    help="path to a freecad.gears checkout (the dir containing pygears/)")
    ap.add_argument("--cxx", default=os.environ.get("CXX", "g++"))
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--show-refusals", action="store_true",
                    help="list the cases OneCAD refuses. Worth reviewing: a refusal the "
                         "reference accepts is either a deliberate fail-closed guard or "
                         "over-strictness, and the difference matters.")
    args = ap.parse_args()

    sys.path.insert(0, os.path.abspath(args.pygears))
    try:
        import numpy as np
        import scipy as sp
        from scipy import optimize  # noqa: F401  (registers sp.optimize)
        from pygears.involute_tooth import InvoluteTooth
        from pygears._functions import reflection, rotation, arc_from_points_and_center
    except ImportError as exc:
        print(f"error: could not import the reference: {exc}", file=sys.stderr)
        print("       need numpy, scipy and a freecad.gears checkout (--pygears).",
              file=sys.stderr)
        return 2

    if not shutil.which(args.cxx):
        print(f"error: compiler {args.cxx!r} not found", file=sys.stderr)
        return 2

    sources = [
        DUMP_SRC,
        os.path.join(WORKER_SRC, "kernel", "gear", "InvoluteMath.cpp"),
        os.path.join(WORKER_SRC, "kernel", "gear", "TimingMath.cpp"),
        os.path.join(WORKER_SRC, "kernel", "gear", "TimingTMath.cpp"),
        os.path.join(WORKER_SRC, "kernel", "gear", "LanternMath.cpp"),
        os.path.join(WORKER_SRC, "kernel", "gear", "WormMath.cpp"),
    ]
    with tempfile.TemporaryDirectory() as tmp:
        binary = os.path.join(tmp, "gear_dump")
        cmd = [args.cxx, "-std=c++20", "-O2", "-I", WORKER_SRC, *sources, "-o", binary]
        print("compiling:", " ".join(cmd[:3]), "...", file=sys.stderr)
        proc = subprocess.run(cmd, capture_output=True, text=True)
        if proc.returncode != 0:
            print(proc.stderr, file=sys.stderr)
            print("error: gear_dump failed to compile standalone. The gear samplers are "
                  "supposed to have NO OCCT dependency — check what was just added.",
                  file=sys.stderr)
            return 2

        cases = build_sweep()
        stdin = "\n".join(c.line() for c in cases) + "\n"
        proc = subprocess.run([binary], input=stdin, capture_output=True, text=True)
        if proc.returncode != 0:
            print(proc.stderr, file=sys.stderr)
            return 2
        ours = parse_dump(proc.stdout)

    if len(ours) != len(cases):
        print(f"error: expected {len(cases)} results, got {len(ours)}", file=sys.stderr)
        return 2

    compared = refused = mismatched = 0
    failures: list[str] = []
    refusals: list[str] = []
    for case, mine in zip(cases, ours):
        if mine.error is not None:
            refused += 1
            refusals.append(f"  {case.line()}\n      -> {mine.error}")
            continue
        p = case.params
        try:
            if case.recipe == "involute":
                theirs = reference_involute(p, np, InvoluteTooth)
            elif case.recipe == "timing":
                theirs = reference_timing(p, np, reflection, arc_from_points_and_center)
            elif case.recipe == "timingT":
                theirs = reference_timing_t(p, np, sp, reflection, rotation)
            elif case.recipe == "lantern":
                theirs = reference_lantern(p, np, sp)
            elif case.recipe == "worm":
                theirs = reference_worm(p, np)
            else:
                continue
        except Exception as exc:  # the reference itself blew up on this input
            failures.append(f"{case.line()}: reference raised {exc!r}")
            mismatched += 1
            continue

        tol = TOLERANCE[case.recipe]
        bad: list[str] = []
        for name, want in theirs.values.items():
            if name not in mine.values:
                continue
            got = mine.values[name]
            if not math.isfinite(got) or abs(got - want) > tol * max(1.0, abs(want)):
                bad.append(f"    {name}: ours={got!r} theirs={want!r}")
        for name, (wx, wy) in theirs.points.items():
            if name not in mine.points:
                continue
            gx, gy = mine.points[name]
            if (abs(gx - wx) > tol * max(1.0, abs(wx))
                    or abs(gy - wy) > tol * max(1.0, abs(wy))):
                bad.append(f"    {name}: ours=({gx!r},{gy!r}) theirs=({wx!r},{wy!r})")
        compared += 1
        if bad:
            mismatched += 1
            failures.append(f"{case.line()}\n" + "\n".join(bad[:6]))

    print(f"\ncases={len(cases)} compared={compared} refused-by-ours={refused} "
          f"mismatched={mismatched}")
    if args.show_refusals and refusals:
        print(f"\n{len(refusals)} case(s) refused by OneCAD:")
        for r in refusals:
            print(r)
    if failures:
        print(f"\n{len(failures)} mismatching case(s):", file=sys.stderr)
        for f in failures[: (len(failures) if args.verbose else 15)]:
            print(f, file=sys.stderr)
        if not args.verbose and len(failures) > 15:
            print(f"  ... and {len(failures) - 15} more (--verbose for all)", file=sys.stderr)
        return 1
    print("AGREEMENT: every compared value matches the reference within tolerance.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
