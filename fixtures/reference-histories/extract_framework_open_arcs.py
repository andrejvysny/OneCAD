#!/usr/bin/env python3
"""Derive numeric Framework open-sweep inputs from the pinned named STEP product."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any

from OCP.BRepAdaptor import BRepAdaptor_Surface
from OCP.TopAbs import TopAbs_FACE
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS

from inspect_framework import SOURCE_SHA
from inspect_framework_residual import PRODUCT, source_enclosure

Vector = list[float]
RADIUS_MM = 0.3


def dot(a: Vector, b: Vector) -> float:
    return sum(x * y for x, y in zip(a, b))


def subtract(a: Vector, b: Vector) -> Vector:
    return [x - y for x, y in zip(a, b)]


def cross(a: Vector, b: Vector) -> Vector:
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def unit(v: Vector) -> Vector:
    length = math.sqrt(dot(v, v))
    if length == 0:
        raise ValueError("degenerate vector")
    return [x / length for x in v]


def xyz(value: Any) -> Vector:
    return [value.X(), value.Y(), value.Z()]


def faces_from_source(path: Path) -> list[Any]:
    iterator = TopExp_Explorer(source_enclosure(path), TopAbs_FACE)
    faces = []
    while iterator.More():
        faces.append(TopoDS.Face(iterator.Current()))
        iterator.Next()
    return faces


def ellipse_frame(faces: list[Any], cylinder_index: int) -> dict[str, Any]:
    cylinder = BRepAdaptor_Surface(faces[cylinder_index]).Cylinder()
    if not math.isclose(cylinder.Radius(), RADIUS_MM, abs_tol=1e-9):
        raise ValueError("source cylinder radius changed")
    axis = cylinder.Axis()
    origin, direction = xyz(axis.Location()), xyz(axis.Direction())
    plane = BRepAdaptor_Surface(faces[111]).Plane().Axis()
    position, normal = xyz(plane.Location()), xyz(plane.Direction())
    denominator = dot(normal, direction)
    if abs(denominator) < 1e-12:
        raise ValueError("offset cylinder and plane do not form a bounded ellipse")
    along = (-RADIUS_MM + dot(normal, subtract(position, origin))) / denominator
    center = [origin[i] + along * direction[i] for i in range(3)]
    minor = unit(cross(direction, normal))
    major = unit(cross(minor, normal))
    return {"centerMm": center, "majorUnit": major, "minorUnit": cross(normal, major),
            "normalUnit": normal, "semiaxesMm": [2 * RADIUS_MM / abs(denominator), 2 * RADIUS_MM]}


def circle_center(a: Vector, b: Vector, c: Vector) -> Vector:
    u, v = subtract(b, a), subtract(c, a)
    normal = cross(u, v)
    denominator = 2 * dot(normal, normal)
    if denominator == 0:
        raise ValueError("degenerate source section")
    left, right = cross(v, normal), cross(normal, u)
    return [a[i] + (dot(u, u) * left[i] + dot(v, v) * right[i]) / denominator for i in range(3)]


def endpoint_angles(face: Any, frame: dict[str, Any]) -> list[float]:
    surface = BRepAdaptor_Surface(face)
    angles = []
    for v in (0.0, 1.0):
        points = [xyz(surface.Value(u, v)) for u in (0.22, 0.5, 0.78)]
        delta = subtract(circle_center(*points), frame["centerMm"])
        a, b = frame["semiaxesMm"]
        angles.append(math.atan2(dot(delta, unit(frame["minorUnit"])) / b, dot(delta, frame["majorUnit"]) / a))
    delta = (angles[1] - angles[0] + math.pi) % (2 * math.pi) - math.pi
    lo, hi = sorted((angles[0], angles[0] + delta))
    if not 0 < hi - lo < math.pi:
        raise ValueError("source section does not define a short arc")
    return [lo, hi]


def profile_frame(frame: dict[str, Any], angle: float) -> dict[str, Any]:
    center, major, minor = frame["centerMm"], frame["majorUnit"], frame["minorUnit"]
    a, b = frame["semiaxesMm"]
    point = [center[i] + a * math.cos(angle) * major[i] + b * math.sin(angle) * minor[i] for i in range(3)]
    tangent = unit([-a * math.sin(angle) * major[i] + b * math.cos(angle) * minor[i] for i in range(3)])
    x_axis = frame["normalUnit"]
    return {"centerMm": point, "normalUnit": tangent, "xAxisUnit": x_axis,
            "yAxisUnit": cross(tangent, x_axis), "radiusMm": RADIUS_MM}


def arc_length(a: float, b: float, lo: float, hi: float, intervals: int) -> float:
    step = (hi - lo) / intervals
    def speed(t: float) -> float:
        return math.hypot(a * math.sin(t), b * math.cos(t))
    odd = sum(speed(lo + i * step) for i in range(1, intervals, 2))
    even = sum(speed(lo + i * step) for i in range(2, intervals, 2))
    return step / 3 * (speed(lo) + speed(hi) + 4 * odd + 2 * even)


def derive_pipe(faces: list[Any], patch_index: int, cylinder_index: int) -> dict[str, Any]:
    frame = ellipse_frame(faces, cylinder_index)
    lo, hi = endpoint_angles(faces[patch_index], frame)
    a, b = frame["semiaxesMm"]
    length = arc_length(a, b, lo, hi, 8192)
    return {"sourcePatchFace": patch_index, "sourceCylinderFace": cylinder_index,
            "sourcePlaneFace": 111, **frame, "pathRangeRadians": [lo, hi],
            "profile": profile_frame(frame, lo), "analyticArcLengthMm": length,
            "analyticVolumeMm3": math.pi * RADIUS_MM ** 2 * length,
            "volumeAssertionToleranceMm3": 1e-6}


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=root / "fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp")
    source = parser.parse_args().source
    faces = faces_from_source(source)
    result = {"sourceSha256": SOURCE_SHA, "sourceProduct": PRODUCT,
              "coordinateFrame": "right-handed Z-up, millimeters",
              "scope": "Source-derived numeric inputs; not a source-equivalence qualification.",
              "pipes": [derive_pipe(faces, 110, 109), derive_pipe(faces, 112, 103)]}
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
