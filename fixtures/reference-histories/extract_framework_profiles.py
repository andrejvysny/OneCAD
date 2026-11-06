#!/usr/bin/env python3
"""Extract two editable planar profile inputs from the pinned Framework STEP."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
from pathlib import Path

from OCP.BRepAdaptor import BRepAdaptor_Curve, BRepAdaptor_Surface
from OCP.BRepTools import BRepTools_WireExplorer
from OCP.BRepGProp import BRepGProp
from OCP.GProp import GProp_GProps
from OCP.GeomAbs import GeomAbs_Line, GeomAbs_Plane
from OCP.STEPControl import STEPControl_Reader
from OCP.TopAbs import TopAbs_FACE, TopAbs_SOLID, TopAbs_WIRE
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS

SOURCE_SHA = "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09"


def side_wire(face: object) -> list[list[dict[str, object]]]:
    explorer = TopExp_Explorer(face, TopAbs_WIRE)
    wires = []
    while explorer.More():
        edges = BRepTools_WireExplorer(TopoDS.Wire(explorer.Current()), face)
        curves = []
        while edges.More():
            curve = BRepAdaptor_Curve(TopoDS.Edge(edges.Current()))
            if curve.GetType() != GeomAbs_Line:
                raise ValueError("nonlinear clip floor boundary")
            a = curve.Value(curve.FirstParameter())
            b = curve.Value(curve.LastParameter())
            curves.append({"kind": "line", "start": [-a.Z(), a.Y()], "end": [-b.Z(), b.Y()]})
            edges.Next()
        wires.append(curves)
        explorer.Next()
    return wires


def extract(source: Path) -> dict[str, object]:
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    if digest != SOURCE_SHA:
        raise ValueError(f"source checksum mismatch: {digest}")
    helper_path = Path(__file__).with_name("extract_ftc09.py")
    spec = importlib.util.spec_from_file_location("reference_curves", helper_path)
    if spec is None or spec.loader is None:
        raise ValueError("curve extractor unavailable")
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    reader = STEPControl_Reader()
    if str(reader.ReadFile(str(source))) != "IFSelect_ReturnStatus.IFSelect_RetDone":
        raise ValueError("STEP reader rejected source")
    reader.TransferRoots()
    solids = TopExp_Explorer(reader.OneShape(), TopAbs_SOLID)
    solids.Next()  # STEPCAF inventory identifies the second solid as the enclosure.
    faces = TopExp_Explorer(solids.Current(), TopAbs_FACE)
    selected = {}
    sides = {}
    cut_planes = {}
    index = 0
    while faces.More():
        if index in (32, 34, 38, 40, 41, 45, 65, 72, 81, 83, 84, 86, 90, 93, 102, 111):
            surface = BRepAdaptor_Surface(TopoDS.Face(faces.Current()))
            if surface.GetType() != GeomAbs_Plane:
                raise ValueError(f"clip boundary {index} is not planar")
            axis = surface.Plane().Axis()
            origin, normal = axis.Location(), axis.Direction()
            cut_planes[index] = {"faceIndex": index, "origin": [origin.X(), origin.Y(), origin.Z()], "normal": [normal.X(), normal.Y(), normal.Z()]}
        if index in (63, 114):
            face = TopoDS.Face(faces.Current())
            surface = BRepAdaptor_Surface(face)
            expected_x = -13.2 if index == 63 else 13.2
            if surface.GetType() != GeomAbs_Plane or abs(surface.Plane().Location().X() - expected_x) > 1e-7:
                raise ValueError(f"face {index} has unexpected side plane")
            sides[index] = {"faceIndex": index, "sourceXmm": expected_x, "wires": side_wire(face)}
        if index in (3, 6, 24, 143, 157, 177):
            face = TopoDS.Face(faces.Current())
            surface = BRepAdaptor_Surface(face)
            if surface.GetType() != GeomAbs_Plane:
                raise ValueError(f"face {index} is not planar")
            plane = surface.Plane()
            expected_y = {3: 1.5, 6: 3.1, 24: 0.65, 143: 1.2, 157: 3.1, 177: 0.0}[index]
            if abs(plane.Location().Y() - expected_y) > 1e-7 or abs(plane.Axis().Direction().Y()) < 0.999999:
                raise ValueError(f"face {index} has unexpected plane")
            properties = GProp_GProps()
            BRepGProp.SurfaceProperties_s(face, properties)
            selected[index] = {"faceIndex": index, "sourceYmm": expected_y, "diagnosticAreaMm2": properties.Mass(), "wires": helper.wire_curves(face)}
        faces.Next()
        index += 1
    if set(selected) != {3, 6, 24, 143, 157, 177} or [len(selected[i]["wires"]) for i in (3, 6, 24, 143, 157, 177)] != [1, 2, 1, 1, 2, 1]:
        raise ValueError("source profile selection changed")
    if set(sides) != {63, 114} or any(len(side["wires"]) != 1 for side in sides.values()):
        raise ValueError("source clip floor selection changed")
    if len(cut_planes) != 16:
        raise ValueError("source clip boundary planes missing")
    return {"schemaVersion": 1, "sourceSha256": digest, "productName": "FW_EXP_1USBC_FRAME_CLIP_BC_229_", "sourceAxes": "main recipe XY = source XZ; side recipe XY = -source Z, source Y", "reader": "cadquery-ocp-novtk 8.0.1.1.0 / OCCT 8.0.1; extraction only", "bottomFace": selected[177], "interiorFloorFace": selected[3], "rearUndersideFace": selected[24], "rearPocketFloorFace": selected[143], "bossTopFaces": [selected[6], selected[157]], "clipFloorFaces": [sides[63], sides[114]], "clipCutPlanes": [cut_planes[i] for i in sorted(cut_planes)], "stockLimitsMm": {"x": [-15.0, 15.0], "y": [0.0, 6.8], "z": [-32.0, 0.0]}, "qualification": "Intermediate inputs only; stock and pocket do not reconstruct the finished enclosure."}


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: extract_framework_profiles.py PINNED_STEP OUTPUT_JSON")
    Path(sys.argv[2]).write_text(json.dumps(extract(Path(sys.argv[1])), indent=2, allow_nan=False) + "\n")
