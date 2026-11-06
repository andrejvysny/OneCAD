#!/usr/bin/env python3
"""Diagnostic same-kernel boolean comparison of an exported native intermediate."""
from __future__ import annotations

import hashlib
import sys
from pathlib import Path

from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut
from OCP.BRepBuilderAPI import BRepBuilderAPI_Transform
from OCP.BRepGProp import BRepGProp
from OCP.GProp import GProp_GProps
from OCP.STEPControl import STEPControl_Reader
from OCP.TopAbs import TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.gp import gp_Trsf

SOURCE_SHA = "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09"


def solid(path: Path, index: int) -> object:
    reader = STEPControl_Reader()
    if str(reader.ReadFile(str(path))) != "IFSelect_ReturnStatus.IFSelect_RetDone":
        raise ValueError(f"STEP reader rejected {path}")
    reader.TransferRoots()
    explorer = TopExp_Explorer(reader.OneShape(), TopAbs_SOLID)
    for _ in range(index):
        if not explorer.More():
            raise ValueError("missing expected solid")
        explorer.Next()
    if not explorer.More():
        raise ValueError("missing expected solid")
    return explorer.Current()


def difference_volume(first: object, second: object) -> float:
    cut = BRepAlgoAPI_Cut(first, second)
    cut.Build()
    if not cut.IsDone():
        raise ValueError("boolean comparison failed")
    properties = GProp_GProps()
    BRepGProp.VolumeProperties_s(cut.Shape(), properties)
    return properties.Mass()


def compare(source: Path, intermediate: Path) -> None:
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA:
        raise ValueError("source checksum mismatch")
    source_shape = solid(source, 1)  # STEPCAF inventory identifies the enclosure.
    transform = gp_Trsf()
    transform.SetValues(1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0)
    source_shape = BRepBuilderAPI_Transform(source_shape, transform, True).Shape()
    intermediate_shape = solid(intermediate, 0)
    print(f"intermediate minus source: {difference_volume(intermediate_shape, source_shape)} mm³")
    print(f"source minus intermediate: {difference_volume(source_shape, intermediate_shape)} mm³")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: compare_framework_intermediate.py PINNED_STEP NATIVE_EXPORT")
    compare(Path(sys.argv[1]), Path(sys.argv[2]))
