#!/usr/bin/env python3
"""Inspect pinned Prusa source solids and candidate hinge surfaces with OCP."""
from __future__ import annotations

import hashlib
import json
import math
import re
import sys
from pathlib import Path

import OCP
from OCP.BRepAdaptor import BRepAdaptor_Surface
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepGProp import BRepGProp
from OCP.Bnd import Bnd_Box
from OCP.GProp import GProp_GProps
from OCP.GeomAbs import GeomAbs_Cylinder
from OCP.STEPControl import STEPControl_Reader
from OCP.TopAbs import TopAbs_FACE, TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS

SOURCES = {
    "box": ("c8b06ef5271166c367d27be13e9c8f93a7ae07707033e4d23bd1ed5997285cb6", 3042, 77, 1.4, (-16.0000009536743, -13.5000000298023, 4.00000005960464), (0.0, 0.0, -1.0)),
    "cover": ("e6391172f932b141483f7b8b1a0b560c4260ba1116e588873916ba0ecacb2bd5", None, None, None, None, None),
    "lever": ("beee16c2e5c09d26777d4971df8c343d467f926121d1753c6b41bc0b5ff6bbc6", 515, 16, 1.65, (-16.0315999507904, 4.00000005960464, 13.5072009086609), (0.0, -1.0, 0.0)),
}


def vector(value: object) -> list[float]:
    return [value.X(), value.Y(), value.Z()]


def inspect(root: Path, name: str) -> dict[str, object]:
    digest, face_id, surface_id, radius, origin, direction = SOURCES[name]
    path = root / f"MINI-fsenzor-{name}.stp"
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError(f"{name}: source checksum mismatch")
    if face_id is not None:
        pattern = rf"#{face_id}=ADVANCED_FACE\('',\([^;]*\),#{surface_id},\.[TF]\.\);"
        if not re.search(pattern, data.decode("ascii")):
            raise ValueError(f"{name}: source face/surface relation changed")
    reader = STEPControl_Reader()
    if str(reader.ReadFile(str(path))) != "IFSelect_ReturnStatus.IFSelect_RetDone":
        raise ValueError(f"{name}: STEP reader rejected source")
    if reader.TransferRoots() < 1:
        raise ValueError(f"{name}: no transferable STEP roots")
    shape = reader.OneShape()
    solids = TopExp_Explorer(shape, TopAbs_SOLID)
    count = 0
    while solids.More():
        count += 1
        solids.Next()
    if count != 1:
        raise ValueError(f"{name}: expected one solid, found {count}")
    volume, area = GProp_GProps(), GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, volume)
    BRepGProp.SurfaceProperties_s(shape, area)
    bounds = Bnd_Box()
    BRepBndLib.AddOptimal_s(shape, bounds, False, False)
    result: dict[str, object] = {"sha256": digest, "solidCount": count,
        "volumeMm3": volume.Mass(), "surfaceAreaMm2": area.Mass(), "boundsMm": [vector(bounds.CornerMin()), vector(bounds.CornerMax())]}
    if face_id is not None:
        matches = 0
        faces = TopExp_Explorer(shape, TopAbs_FACE)
        while faces.More():
            surface = BRepAdaptor_Surface(TopoDS.Face(faces.Current()), True)
            if surface.GetType() == GeomAbs_Cylinder:
                cylinder = surface.Cylinder()
                actual_origin = vector(cylinder.Axis().Location())
                actual_direction = vector(cylinder.Axis().Direction())
                if (abs(cylinder.Radius() - radius) < 1e-7
                    and math.dist(actual_origin, origin) < 1e-6
                    and math.dist(actual_direction, direction) < 1e-7):
                    matches += 1
                    face_bounds = Bnd_Box()
                    BRepBndLib.AddOptimal_s(faces.Current(), face_bounds, False, False)
                    candidate_bounds = [vector(face_bounds.CornerMin()), vector(face_bounds.CornerMax())]
                    properties = GProp_GProps()
                    BRepGProp.SurfaceProperties_s(faces.Current(), properties)
                    candidate_area = properties.Mass()
            faces.Next()
        if matches != 1:
            raise ValueError(f"{name}: expected one candidate cylinder, found {matches}")
        result["candidate"] = {"stepFace": face_id, "stepCylinder": surface_id,
            "radiusMm": radius, "originMm": origin, "axis": direction, "matchingImportedFaces": matches,
            "faceBoundsMm": candidate_bounds, "faceAreaMm2": candidate_area}
    return result


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: inspect_prusa_sensor.py PRUSA_DIR OUTPUT_JSON")
    report = {"reader": f"cadquery-ocp-novtk {OCP.__version__}", "units": "mm",
        "sourceRevision": "853bc30c4b10190f1d669ed6d0a567e333c28f21",
        "parts": {name: inspect(Path(sys.argv[1]), name) for name in SOURCES}}
    Path(sys.argv[2]).write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
