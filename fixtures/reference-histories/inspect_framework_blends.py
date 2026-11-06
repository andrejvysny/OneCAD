#!/usr/bin/env python3
"""Map pinned Framework B-spline faces to their exact topological neighbors."""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

from OCP.BRepAdaptor import BRepAdaptor_Surface
from OCP.GeomAbs import GeomAbs_BSplineSurface, GeomAbs_Cylinder
from OCP.STEPControl import STEPControl_Reader
from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS

SOURCE_SHA = "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09"


def inspect(source: Path) -> dict[str, object]:
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA:
        raise ValueError("source checksum mismatch")
    reader = STEPControl_Reader()
    if str(reader.ReadFile(str(source))) != "IFSelect_ReturnStatus.IFSelect_RetDone":
        raise ValueError("source STEP refused")
    reader.TransferRoots()
    solids = TopExp_Explorer(reader.OneShape(), TopAbs_SOLID)
    solids.Next()  # Named STEPCAF inventory identifies the second solid.
    faces = TopExp_Explorer(solids.Current(), TopAbs_FACE)
    entries = []
    while faces.More():
        face = TopoDS.Face(faces.Current())
        edges = TopExp_Explorer(face, TopAbs_EDGE)
        seen = []
        while edges.More():
            seen.append(edges.Current())
            edges.Next()
        entries.append((face, seen))
        faces.Next()
    result = []
    for index, (face, edges) in enumerate(entries):
        surface = BRepAdaptor_Surface(face)
        if surface.GetType() != GeomAbs_BSplineSurface:
            continue
        neighbors = []
        for other_index, (other, other_edges) in enumerate(entries):
            if other_index == index or not any(a.IsSame(b) for a in edges for b in other_edges):
                continue
            adjacent = BRepAdaptor_Surface(other)
            kind = str(adjacent.GetType()).split(".")[-1]
            row = {"faceIndex": other_index, "surfaceType": kind}
            if adjacent.GetType() == GeomAbs_Cylinder:
                row["radiusMm"] = adjacent.Cylinder().Radius()
            neighbors.append(row)
        spline = surface.BSpline()
        result.append({"faceIndex": index, "degreeUV": [spline.UDegree(), spline.VDegree()], "poleCountUV": [spline.NbUPoles(), spline.NbVPoles()], "rationalUV": [spline.IsURational(), spline.IsVRational()], "neighbors": neighbors})
    if len(result) != 18:
        raise ValueError(f"expected 18 B-spline faces, found {len(result)}")
    return {"sourceSha256": SOURCE_SHA, "reader": "cadquery-ocp-novtk 8.0.1.1.0 / OCCT 8.0.1", "qualification": "Source adjacency evidence, not a recovered feature history or independent oracle.", "faces": result}


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: inspect_framework_blends.py PINNED_STEP OUTPUT_JSON")
    Path(sys.argv[2]).write_text(json.dumps(inspect(Path(sys.argv[1])), indent=2, allow_nan=False) + "\n")
