#!/usr/bin/env python3
"""Inventory pinned Framework STEP assembly labels; not an independent oracle."""
from __future__ import annotations

import hashlib
import json
import sys
from collections import Counter
from pathlib import Path

from OCP.BRepAdaptor import BRepAdaptor_Surface
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepGProp import BRepGProp
from OCP.Bnd import Bnd_Box
from OCP.GProp import GProp_GProps
from OCP.STEPCAFControl import STEPCAFControl_Reader
from OCP.TCollection import TCollection_ExtendedString
from OCP.TDF import TDF_Label
from OCP.TDataStd import TDataStd_Name
from OCP.TDocStd import TDocStd_Document
from OCP.TopAbs import TopAbs_FACE, TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS
from OCP.XCAFDoc import XCAFDoc_DocumentTool, XCAFDoc_ShapeTool
from OCP.collections import Sequence_TDF_Label

SOURCE_SHA = "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09"


def name(label: TDF_Label) -> str:
    attribute = TDataStd_Name()
    return attribute.Get().ToExtString() if label.FindAttribute(TDataStd_Name.GetID_s(), attribute) else ""


def inventory(label: TDF_Label, path: list[str]) -> list[dict[str, object]]:
    referred = TDF_Label()
    if not XCAFDoc_ShapeTool.GetReferredShape_s(label, referred):
        referred = label
    path = [*path, name(label)]
    children = Sequence_TDF_Label()
    XCAFDoc_ShapeTool.GetComponents_s(referred, children)
    if children.Length():
        return [item for i in range(1, children.Length() + 1) for item in inventory(children.Value(i), path)]
    shape = XCAFDoc_ShapeTool.GetShape_s(label)
    solids = TopExp_Explorer(shape, TopAbs_SOLID)
    result = []
    while solids.More():
        solid = solids.Current()
        properties = GProp_GProps()
        BRepGProp.VolumeProperties_s(solid, properties)
        counts: Counter[str] = Counter()
        faces = TopExp_Explorer(solid, TopAbs_FACE)
        while faces.More():
            kind = str(BRepAdaptor_Surface(TopoDS.Face(faces.Current())).GetType()).split(".")[-1]
            counts[kind] += 1
            faces.Next()
        surface = GProp_GProps()
        BRepGProp.SurfaceProperties_s(solid, surface)
        bounds = Bnd_Box()
        BRepBndLib.AddOptimal_s(solid, bounds, False, False)
        low, high = bounds.CornerMin(), bounds.CornerMax()
        geometry = {"volumeMm3": properties.Mass(), "areaMm2": surface.Mass(), "localBoundsMm": [[low.X(), low.Y(), low.Z()], [high.X(), high.Y(), high.Z()]], "faceTypes": dict(sorted(counts.items()))}
        fingerprint = hashlib.sha256(json.dumps(geometry, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()
        result.append({"occurrencePath": path, "productName": name(referred), "diagnosticGeometry": geometry, "diagnosticGeometrySha256": fingerprint})
        solids.Next()
    return result


def inspect(source: Path) -> dict[str, object]:
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    if digest != SOURCE_SHA:
        raise ValueError(f"source checksum mismatch: {digest}")
    reader = STEPCAFControl_Reader()
    reader.SetNameMode(True)
    if str(reader.ReadFile(str(source))) != "IFSelect_ReturnStatus.IFSelect_RetDone":
        raise ValueError("STEP reader rejected source")
    document = TDocStd_Document(TCollection_ExtendedString("MDTV-XCAF"))
    if not reader.Transfer(document):
        raise ValueError("STEP assembly transfer failed")
    tool = XCAFDoc_DocumentTool.ShapeTool_s(document.Main())
    roots = Sequence_TDF_Label()
    tool.GetFreeShapes(roots)
    solids = [item for i in range(1, roots.Length() + 1) for item in inventory(roots.Value(i), [])]
    if len(solids) != 6:
        raise ValueError(f"expected six source solids, found {len(solids)}")
    return {"schemaVersion": 1, "sourceSha256": digest, "reader": "cadquery-ocp-novtk 8.0.1.1.0 / OCCT 8.0.1", "qualification": "Same-kernel diagnostic inventory only; not a mass oracle or native feature history.", "solids": solids}


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: inspect_framework.py PINNED_STEP OUTPUT_JSON")
    Path(sys.argv[2]).write_text(json.dumps(inspect(Path(sys.argv[1])), indent=2, allow_nan=False) + "\n")
