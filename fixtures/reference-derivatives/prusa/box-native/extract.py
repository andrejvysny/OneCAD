#!/usr/bin/env python3
"""Extract bounded analytic stock evidence from the pinned Prusa box STEP."""
from __future__ import annotations

from collections import Counter
import hashlib
import json
import math
from pathlib import Path
from typing import Any

from OCP.Bnd import Bnd_Box
from OCP.BRepAdaptor import BRepAdaptor_Curve, BRepAdaptor_Surface
from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepBuilderAPI import BRepBuilderAPI_MakeFace
from OCP.BRepGProp import BRepGProp
from OCP.BRepPrimAPI import BRepPrimAPI_MakeBox, BRepPrimAPI_MakePrism
from OCP.GeomAbs import (GeomAbs_BSplineSurface, GeomAbs_Circle, GeomAbs_Cone,
                         GeomAbs_Cylinder, GeomAbs_Line, GeomAbs_Plane,
                         GeomAbs_Sphere, GeomAbs_Torus)
from OCP.GProp import GProp_GProps
from OCP.STEPControl import STEPControl_Reader
from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_WIRE
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS
from OCP.gp import gp_Pnt, gp_Vec

ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / "fixtures/public-cad/prusa/MINI-fsenzor-box.stp"
SOURCE_SHA = "c8b06ef5271166c367d27be13e9c8f93a7ae07707033e4d23bd1ed5997285cb6"


def xyz(point: Any) -> list[float]:
    return [point.X(), point.Y(), point.Z()]


def measure(shape: Any, surface: bool = False) -> float:
    result = GProp_GProps()
    if surface:
        BRepGProp.SurfaceProperties_s(shape, result)
    else:
        BRepGProp.VolumeProperties_s(shape, result)
    return result.Mass()


def outline(wire: Any) -> list[dict[str, Any]]:
    iterator = TopExp_Explorer(wire, TopAbs_EDGE)
    curves: list[dict[str, Any]] = []
    while iterator.More():
        curve = BRepAdaptor_Curve(TopoDS.Edge(iterator.Current()))
        first, last = curve.FirstParameter(), curve.LastParameter()
        a, b, middle = (curve.Value(t) for t in (first, last, (first + last) / 2))
        if curve.GetType() == GeomAbs_Line:
            curves.append({"kind": "line", "start": [a.X(), a.Y()], "end": [b.X(), b.Y()]})
        elif curve.GetType() == GeomAbs_Circle:
            circle = curve.Circle()
            center = circle.Location()
            def angle(point: Any) -> float:
                return math.atan2(point.Y() - center.Y(), point.X() - center.X())
            start, end, mid = angle(a), angle(b), angle(middle)
            if (mid - start) % math.tau > (end - start) % math.tau:
                start, end = end, start
            curves.append({"kind": "arc", "center": [center.X(), center.Y()],
                           "radius": circle.Radius(), "startAngle": start,
                           "endAngle": start + (end - start) % math.tau})
        else:
            raise ValueError(f"unsupported bottom outline curve: {curve.GetType()}")
        iterator.Next()
    return curves


