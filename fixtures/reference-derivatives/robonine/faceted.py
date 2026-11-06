from guards import check_source, check_version, require_new
from typing import Any
import sys, struct, hashlib, json, time, collections, math
from pathlib import Path
import OCP
from OCP.gp import gp_Pnt
from OCP.BRepBuilderAPI import BRepBuilderAPI_MakeVertex, BRepBuilderAPI_MakeEdge, BRepBuilderAPI_MakeWire, BRepBuilderAPI_MakeFace, BRepBuilderAPI_MakeSolid
from OCP.BRep import BRep_Builder
from OCP.TopoDS import TopoDS, TopoDS_Shell, TopoDS_Compound
from OCP.BRepCheck import BRepCheck_Analyzer
from OCP.BRepTools import BRepTools
from OCP.BRepGProp import BRepGProp
from OCP.GProp import GProp_GProps
from OCP.Bnd import Bnd_Box
from OCP.BRepBndLib import BRepBndLib
name = sys.argv[1]
work = Path(sys.argv[2])
check_version()
check_source(name, work / f"{name}.stl")
require_new(work / f"{name}-faceted.brep")
src = work / f'{name}.stl'
data = src.read_bytes()
count = struct.unpack_from('<I', data, 80)[0]
assert len(data) == 84 + count * 50
faces = []
graph = collections.defaultdict(set)
totals = []
for i in range(count):
    p = 84 + 50 * i + 12
    raw = [data[p + j * 12:p + (j + 1) * 12] for j in range(3)]
    faces.append(raw)
    for a, b in zip(raw, raw[1:] + raw[:1]):
        graph[a].add(b)
        graph[b].add(a)
components = {}
sizes = []
while len(components) < len(graph):
    start = next((v for v in graph if v not in components))
    number = len(sizes)
    stack = [start]
    size = 0
    while stack:
        v = stack.pop()
        if v in components:
            continue
        components[v] = number
        size += 1
        stack.extend(graph[v] - components.keys())
    sizes.append(size)
builder = BRep_Builder()
shells = []
for _ in sizes:
    shell = TopoDS_Shell()
    builder.MakeShell(shell)
    shells.append(shell)
vertices = {}
edges = {}
signed = [[] for _ in sizes]

def vertex(raw: bytes) -> Any:
    if raw not in vertices:
        xyz = struct.unpack('<3f', raw)
        m = BRepBuilderAPI_MakeVertex(gp_Pnt(*xyz))
        vertices[raw] = m.Vertex()
    return vertices[raw]

def edge(a: bytes, b: bytes) -> Any:
    key = tuple(sorted((a, b)))
    if key not in edges:
        m = BRepBuilderAPI_MakeEdge(vertex(key[0]), vertex(key[1]))
        if not m.IsDone():
            raise RuntimeError('edge creation refused')
        edges[key] = m.Edge()
    return edges[key] if (a, b) == key else TopoDS.Edge(edges[key].Reversed())
for i, raw in enumerate(faces):
    idx = components[raw[0]]
    assert all((components[v] == idx for v in raw))
    wire = BRepBuilderAPI_MakeWire()
    for a, b in zip(raw, raw[1:] + raw[:1]):
        wire.Add(edge(a, b))
    if not wire.IsDone():
        raise RuntimeError(f'wire failed triangle {i}')
    face = BRepBuilderAPI_MakeFace(wire.Wire(), True)
    if not face.IsDone():
        raise RuntimeError(f'face failed triangle {i}')
    builder.Add(shells[idx], face.Face())
    a, b, c = [struct.unpack('<3f', v) for v in raw]
    signed[idx].append(sum((a[k] * (b[(k + 1) % 3] * c[(k + 2) % 3] - b[(k + 2) % 3] * c[(k + 1) % 3]) for k in range(3))) / 6)
solids = []
shell_data = []
for idx, shell in enumerate(shells):
    maker = BRepBuilderAPI_MakeSolid(shell)
    if not maker.IsDone():
        raise RuntimeError(f'solid creation refused shell {idx}')
    solid = maker.Solid()
    valid = BRepCheck_Analyzer(solid).IsValid()
    props = GProp_GProps()
    BRepGProp.VolumeProperties_s(solid, props)
    shell_data.append(dict(vertices=sizes[idx], signed_triangle_sum_mm3=math.fsum(signed[idx]), solid_valid=valid, volume_mm3=props.Mass()))
    solids.append(solid)
compound = TopoDS_Compound()
builder.MakeCompound(compound)
for solid in solids:
    builder.Add(compound, solid)
valid = BRepCheck_Analyzer(compound).IsValid()
box = Bnd_Box()
BRepBndLib.AddOptimal_s(compound, box, False, False)
out = work / f'{name}-faceted.brep'
assert BRepTools.Write_s(compound, str(out))
b = out.read_bytes()
print(json.dumps(dict(source_sha256=hashlib.sha256(data).hexdigest(), ocp=OCP.__version__, faces=count, vertices=len(vertices), edges=len(edges), shells=len(shells), solids=len(solids), per_shell=shell_data, compound_valid=valid, bounds=[[box.CornerMin().X(), box.CornerMin().Y(), box.CornerMin().Z()], [box.CornerMax().X(), box.CornerMax().Y(), box.CornerMax().Z()]], output=str(out), output_bytes=len(b), output_sha256=hashlib.sha256(b).hexdigest()), indent=2))
