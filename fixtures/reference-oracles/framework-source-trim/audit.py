#!/usr/bin/env python3
"""Inspect a pinned Framework face's endpoint trim assumptions using STEPCAF."""
from __future__ import annotations

import hashlib
import json
import math
import platform
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
SCRIPTS = ROOT / "fixtures/reference-histories"
SOURCE = ROOT / "fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp"
PINNED = {
    SOURCE: "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09",
    SCRIPTS / "extract_framework_open_arcs.py": "099dddfb0795ba9b12352201a0db84f68667584b621e4c5df89c3551ce0b2819",
    SCRIPTS / "inspect_framework_residual.py": "a3c89c9e90d10dd4dddce738eb9fe2efee98f3b0913fcfc3f9ded5d28294512d",
    SCRIPTS / "inspect_framework.py": "16ae4dad44c646f707207ef6245086af92717ac27190e7bd968e5f97b1736696",
}


def digest(path: Path) -> str:
    with path.open("rb") as source:
        h = hashlib.sha256()
        for block in iter(lambda: source.read(65536), b""):
            h.update(block)
        return h.hexdigest()


def verify(path: Path, expected: str) -> None:
    if digest(path) != expected:
        raise ValueError(f"SHA-256 mismatch: {path}")


def point(p: Any) -> list[float]:
    return [p.X(), p.Y(), p.Z()]


def uv(p: Any) -> list[float]:
    return [p.X(), p.Y()]


def pcurve_data(adaptor: Any) -> dict[str, Any]:
    if not str(adaptor.GetType()).endswith("GeomAbs_BSplineCurve"):
        raise ValueError("unsupported pcurve type")
    curve = adaptor.BSpline()
    if curve.Degree() != 1 or curve.IsRational() or curve.IsPeriodic():
        raise ValueError("unsupported pcurve geometry")
    return {
        "degree": curve.Degree(),
        "rational": curve.IsRational(),
        "periodic": curve.IsPeriodic(),
        "poles": [uv(curve.Pole(i)) for i in range(1, curve.NbPoles() + 1)],
        "knots": [
            {"value": curve.Knot(i), "multiplicity": curve.Multiplicity(i)}
            for i in range(1, curve.NbKnots() + 1)
        ],
    }


def edge_data(face: Any, edge: Any) -> dict[str, Any]:
    from OCP.BRep import BRep_Tool
    from OCP.BRepAdaptor import BRepAdaptor_Curve
    from OCP.BRepTools import BRepTools
    from OCP.Geom2dAdaptor import Geom2dAdaptor_Curve

    curve = BRepAdaptor_Curve(edge)
    t0, t1 = BRep_Tool.Range_s(edge, face)
    pcurve = BRep_Tool.CurveOnSurface_s(edge, face, 0.0, 0.0)
    if pcurve is None:
        raise ValueError("source edge lacks a pcurve")
    result: dict[str, Any] = {
        "orientation": str(edge.Orientation()),
        "curve3dKind": str(curve.GetType()),
        "pcurve": pcurve_data(Geom2dAdaptor_Curve(pcurve)),
        "pcurveRange": [t0, t1],
        "pcurveEndpoints": [uv(pcurve.Value(t0)), uv(pcurve.Value(t1))],
        "uvBounds": list(BRepTools.UVBounds_s(face, edge)),
        "edgeToleranceMm": BRep_Tool.Tolerance_s(edge),
        "edge3dEnds": [point(curve.Value(curve.FirstParameter())), point(curve.Value(curve.LastParameter()))],
    }
    if str(curve.GetType()).endswith("GeomAbs_Circle"):
        circle = curve.Circle()
        direction = circle.Axis().Direction()
        center = point(circle.Location())
        result["circle"] = {
            "centerMm": center, "radiusMm": circle.Radius(),
            "axis": [direction.X(), direction.Y(), direction.Z()],
        }
    return result


def wire_data(face: Any) -> list[dict[str, Any]]:
    from OCP.BRepTools import BRepTools, BRepTools_WireExplorer
    from OCP.TopAbs import TopAbs_WIRE
    from OCP.TopExp import TopExp_Explorer
    from OCP.TopoDS import TopoDS

    result = []
    outer = BRepTools.OuterWire_s(face)
    wires = TopExp_Explorer(face, TopAbs_WIRE)
    while wires.More():
        wire = TopoDS.Wire(wires.Current())
        walker = BRepTools_WireExplorer(wire, face)
        edges = []
        while walker.More():
            edges.append(edge_data(face, walker.Current()))
            walker.Next()
        result.append({"outer": wire.IsSame(outer), "orientation": str(wire.Orientation()), "edges": edges})
        wires.Next()
    return result


