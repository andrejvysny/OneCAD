#!/usr/bin/env python3
"""Extract analytic construction evidence from the pinned Prusa cover STEP."""
from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
from typing import Any

from OCP.BRepAdaptor import BRepAdaptor_Curve, BRepAdaptor_Surface
from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut
from OCP.BRepBuilderAPI import BRepBuilderAPI_MakeEdge, BRepBuilderAPI_MakeFace, BRepBuilderAPI_MakeWire
from OCP.BRepGProp import BRepGProp
from OCP.BRepPrimAPI import BRepPrimAPI_MakePrism
from OCP.GeomAbs import GeomAbs_Circle, GeomAbs_Cone, GeomAbs_Cylinder, GeomAbs_Line, GeomAbs_Plane
from OCP.GProp import GProp_GProps
from OCP.STEPControl import STEPControl_Reader
from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_WIRE
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS
from OCP.gp import gp_Pnt, gp_Vec

ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / "fixtures/public-cad/prusa/MINI-fsenzor-cover.stp"
EXPECTED = "e6391172f932b141483f7b8b1a0b560c4260ba1116e588873916ba0ecacb2bd5"


def xyz(p: Any) -> list[float]:
    return [p.X(), p.Y(), p.Z()]


def edges(wire: Any) -> list[dict[str, Any]]:
    explorer = TopExp_Explorer(wire, TopAbs_EDGE)
    result: list[dict[str, Any]] = []
    while explorer.More():
        curve = BRepAdaptor_Curve(TopoDS.Edge(explorer.Current()))
        first, last = curve.FirstParameter(), curve.LastParameter()
        item: dict[str, Any] = {"type": str(curve.GetType()).split(".")[-1],
                                "start": xyz(curve.Value(first)), "end": xyz(curve.Value(last)),
                                "mid": xyz(curve.Value((first + last) / 2))}
        if curve.GetType() == GeomAbs_Circle:
            circle = curve.Circle()
            item.update(center=xyz(circle.Location()), radius=circle.Radius(), axis=xyz(circle.Axis().Direction()))
        elif curve.GetType() != GeomAbs_Line:
            item["unsupported"] = True
        result.append(item)
        explorer.Next()
    return result


def face_data(face: Any, index: int) -> dict[str, Any]:
    surface = BRepAdaptor_Surface(face)
    kind = surface.GetType()
    item: dict[str, Any] = {"index": index, "surface": str(kind).split(".")[-1],
                            "uRange": [surface.FirstUParameter(), surface.LastUParameter()],
                            "vRange": [surface.FirstVParameter(), surface.LastVParameter()]}
    if kind == GeomAbs_Plane:
        plane = surface.Plane()
        item.update(origin=xyz(plane.Location()), axis=xyz(plane.Axis().Direction()))
    elif kind in (GeomAbs_Cylinder, GeomAbs_Cone):
        shape = surface.Cylinder() if kind == GeomAbs_Cylinder else surface.Cone()
        item.update(origin=xyz(shape.Location()), axis=xyz(shape.Axis().Direction()))
        if kind == GeomAbs_Cylinder:
            item["radius"] = shape.Radius()
        else:
            item.update(referenceRadius=shape.RefRadius(), semiAngleRadians=shape.SemiAngle())
    else:
        item["unsupported"] = True
    wires = TopExp_Explorer(face, TopAbs_WIRE)
    item["wires"] = []
    while wires.More():
        item["wires"].append(edges(wires.Current()))
        wires.Next()
    return item


def volume(shape: Any) -> float:
    properties = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, properties)
    return properties.Mass()


def top_stock_difference(source: Any, top_face: Any) -> float:
    wires = TopExp_Explorer(top_face, TopAbs_WIRE)
    face = BRepBuilderAPI_MakeFace(TopoDS.Wire(wires.Current())).Face()
    stock = BRepPrimAPI_MakePrism(face, gp_Vec(0, -4, 0)).Shape()
    difference = BRepAlgoAPI_Cut(source, stock)
    difference.Build()
    if not difference.IsDone():
        raise RuntimeError("source-minus-top-stock failed")
    return volume(difference.Shape())


