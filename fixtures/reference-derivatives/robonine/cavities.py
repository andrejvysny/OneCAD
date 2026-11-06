from guards import check_source, check_version, require_new
from typing import Any
import sys, json, hashlib, time
from pathlib import Path
import OCP
from OCP.TopoDS import TopoDS, TopoDS_Shape
from OCP.TopExp import TopExp_Explorer
from OCP.TopAbs import TopAbs_SOLID, TopAbs_SHELL, TopAbs_VERTEX, TopAbs_IN, TopAbs_OUT
from OCP.BRep import BRep_Builder, BRep_Tool
from OCP.BRepTools import BRepTools
from OCP.BRepBuilderAPI import BRepBuilderAPI_MakeSolid
from OCP.BRepClass3d import BRepClass3d_SolidClassifier
from OCP.BRepExtrema import BRepExtrema_DistShapeShape
from OCP.BRepCheck import BRepCheck_Analyzer
from OCP.BRepGProp import BRepGProp
from OCP.GProp import GProp_GProps
from OCP.Bnd import Bnd_Box
from OCP.BRepBndLib import BRepBndLib
from OCP.Precision import Precision
name = sys.argv[1]
work = Path(sys.argv[2])
check_version()
check_source(name, work / f"{name}.stl")
require_new(work / f"{name}-cavity.brep")
inp = work / f'{name}-faceted.brep'
source = work / f'{name}.stl'
shape = TopoDS_Shape()
assert BRepTools.Read_s(shape, str(inp), BRep_Builder())

def explore(obj: Any, kind: Any, cast: Any) -> list[Any]:
    e = TopExp_Explorer(obj, kind)
    ret = []
    while e.More():
        ret.append(cast(e.Current()))
        e.Next()
    return ret
solids = explore(shape, TopAbs_SOLID, TopoDS.Solid)
assert len(solids) == 3
entries = []
for solid in solids:
    shells = explore(solid, TopAbs_SHELL, TopoDS.Shell)
    assert len(shells) == 1
    p = GProp_GProps()
    BRepGProp.VolumeProperties_s(solid, p)
    entries.append((p.Mass(), solid, shells[0]))
entries.sort(key=lambda x: x[0], reverse=True)
assert entries[0][0] > 0 and all((x[0] < 0 for x in entries[1:]))
outer = entries[0][1]
tol = Precision.Confusion_s()

def points(shell: Any) -> list[Any]:
    pts = {}
    for v in explore(shell, TopAbs_VERTEX, TopoDS.Vertex):
        p = BRep_Tool.Pnt_s(v)
        pts[p.X(), p.Y(), p.Z()] = p
    return list(pts.values())

def classify(solid: Any, ps: list[Any]) -> dict[str, int]:
    cl = BRepClass3d_SolidClassifier(solid)
    res = {'IN': 0, 'OUT': 0, 'OTHER': 0}
    for p in ps:
        cl.Perform(p, tol)
        s = cl.State()
        res['IN' if s == TopAbs_IN else 'OUT' if s == TopAbs_OUT else 'OTHER'] += 1
    return res
contain = []
for _, _, shell in entries[1:]:
    result = classify(outer, points(shell))
    contain.append(result)
    assert result['IN'] == len(points(shell))
distances = []
for i, j in ((0, 1), (0, 2), (1, 2)):
    print(f'{name} measuring shell separation {i}/{j}', file=sys.stderr, flush=True)
    dist = BRepExtrema_DistShapeShape(entries[i][2], entries[j][2])
    assert dist.IsDone()
    value = dist.Value()
    distances.append({'pair': [i, j], 'minimum_mm': value})
    assert value > tol
for i, j in ((1, 2), (2, 1)):
    positive = BRepBuilderAPI_MakeSolid(TopoDS.Shell(entries[j][2].Reversed())).Solid()
    r = classify(positive, points(entries[i][2]))
    assert r['OUT'] == len(points(entries[i][2]))
mk = BRepBuilderAPI_MakeSolid(entries[0][2])
mk.Add(entries[1][2])
mk.Add(entries[2][2])
assert mk.IsDone()
final = mk.Solid()
valid = BRepCheck_Analyzer(final).IsValid()
cl = BRepClass3d_SolidClassifier(final)
cl.PerformInfinitePoint(tol)
infinite = 'OUT' if cl.State() == TopAbs_OUT else 'OTHER'
props = GProp_GProps()
BRepGProp.VolumeProperties_s(final, props)
box = Bnd_Box()
BRepBndLib.AddOptimal_s(final, box, False, False)
out = work / f'{name}-cavity.brep'
assert BRepTools.Write_s(final, str(out))
data = out.read_bytes()
print(json.dumps({'input_sha256': hashlib.sha256(inp.read_bytes()).hexdigest(), 'stl_sha256': hashlib.sha256(source.read_bytes()).hexdigest(), 'ocp': OCP.__version__, 'precision_confusion_mm': tol, 'cavity_vertices_vs_outer': contain, 'shell_pair_minimum_distances': distances, 'cavities_mutually_outside': True, 'shells': len(explore(final, TopAbs_SHELL, TopoDS.Shell)), 'solids': 1, 'brepcheck_valid': valid, 'infinite_point': infinite, 'signed_volume_mm3': props.Mass(), 'bounds_mm': [[box.CornerMin().X(), box.CornerMin().Y(), box.CornerMin().Z()], [box.CornerMax().X(), box.CornerMax().Y(), box.CornerMax().Z()]], 'output': str(out), 'output_sha256': hashlib.sha256(data).hexdigest(), 'output_bytes': len(data), 'limitation': 'No global self-intersection proof; same-kernel only; no physical or native-history claim'}, indent=2))
