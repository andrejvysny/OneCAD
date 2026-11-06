#!/usr/bin/env python3
"""Fit diagnostic analytic surfaces to samples of the pinned Framework STEP."""
from __future__ import annotations

import hashlib
import json
import math
import sys
from pathlib import Path

from OCP.BRepAdaptor import BRepAdaptor_Surface
from OCP.BRepClass import BRepClass_FaceClassifier
from OCP.GeomAbs import GeomAbs_Cylinder
from OCP.STEPControl import STEPControl_Reader
from OCP.TopAbs import TopAbs_FACE, TopAbs_IN, TopAbs_ON, TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS
from OCP.gp import gp_Pnt2d

SOURCE_SHA = "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09"
ADJACENCY_SHA = "873aac15e36c0894fe8256b986eac8e94915242d55d3ebaa2d948e5a924d3978"
CONNECTIVITY_UNCERTAINTY_MM = 0.004470832106062  # Source STEP #8144; not a fit tolerance.
READER_CONFUSION_MM = 1e-7  # OCCT Precision::Confusion; not a source precision claim.


def solve(matrix: list[list[float]], values: list[float]) -> list[float] | None:
    augmented = [row[:] + [value] for row, value in zip(matrix, values)]
    for column in range(4):
        pivot = max(range(column, 4), key=lambda row: abs(augmented[row][column]))
        if abs(augmented[pivot][column]) < 1e-20:
            return None
        augmented[column], augmented[pivot] = augmented[pivot], augmented[column]
        factor = augmented[column][column]
        augmented[column] = [value / factor for value in augmented[column]]
        for row in range(4):
            if row != column:
                factor = augmented[row][column]
                augmented[row] = [a-factor*b for a, b in zip(augmented[row], augmented[column])]
    return [augmented[row][4] for row in range(4)]


def sphere_fit(points: list[list[float]]) -> dict[str, object] | None:
    mean = [sum(point[i] for point in points)/len(points) for i in range(3)]
    local = [[point[i]-mean[i] for i in range(3)] for point in points]
    rows = [[2*p[0],2*p[1],2*p[2],1.0] for p in local]
    rhs = [sum(v*v for v in point) for point in local]
    matrix = [[sum(row[i]*row[j] for row in rows) for j in range(4)] for i in range(4)]
    values = [sum(row[i]*value for row, value in zip(rows, rhs)) for i in range(4)]
    answer = solve(matrix, values)
    if answer is None:
        return None
    center = [mean[i]+answer[i] for i in range(3)]
    radius = math.sqrt(max(0.0, answer[3]+sum(v*v for v in answer[:3])))
    residuals = [abs(math.dist(point,center)-radius) for point in points]
    return {"centerMm": center, "radiusMm": radius, "maxResidualMm": max(residuals)}


def points_on_face(face: object) -> list[list[float]]:
    surface = BRepAdaptor_Surface(face)
    u0,u1 = surface.FirstUParameter(),surface.LastUParameter()
    v0,v1 = surface.FirstVParameter(),surface.LastVParameter()
    result = []
    for i in range(1,20):
        for j in range(1,20):
            u=u0+(u1-u0)*i/20; v=v0+(v1-v0)*j/20
            classification = BRepClass_FaceClassifier(face,gp_Pnt2d(u,v),READER_CONFUSION_MM).State()
            if classification in (TopAbs_IN,TopAbs_ON):
                point=surface.Value(u,v)
                result.append([point.X(),point.Y(),point.Z()])
    return result


def cylinder_residual(points: list[list[float]], face: object) -> dict[str, object]:
    cylinder = BRepAdaptor_Surface(face).Cylinder()
    axis=cylinder.Axis(); origin=axis.Location(); direction=axis.Direction()
    o=[origin.X(),origin.Y(),origin.Z()]; d=[direction.X(),direction.Y(),direction.Z()]
    def radius(point: list[float]) -> float:
        v=[point[i]-o[i] for i in range(3)]; projection=sum(v[i]*d[i] for i in range(3))
        return math.sqrt(sum((v[i]-projection*d[i])**2 for i in range(3)))
    return {"radiusMm":cylinder.Radius(),"maxResidualMm":max(abs(radius(p)-cylinder.Radius()) for p in points)}


def inspect(source: Path, adjacency: Path) -> dict[str, object]:
    if hashlib.sha256(source.read_bytes()).hexdigest()!=SOURCE_SHA:
        raise ValueError("source checksum mismatch")
    data=adjacency.read_bytes()
    if hashlib.sha256(data).hexdigest()!=ADJACENCY_SHA:
        raise ValueError("adjacency checksum mismatch")
    saved=json.loads(data)
    if saved["sourceSha256"]!=SOURCE_SHA:
        raise ValueError("adjacency source mismatch")
    reader=STEPControl_Reader()
    if str(reader.ReadFile(str(source)))!="IFSelect_ReturnStatus.IFSelect_RetDone":
        raise ValueError("source STEP refused")
    reader.TransferRoots(); solids=TopExp_Explorer(reader.OneShape(),TopAbs_SOLID); solids.Next()
    explorer=TopExp_Explorer(solids.Current(),TopAbs_FACE); faces=[]
    while explorer.More():
        faces.append(TopoDS.Face(explorer.Current())); explorer.Next()
    result=[]
    for item in saved["faces"]:
        index=item["faceIndex"]; points=points_on_face(faces[index])
        if len(points)<8:
            raise ValueError(f"insufficient trimmed samples on face {index}: {len(points)}")
        cylinders=[]
        for neighbor in item["neighbors"]:
            k=neighbor["faceIndex"]
            if BRepAdaptor_Surface(faces[k]).GetType()==GeomAbs_Cylinder:
                cylinders.append({"faceIndex":k,**cylinder_residual(points,faces[k])})
        result.append({"faceIndex":index,"sampleCount":len(points),"sphereFit":sphere_fit(points),"adjacentCylinderComparisons":cylinders})
    return {"sourceSha256":SOURCE_SHA,"method":"19x19 interior UV grid, trimmed to face; centered least-squares sphere fit and adjacent source cylinders","sourceConnectivityUncertaintyMm":CONNECTIVITY_UNCERTAINTY_MM,"readerConfusionMm":READER_CONFUSION_MM,"qualification":"Finite samples cannot establish exact surface identity. Connectivity uncertainty and reader confusion are not source fit acceptance tolerances.","faces":result}


if __name__=="__main__":
    if len(sys.argv)!=4:
        raise SystemExit("usage: inspect_framework_surfaces.py PINNED_STEP PINNED_ADJACENCY OUTPUT_JSON")
    Path(sys.argv[3]).write_text(json.dumps(inspect(Path(sys.argv[1]),Path(sys.argv[2])),indent=2,allow_nan=False)+"\n")