def lower_stock(source: Any, lower_face: Any, face_record: dict[str, Any]) -> dict[str, Any]:
    wire = TopExp_Explorer(lower_face, TopAbs_WIRE)
    explorer = TopExp_Explorer(wire.Current(), TopAbs_EDGE)
    source_edges: list[Any] = []
    while explorer.More():
        source_edges.append(TopoDS.Edge(explorer.Current()))
        explorer.Next()
    if len(source_edges) != 16:
        raise ValueError("unexpected lower outline")
    profile = face_record["wires"][0]
    start, end = profile[4]["end"], profile[14]["start"]
    builder = BRepBuilderAPI_MakeWire()
    for edge in source_edges[4:15]:
        builder.Add(edge)
    builder.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(*start), gp_Pnt(*end)).Edge())
    if not builder.IsDone():
        raise RuntimeError("lower outline bridge did not form a wire")
    face = BRepBuilderAPI_MakeFace(builder.Wire()).Face()
    props = GProp_GProps()
    BRepGProp.SurfaceProperties_s(face, props)
    stock = BRepPrimAPI_MakePrism(face, gp_Vec(0, 4, 0)).Shape()
    difference = BRepAlgoAPI_Cut(source, stock)
    difference.Build()
    if not difference.IsDone():
        raise RuntimeError("source-minus-lower-stock failed")
    segments = profile[4:15] + [{"type": "GeomAbs_Line", "start": start, "end": end}]
    curves: list[dict[str, Any]] = []
    for segment in segments:
        if segment["type"] == "GeomAbs_Line":
            curves.append({"kind": "line", "start": [segment["start"][0], segment["start"][2]],
                           "end": [segment["end"][0], segment["end"][2]]})
        elif segment["type"] == "GeomAbs_Circle":
            center = [segment["center"][0], segment["center"][2]]
            def angle(point: list[float]) -> float:
                return math.atan2(point[2] - center[1], point[0] - center[0])
            first, last, middle = angle(segment["start"]), angle(segment["end"]), angle(segment["mid"])
            if (middle - first) % math.tau > (last - first) % math.tau:
                first, last = last, first
            curves.append({"kind": "arc", "center": center, "radius": segment["radius"],
                           "startAngle": first, "endAngle": first + (last - first) % math.tau})
        else:
            raise ValueError("unsupported lower contour curve")
    return {"sourceSha256": EXPECTED, "description": "Lower contour with tab-footprint bridge; incomplete stock.",
            "sourceFaceIndexZeroBased": 46, "bridge": {"start": start, "end": end},
            "planeOrigin": [0, start[1] + 4, 0], "xAxis": [1, 0, 0], "yAxis": [0, 0, 1],
            "normal": [0, -1, 0], "extrusionMm": 4.0, "outerAreaMm2SameKernel": props.Mass(),
            "sourceMinusStockVolumeMm3SameKernel": volume(difference.Shape()), "curves": curves}


def recess_recipe(face: Any, record: dict[str, Any]) -> dict[str, Any]:
    wire = TopExp_Explorer(face, TopAbs_WIRE)
    outer = BRepBuilderAPI_MakeFace(TopoDS.Wire(wire.Current())).Face()
    props = GProp_GProps()
    BRepGProp.SurfaceProperties_s(outer, props)
    curves: list[dict[str, Any]] = []
    for segment in record["wires"][0]:
        if segment["type"] == "GeomAbs_Line":
            curves.append({"kind": "line", "start": [segment["start"][0], segment["start"][2]],
                           "end": [segment["end"][0], segment["end"][2]]})
        elif segment["type"] == "GeomAbs_Circle":
            center = [segment["center"][0], segment["center"][2]]
            def angle(point: list[float]) -> float:
                return math.atan2(point[2] - center[1], point[0] - center[0])
            first, last, middle = angle(segment["start"]), angle(segment["end"]), angle(segment["mid"])
            if (middle - first) % math.tau > (last - first) % math.tau:
                first, last = last, first
            curves.append({"kind": "arc", "center": center, "radius": segment["radius"],
                           "startAngle": first, "endAngle": first + (last - first) % math.tau})
        else:
            raise ValueError("unsupported recess outer curve")
    return {"sourceSha256": EXPECTED, "sourceFaceIndexZeroBased": 10,
            "description": "Recess outer loop only; subsequent hinge cut creates the inner wire.",
            "planeOrigin": [0, record["origin"][1], 0], "extrusionMm": 1.0,
            "outerAreaMm2SameKernel": props.Mass(), "curves": curves}


def main() -> None:
    source_hash = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    if source_hash != EXPECTED:
        raise ValueError("source hash does not match pinned Prusa cover")
    reader = STEPControl_Reader()
    reader.ReadFile(str(SOURCE))
    reader.TransferRoots()
    shape = reader.OneShape()
    explorer = TopExp_Explorer(shape, TopAbs_FACE)
    faces: list[dict[str, Any]] = []
    missing = math.nan
    lower = None
    recess = None
    while explorer.More():
        face = TopoDS.Face(explorer.Current())
        if len(faces) == 1:
            missing = top_stock_difference(shape, face)
        if len(faces) == 10:
            recess = face
        if len(faces) == 46:
            lower = face
        faces.append(face_data(face, len(faces)))
        explorer.Next()
    if len(faces) != 61 or not math.isfinite(missing):
        raise ValueError("unexpected STEP face inventory")
    report = {"sourceSha256": source_hash, "reader": "cadquery-ocp-novtk 8.0.1.1",
              "description": "Same-kernel construction evidence, not an independent oracle.",
              "sourceMinusTopStockVolumeMm3": missing, "faces": faces}
    output = Path(__file__).with_name("geometry-extraction.json")
    output.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")
    if lower is None:
        raise ValueError("lower face missing")
    if recess is None:
        raise ValueError("recess face missing")
    recess_data = recess_recipe(recess, faces[10])
    Path(__file__).with_name("recess-recipe.json").write_text(json.dumps(recess_data, indent=2, allow_nan=False) + "\n")
    recipe = lower_stock(shape, lower, faces[46])
    Path(__file__).with_name("lower-stock-recipe.json").write_text(json.dumps(recipe, indent=2, allow_nan=False) + "\n")
    print(f"wrote {output}; source minus top stock {missing:.12f} mm³")


if __name__ == "__main__":
    main()
