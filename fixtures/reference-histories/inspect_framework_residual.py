#!/usr/bin/env python3
"""Check named Framework enclosure and native STEP Booleans; diagnostic, same kernel."""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path
from typing import Any

from OCP.BRepAlgoAPI import BRepAlgoAPI_Common, BRepAlgoAPI_Cut
from OCP.BRepBuilderAPI import BRepBuilderAPI_Transform
from OCP.BRepCheck import BRepCheck_Analyzer
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepGProp import BRepGProp
from OCP.Bnd import Bnd_Box
from OCP.GProp import GProp_GProps
from OCP.STEPCAFControl import STEPCAFControl_Reader
from OCP.STEPControl import STEPControl_Reader
from OCP.TCollection import TCollection_ExtendedString
from OCP.TDF import TDF_Label
from OCP.TDocStd import TDocStd_Document
from OCP.TopAbs import TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.XCAFDoc import XCAFDoc_DocumentTool, XCAFDoc_ShapeTool
from OCP.collections import Sequence_TDF_Label
from OCP.gp import gp_Trsf

from inspect_framework import SOURCE_SHA, name

PRODUCT = "FW_EXP_1USBC_FRAME_CLIP_BC_229_"


def solids(shape: Any) -> list[Any]:
    found = []
    iterator = TopExp_Explorer(shape, TopAbs_SOLID)
    while iterator.More():
        found.append(iterator.Current())
        iterator.Next()
    return found


def matches(label: TDF_Label) -> list[Any]:
    referred = TDF_Label()
    if not XCAFDoc_ShapeTool.GetReferredShape_s(label, referred):
        referred = label
    children = Sequence_TDF_Label()
    XCAFDoc_ShapeTool.GetComponents_s(referred, children)
    if children.Length():
        return [shape for i in range(1, children.Length() + 1) for shape in matches(children.Value(i))]
    return solids(XCAFDoc_ShapeTool.GetShape_s(label)) if name(referred) == PRODUCT else []


def source_enclosure(path: Path) -> Any:
    if hashlib.sha256(path.read_bytes()).hexdigest() != SOURCE_SHA:
        raise ValueError("source checksum mismatch")
    reader = STEPCAFControl_Reader()
    reader.SetNameMode(True)
    if str(reader.ReadFile(str(path))) != "IFSelect_ReturnStatus.IFSelect_RetDone":
        raise ValueError("source STEP read failed")
    document = TDocStd_Document(TCollection_ExtendedString("MDTV-XCAF"))
    if not reader.Transfer(document):
        raise ValueError("source STEPCAF transfer failed")
    roots = Sequence_TDF_Label()
    XCAFDoc_DocumentTool.ShapeTool_s(document.Main()).GetFreeShapes(roots)
    found = [shape for i in range(1, roots.Length() + 1) for shape in matches(roots.Value(i))]
    if len(found) != 1:
        raise ValueError(f"expected one named enclosure solid, found {len(found)}")
    transform = gp_Trsf()
    transform.SetValues(1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0)
    return BRepBuilderAPI_Transform(found[0], transform, True).Shape()


def native_solid(path: Path) -> Any:
    reader = STEPControl_Reader()
    if str(reader.ReadFile(str(path))) != "IFSelect_ReturnStatus.IFSelect_RetDone":
        raise ValueError("native STEP read failed")
    reader.TransferRoots()
    found = solids(reader.OneShape())
    if len(found) != 1:
        raise ValueError(f"expected one native solid, found {len(found)}")
    return found[0]


def volume(shape: Any) -> tuple[float, float]:
    props = GProp_GProps()
    estimate = BRepGProp.VolumeProperties_s(shape, props, 1e-10, True, False)
    return props.Mass(), estimate


def checked_boolean(kind: Any, first: Any, second: Any) -> Any:
    operation = kind(first, second)
    operation.Build()
    if not operation.IsDone() or operation.Shape().IsNull():
        raise ValueError("Boolean failed")
    result = operation.Shape()
    if not BRepCheck_Analyzer(result).IsValid():
        raise ValueError("Boolean output is invalid")
    return result


def component(shape: Any) -> dict[str, Any]:
    props = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, props, 1e-10, True, False)
    box = Bnd_Box()
    BRepBndLib.AddOptimal_s(shape, box, False, False)
    points = [box.CornerMin(), box.CornerMax()]
    return {"volumeMm3": props.Mass(), "centroidMm": [props.CentreOfMass().X(), props.CentreOfMass().Y(), props.CentreOfMass().Z()], "boundsMm": [[p.X(), p.Y(), p.Z()] for p in points]}


def inspect(source_path: Path, native_path: Path) -> dict[str, Any]:
    source, native = source_enclosure(source_path), native_solid(native_path)
    if not BRepCheck_Analyzer(source).IsValid() or not BRepCheck_Analyzer(native).IsValid():
        raise ValueError("source or native solid is invalid")
    extra = checked_boolean(BRepAlgoAPI_Cut, native, source)
    missing = checked_boolean(BRepAlgoAPI_Cut, source, native)
    common = checked_boolean(BRepAlgoAPI_Common, source, native)
    (source_mass, source_error), (native_mass, native_error), (common_mass, common_error) = volume(source), volume(native), volume(common)
    (extra_mass, extra_error), (missing_mass, missing_error) = volume(extra), volume(missing)
    source_balance = source_mass - common_mass - missing_mass
    native_balance = native_mass - common_mass - extra_mass
    source_estimate = abs(source_mass * source_error) + abs(common_mass * common_error) + abs(missing_mass * missing_error)
    native_estimate = abs(native_mass * native_error) + abs(common_mass * common_error) + abs(extra_mass * extra_error)
    return {"sourceProduct": PRODUCT, "sourceSolidCount": 1, "nativeSolidCount": 1,
            "sourceConservationWithinIntegrationEstimate": abs(source_balance) <= source_estimate,
            "nativeConservationWithinIntegrationEstimate": abs(native_balance) <= native_estimate,
            "integrationRelativeErrorEstimates": {"source": source_error, "native": native_error, "common": common_error, "extra": extra_error, "missing": missing_error},
            "sourceMm3": source_mass, "nativeMm3": native_mass, "commonMm3": common_mass,
            "nativeMinusSourceMm3": extra_mass, "sourceMinusNativeMm3": missing_mass,
            "sourceConservationErrorMm3": source_balance,
            "nativeConservationErrorMm3": native_balance,
            "extraComponents": sorted((component(s) for s in solids(extra)), key=lambda x: x["volumeMm3"], reverse=True),
            "missingComponents": sorted((component(s) for s in solids(missing)), key=lambda x: x["volumeMm3"], reverse=True)}


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: inspect_framework_residual.py SOURCE_STEP NATIVE_STEP")
    print(json.dumps(inspect(Path(sys.argv[1]), Path(sys.argv[2])), indent=2))