def validate_linear_pcurve(edge: dict[str, Any]) -> list[float]:
    try:
        curve = edge["pcurve"]
        poles, knots, interval = curve["poles"], curve["knots"], edge["pcurveRange"]
        if (curve["degree"] != 1 or curve["rational"] is not False or curve["periodic"] is not False
                or len(poles) != 2 or any(len(p) != 2 for p in poles)
                or len(knots) != 2 or len(interval) != 2):
            raise ValueError("unsupported pcurve structure")
        values = [*poles[0], *poles[1], knots[0]["value"], knots[1]["value"], *interval]
        if not all(math.isfinite(x) for x in values):
            raise ValueError("non-finite pcurve data")
        k0, k1 = knots[0]["value"], knots[1]["value"]
        if (knots[0]["multiplicity"] != 2 or knots[1]["multiplicity"] != 2
                or not k0 < k1 or not k0 <= interval[0] < interval[1] <= k1):
            raise ValueError("pcurve interval outside clamped knot domain")
        return [poles[0][1], poles[1][1]]
    except (KeyError, TypeError, IndexError) as error:
        raise ValueError("malformed pcurve data") from error


def validate_endpoint_hypothesis(wires: list[dict[str, Any]]) -> dict[str, Any]:
    if len(wires) != 1 or not wires[0]["outer"] or len(wires[0]["edges"]) != 4:
        raise ValueError("unexpected face-wire graph")
    edges = wires[0]["edges"]
    if not edges[0]["curve3dKind"].endswith("GeomAbs_Circle"):
        raise ValueError("expected edge 0 to be circular")
    edge0 = validate_linear_pcurve(edges[0])
    edge2 = validate_linear_pcurve(edges[2])
    if not all(x > 0.0 for x in edge0) or not all(x > 1.0 for x in edge2):
        raise ValueError("edge 0 or edge 2 parameter observations differ")
    return {"vZeroIsEdge0": False, "vOneIsEdge2": False,
            "basis": "Transferred edge 0 and edge 2 use nonrational degree-one, two-pole pcurves within clamped knot domains; their v poles are strictly above 0 and 1 respectively.",
            "limits": "Edge numbering follows transferred wire traversal; opposite-end intent and material ownership are not established."}


def run() -> dict[str, Any]:
    from OCP import __version__
    from OCP.BRepAdaptor import BRepAdaptor_Surface
    from OCP.BRepClass import BRepClass_FaceClassifier
    from OCP.BRepTools import BRepTools
    from OCP.gp import gp_Pnt2d

    if __version__ != "8.0.1.1" or platform.machine() != "x86_64":
        raise ValueError("this diagnostic requires the recorded x86_64 OCP 8.0.1.1 environment")
    for path, expected in PINNED.items():
        verify(path, expected)
    sys.path.insert(0, str(SCRIPTS))
    from extract_framework_open_arcs import faces_from_source
    from inspect_framework_residual import PRODUCT

    if PRODUCT != "FW_EXP_1USBC_FRAME_CLIP_BC_229_":
        raise ValueError("unexpected named source product")
    faces = faces_from_source(SOURCE)
    if len(faces) != 179:
        raise ValueError("unexpected transferred face count")
    face = faces[66]
    surface = BRepAdaptor_Surface(face)
    if not str(surface.GetType()).endswith("GeomAbs_BSplineSurface"):
        raise ValueError("unexpected face surface")
    wires = wire_data(face)
    hypothesis = validate_endpoint_hypothesis(wires)
    samples = [
        {"uv": [u, v], "state": str(BRepClass_FaceClassifier(face, gp_Pnt2d(u, v), 1e-7).State())}
        for v in (0.0, 1.0) for u in (0.22, 0.5, 0.78)
    ]
    for path, expected in PINNED.items():
        verify(path, expected)
    return {
        "sourceProduct": PRODUCT, "sourceSha256Before": PINNED[SOURCE], "sourceSha256After": digest(SOURCE),
        "generatorSha256": digest(Path(__file__)), "helperSha256": {str(p.relative_to(ROOT)): h for p, h in PINNED.items() if p != SOURCE},
        "reader": "STEPCAF via named-product helper, transformed (x,y,z) to (x,-z,y)",
        "ocpVersion": __version__, "architecture": platform.machine(), "faceIndex": 66,
        "faceIndexMeaning": "zero-based TopExp_Explorer index after transfer; not a STEP entity number",
        "surfaceKind": str(surface.GetType()), "uvBounds": list(BRepTools.UVBounds_s(face)),
        "faceOrientation": str(face.Orientation()), "wires": wires, "endpointHypothesis": hypothesis,
        "sampleClassificationTolerance": 1e-7, "sampleClassification": samples,
        "scope": "Transferred source topology only; no independent STEP interpretation, material ownership, source-equivalence, or accuracy qualification.",
    }


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: audit.py NEW_OUTPUT.json")
    with Path(sys.argv[1]).open("x", encoding="utf-8") as output:
        json.dump(run(), output, indent=2, allow_nan=False)
        output.write("\n")


if __name__ == "__main__":
    main()