def inventory(face: Any, index: int) -> dict[str, Any]:
    surface = BRepAdaptor_Surface(face)
    bounds = Bnd_Box()
    BRepBndLib.Add_s(face, bounds)
    record: dict[str, Any] = {"index": index, "type": str(surface.GetType()).split(".")[-1],
                              "areaMm2": measure(face, True),
                              "boundsMm": [bounds.GetXMin(), bounds.GetYMin(), bounds.GetZMin(),
                                           bounds.GetXMax(), bounds.GetYMax(), bounds.GetZMax()]}
    if surface.GetType() == GeomAbs_Plane:
        plane = surface.Plane()
        record.update(origin=xyz(plane.Location()), axis=xyz(plane.Axis().Direction()))
    elif surface.GetType() == GeomAbs_Cylinder:
        cylinder = surface.Cylinder()
        record.update(origin=xyz(cylinder.Location()), axis=xyz(cylinder.Axis().Direction()), radius=cylinder.Radius(),
                      vRange=[surface.FirstVParameter(), surface.LastVParameter()])
    elif surface.GetType() == GeomAbs_Cone:
        cone = surface.Cone()
        record.update(origin=xyz(cone.Location()), axis=xyz(cone.Axis().Direction()),
                      referenceRadius=cone.RefRadius(), semiAngleRadians=cone.SemiAngle(),
                      vRange=[surface.FirstVParameter(), surface.LastVParameter()])
    elif surface.GetType() == GeomAbs_Torus:
        torus = surface.Torus()
        record.update(origin=xyz(torus.Location()), axis=xyz(torus.Axis().Direction()),
                      majorRadius=torus.MajorRadius(), minorRadius=torus.MinorRadius())
    elif surface.GetType() == GeomAbs_Sphere:
        sphere = surface.Sphere()
        record.update(origin=xyz(sphere.Location()), radius=sphere.Radius())
    elif surface.GetType() == GeomAbs_BSplineSurface:
        spline = surface.BSpline()
        record.update(uDegree=spline.UDegree(), vDegree=spline.VDegree(),
                      uPoles=spline.NbUPoles(), vPoles=spline.NbVPoles(),
                      uRational=spline.IsURational(), vRational=spline.IsVRational(),
                      uRange=[surface.FirstUParameter(), surface.LastUParameter()],
                      vRange=[surface.FirstVParameter(), surface.LastVParameter()])
    return record


def stock_difference(source: Any, stock: Any) -> float:
    result = BRepAlgoAPI_Cut(source, stock)
    result.Build()
    if not result.IsDone():
        raise RuntimeError("source minus stock Boolean failed")
    return measure(result.Shape())


def main() -> None:
    digest = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    if digest != SOURCE_SHA:
        raise ValueError("source hash mismatch")
    reader = STEPControl_Reader()
    reader.ReadFile(str(SOURCE))
    reader.TransferRoots()
    shape = reader.OneShape()
    explorer = TopExp_Explorer(shape, TopAbs_FACE)
    faces: list[dict[str, Any]] = []
    bottom = None
    while explorer.More():
        face = TopoDS.Face(explorer.Current())
        if len(faces) == 34:
            bottom = face
        faces.append(inventory(face, len(faces)))
        explorer.Next()
    if len(faces) != 166 or bottom is None:
        raise ValueError("unexpected source face inventory")
    wire = TopExp_Explorer(bottom, TopAbs_WIRE)
    outer = TopoDS.Wire(wire.Current())
    planar = BRepBuilderAPI_MakeFace(outer).Face()
    stock = BRepPrimAPI_MakePrism(planar, gp_Vec(0, 0, 13)).Shape()
    plane = BRepAdaptor_Surface(bottom).Plane()
    recipe = {"sourceSha256": digest, "sourceFaceIndexZeroBased": 34,
              "description": "Bottom outer contour only; not a complete box.",
              "planeOrigin": xyz(plane.Location()), "extrusionMm": 13.0,
              "outerAreaMm2SameKernel": measure(planar, True),
              "sourceMinusStockVolumeMm3SameKernel": stock_difference(shape, stock),
              "curves": outline(outer)}
    result = {"sourceSha256": digest, "reader": "cadquery-ocp-novtk 8.0.1.1",
              "description": "Same-kernel source inventory; not an independent oracle.",
              "faceCounts": dict(Counter(face["type"] for face in faces)), "faces": faces}
    destination = Path(__file__).resolve().parent
    (destination / "bottom-stock-recipe.json").write_text(json.dumps(recipe, indent=2, allow_nan=False) + "\n")
    (destination / "surface-inventory.json").write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
    enclosing = BRepPrimAPI_MakeBox(gp_Pnt(-40, -26, -10), 61, 28, 15).Shape()
    outer = {"sourceSha256": digest, "description": "Deliberately oversized enclosing stock; not source geometry.",
             "corner": [-40.0, -26.0, -10.0], "dimensions": [61.0, 28.0, 15.0],
             "sourceMinusStockVolumeMm3SameKernel": stock_difference(shape, enclosing)}
    (destination / "enclosing-stock-recipe.json").write_text(json.dumps(outer, indent=2, allow_nan=False) + "\n")
    print(f"bottom outer area {recipe['outerAreaMm2SameKernel']}; source minus stock {recipe['sourceMinusStockVolumeMm3SameKernel']}")


if __name__ == "__main__":
    main()
