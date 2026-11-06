#!/usr/bin/env python3
"""Extract editable planar curve inputs from the pinned NIST FTC09 STEP.

This is geometry extraction, not an independent oracle: OCP wraps OCCT. The
checked JSON is a recipe input; source-authored validation properties remain in
fixtures/reference-oracles/nist-ftc09.json and are never rewritten here.
"""
from __future__ import annotations

import hashlib
import json
import math
import sys
from pathlib import Path

from OCP.BRepAdaptor import BRepAdaptor_Curve, BRepAdaptor_Surface
from OCP.BRepTools import BRepTools_WireExplorer
from OCP.GeomAbs import GeomAbs_Circle, GeomAbs_Cone, GeomAbs_Line, GeomAbs_Plane
from OCP.STEPControl import STEPControl_Reader
from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_WIRE
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS

EXPECTED = "d12866d3ab0fc260efd0f1a5c54af47950be513486f61f65513877045582afa8"


def xy(point: object) -> list[float]:
    return [point.X(), point.Z()]


def arc(curve: BRepAdaptor_Curve, start: float, end: float) -> dict[str, object]:
    circle = curve.Circle()
    center = xy(circle.Location())
    def angle(parameter: float) -> float:
        point = xy(curve.Value(parameter))
        return math.atan2(point[1] - center[1], point[0] - center[0]) % (2 * math.pi)
    first, mid, last = angle(start), angle((start + end) / 2), angle(end)
    if (mid - first) % (2 * math.pi) > (last - first) % (2 * math.pi):
        first, last = last, first
    sweep = (last - first) % (2 * math.pi)
    if sweep < 1e-12:
        raise ValueError("unsupported full-circle edge")
    return {"kind": "arc", "center": center, "radius": circle.Radius(), "start": first, "end": first + sweep}


def wire_curves(face: object) -> list[list[dict[str, object]]]:
    wires: list[list[dict[str, object]]] = []
    explorer = TopExp_Explorer(face, TopAbs_WIRE)
    while explorer.More():
        wire = TopoDS.Wire(explorer.Current())
        edges = BRepTools_WireExplorer(wire, face)
        curves: list[dict[str, object]] = []
        while edges.More():
            curve = BRepAdaptor_Curve(TopoDS.Edge(edges.Current()))
            start, end = curve.FirstParameter(), curve.LastParameter()
            if curve.GetType() == GeomAbs_Line:
                curves.append({"kind": "line", "start": xy(curve.Value(start)), "end": xy(curve.Value(end))})
            elif curve.GetType() == GeomAbs_Circle:
                curves.append(arc(curve, start, end))
            else:
                raise ValueError(f"unsupported source curve {curve.GetType()}")
            edges.Next()
        if not curves:
            raise ValueError("empty source wire")
        if len(curves) == 2 and all(curve["kind"] == "arc" for curve in curves):
            first, second = curves
            same_center = all(abs(a - b) < 1e-8 for a, b in zip(first["center"], second["center"]))
            same_radius = abs(first["radius"] - second["radius"]) < 1e-8
            sweep = sum(curve["end"] - curve["start"] for curve in curves)
            if same_center and same_radius and abs(sweep - 2 * math.pi) < 1e-8:
                curves = [{"kind": "circle", "center": first["center"], "radius": first["radius"]}]
        wires.append(curves)
        explorer.Next()
    return wires


def extract(source: Path) -> dict[str, object]:
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    if digest != EXPECTED:
        raise ValueError(f"source checksum mismatch: {digest}")
    reader = STEPControl_Reader()
    if str(reader.ReadFile(str(source))) != "IFSelect_ReturnStatus.IFSelect_RetDone":
        raise ValueError("STEP reader rejected source")
    reader.TransferRoots()
    faces = TopExp_Explorer(reader.OneShape(), TopAbs_FACE)
    selected: dict[int, object] = {}
    countersinks: dict[tuple[float, float], dict[str, object]] = {}
    index = 0
    while faces.More():
        if index in (0, 144, 145):
            face = TopoDS.Face(faces.Current())
            surface = BRepAdaptor_Surface(face)
            if surface.GetType() != GeomAbs_Plane:
                raise ValueError(f"face {index} is not planar")
            plane = surface.Plane()
            expected_y = 0.0 if index == 0 else 1.76784
            if abs(plane.Axis().Direction().Y()) < 0.999999 or abs(plane.Location().Y() - expected_y) > 1e-7:
                raise ValueError(f"face {index} has an unexpected plane")
            selected[index] = wire_curves(face)
        face = TopoDS.Face(faces.Current())
        surface = BRepAdaptor_Surface(face)
        if surface.GetType() == GeomAbs_Cone:
            circles = []
            edges = TopExp_Explorer(face, TopAbs_EDGE)
            while edges.More():
                curve = BRepAdaptor_Curve(TopoDS.Edge(edges.Current()))
                if curve.GetType() == GeomAbs_Circle:
                    circle = curve.Circle()
                    circles.append((circle.Location().Y(), circle.Radius(), xy(circle.Location())))
                edges.Next()
            circles.sort()
            if len(circles) != 2:
                raise ValueError("unexpected conical face boundaries")
            low, high = circles
            center = (round(high[2][0], 8), round(high[2][1], 8))
            item = {"center": high[2], "lowerRadius": low[1], "upperRadius": high[1], "depthMm": high[0] - low[0], "semiAngleRad": surface.Cone().SemiAngle()}
            if center in countersinks and countersinks[center] != item:
                raise ValueError("inconsistent split countersink")
            countersinks[center] = item
        faces.Next()
        index += 1
    if len(countersinks) != 4:
        raise ValueError(f"expected four countersinks, found {len(countersinks)}")
    if set(selected) != {0, 144, 145}:
        raise ValueError("missing expected source faces")
    return {
        "schemaVersion": 1,
        "sourceSha256": digest,
        "reader": "cadquery-ocp-novtk 8.0.1.1.0 / OCCT 8.0.1; extraction only",
        "sourceAxes": "recipe XY = source XZ; extrusion Z = source Y",
        "thicknessMm": 3.03784,
        "blindCutDepthMm": 1.27,
        "bottomFaceIndex": 0,
        "bottomWires": selected[0],
        "blindPocketFaceIndices": [144, 145],
        "blindPocketWires": [selected[144], selected[145]],
        "countersinks": [countersinks[key] for key in sorted(countersinks)],
    }


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: extract_ftc09.py PINNED_STEP OUTPUT_JSON")
    Path(sys.argv[2]).write_text(json.dumps(extract(Path(sys.argv[1])), indent=2, allow_nan=False) + "\n")
