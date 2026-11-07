"""Private OCCT Cut lineage diagnostic on captured live operands."""
from __future__ import annotations
import hashlib, json
from pathlib import Path
from OCP.BinTools import BinTools
from OCP.BRepTools import BRepTools
from OCP.BRepAlgoAPI import BRepAlgoAPI_BooleanOperation
from OCP.BOPAlgo import BOPAlgo_CUT
from OCP.collections import List_TopoDS_Shape, IndexedMap_TopoDS_Shape_TopTools_ShapeMapHasher as ShapeMap
from OCP.BRepCheck import BRepCheck_Analyzer
from OCP.BRep import BRep_Tool
from OCP.BRepAdaptor import BRepAdaptor_Surface, BRepAdaptor_Curve
from OCP.BRepGProp import BRepGProp
from OCP.BRepBndLib import BRepBndLib
from OCP.Bnd import Bnd_Box
from OCP.GProp import GProp_GProps
from OCP.TopoDS import TopoDS, TopoDS_Shape
from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_SOLID
from OCP.TopExp import TopExp, TopExp_Explorer

base = Path('/private/tmp/onecad-framework-negative-cut-operands-427')
paths = [base / 'input-0.brep', base / 'input-1.brep']
hashes = [hashlib.sha256(p.read_bytes()).hexdigest() for p in paths]
shapes = []
for path in paths:
    shape = TopoDS_Shape()
    assert BinTools.Read_s(shape, str(path))
    shapes.append(shape)

def census(shape):
    v, a = GProp_GProps(), GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, v)
    BRepGProp.SurfaceProperties_s(shape, a)
    b = Bnd_Box(); BRepBndLib.AddOptimal_s(shape, b, False, False)
    lo, hi = b.CornerMin(), b.CornerMax()
    sm = ShapeMap(); TopExp.MapShapes_s(shape, TopAbs_SOLID, sm)
    return {'valid': BRepCheck_Analyzer(shape).IsValid(), 'solidCount': sm.Extent(),
            'volumeMm3': v.Mass(), 'areaMm2': a.Mass(),
            'boundsMm': [[lo.X(),lo.Y(),lo.Z()],[hi.X(),hi.Y(),hi.Z()]]}

a = List_TopoDS_Shape(); a.Append(shapes[0])
b = List_TopoDS_Shape(); b.Append(shapes[1])
op = BRepAlgoAPI_BooleanOperation(); op.SetOperation(BOPAlgo_CUT)
op.SetArguments(a); op.SetTools(b); op.SetNonDestructive(True); op.SetRunParallel(False)
op.SetUseOBB(False); op.Build()
assert op.IsDone() and not op.Shape().IsNull()
out = op.Shape(); outpath = Path('/private/tmp/onecad-resume-evidence/framework-negative-cut-raw.brep')
assert not outpath.exists() and BRepTools.Write_s(out, str(outpath))
faces, edges = ShapeMap(), ShapeMap()
TopExp.MapShapes_s(out, TopAbs_FACE, faces); TopExp.MapShapes_s(out, TopAbs_EDGE, edges)
source_maps=[]
for source in shapes:
    m=ShapeMap(); TopExp.MapShapes_s(source, TopAbs_FACE, m); source_maps.append(m)

def face_lineage(face):
    result=[]
    for side, m in enumerate(source_maps):
        for i in range(1,m.Extent()+1):
            original=m.FindKey(i)
            for label, group in [('unchanged', [original]), ('modified', op.Modified(original)), ('generated', op.Generated(original))]:
                if any(item.IsSame(face) for item in group):
                    result.append({'input':side,'faceIndex':i,'relation':label,'type':str(BRepAdaptor_Surface(TopoDS.Face(original)).GetType())})
    return result

nonmanifold=[]
for i in range(1,edges.Extent()+1):
    edge=TopoDS.Edge(edges.FindKey(i))
    if BRep_Tool.Degenerated_s(edge): continue
    uses=[]
    for j in range(1,faces.Extent()+1):
        face=TopoDS.Face(faces.FindKey(j)); it=TopExp_Explorer(face,TopAbs_EDGE)
        while it.More():
            use=it.Current()
            if use.IsSame(edge):
                props=GProp_GProps(); BRepGProp.SurfaceProperties_s(face,props)
                uses.append({'faceIndex':j,'faceOrientation':str(face.Orientation()),
                    'edgeOrientation':str(use.Orientation()), 'surface':str(BRepAdaptor_Surface(face).GetType()),
                    'faceAreaMm2':props.Mass(), 'lineage':face_lineage(face)})
            it.Next()
    if len(uses)<=2: continue
    c=BRepAdaptor_Curve(edge); p,q=c.Value(c.FirstParameter()),c.Value(c.LastParameter())
    nonmanifold.append({'edgeIndex':i, 'curve':str(c.GetType()), 'toleranceMm':BRep_Tool.Tolerance_s(edge),
                        'endpointsMm':[[p.X(),p.Y(),p.Z()],[q.X(),q.Y(),q.Z()]], 'uses':uses})
print(json.dumps({'scope':'private OCP x86_64 same-kernel diagnostic; no publication or source equivalence claim',
 'inputHashes':hashes, 'config':{'nonDestructive':True,'parallel':False,'useOBB':False,'fuzzyMm':op.FuzzyValue()},
 'target':census(shapes[0]),'tool':census(shapes[1]),'result':census(out),
 'rawOutputSha256':hashlib.sha256(outpath.read_bytes()).hexdigest(),'nonmanifold':nonmanifold,
 'inputHashesUnchanged':hashes==[hashlib.sha256(p.read_bytes()).hexdigest() for p in paths]},indent=2))
