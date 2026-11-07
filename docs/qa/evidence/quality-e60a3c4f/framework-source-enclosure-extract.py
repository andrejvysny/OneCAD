"""Read-only STEP/XCAF occurrence and BRepCheck audit of pinned Framework source."""
import hashlib
import json
from pathlib import Path
import OCP
from OCP.STEPCAFControl import STEPCAFControl_Reader
from OCP.TDocStd import TDocStd_Document
from OCP.TCollection import TCollection_ExtendedString
from OCP.TDataStd import TDataStd_Name
from OCP.TDF import TDF_Label
from OCP.XCAFDoc import XCAFDoc_DocumentTool, XCAFDoc_ShapeTool
from OCP.collections import Sequence_TDF_Label
from OCP.TopLoc import TopLoc_Location
from OCP.TopExp import TopExp_Explorer
from OCP.TopAbs import TopAbs_SOLID, TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX
from OCP.GProp import GProp_GProps
from OCP.BRepGProp import BRepGProp
from OCP.BRepCheck import BRepCheck_Analyzer
from OCP.BRepTools import BRepTools

source = Path('/private/tmp/onecad-native-public-assembly/fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp')
sha = hashlib.sha256(source.read_bytes()).hexdigest()
assert sha == '6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09'
assert OCP.__version__ == '8.0.1.1'
reader = STEPCAFControl_Reader()
reader.SetNameMode(True)
assert 'RetDone' in str(reader.ReadFile(str(source)))
doc = TDocStd_Document(TCollection_ExtendedString('MDTV-XCAF'))
assert reader.Transfer(doc)
tool = XCAFDoc_DocumentTool.ShapeTool_s(doc.Main())
roots = Sequence_TDF_Label()
tool.GetFreeShapes(roots)
rows = []

def name(label: TDF_Label) -> str:
    attr = TDataStd_Name()
    return attr.Get().ToExtString() if label.FindAttribute(TDataStd_Name.GetID_s(), attr) else ''

def walk(label: TDF_Label, parent: TopLoc_Location, parents: list[dict[str, str]]) -> None:
    ref = TDF_Label()
    base = ref if XCAFDoc_ShapeTool.GetReferredShape_s(label, ref) else label
    world = parent.Multiplied(XCAFDoc_ShapeTool.GetLocation_s(label))
    path = parents + [{'occurrence': name(label), 'definition': name(base)}]
    children = Sequence_TDF_Label()
    XCAFDoc_ShapeTool.GetComponents_s(base, children)
    if children.Length():
        for i in range(1, children.Length() + 1):
            walk(children.Value(i), world, path)
        return
    shape = XCAFDoc_ShapeTool.GetShape_s(base)
    solids = TopExp_Explorer(shape, TopAbs_SOLID)
    while solids.More():
        solid = solids.Current()
        if name(base) == "FW_EXP_1USBC_FRAME_CLIP_BC_229_":
            assert all(world.Transformation().Value(i, j) == (1.0 if i == j else 0.0) for i in range(1, 4) for j in range(1, 5)), "unexpected frame occurrence transform"
            assert BRepTools.Write_s(solid, "/private/tmp/onecad-resume-evidence/framework-source-enclosure-raw.brep")
        props = GProp_GProps()
        BRepGProp.VolumeProperties_s(solid, props)
        check = BRepCheck_Analyzer(solid)
        invalid = []
        for label_kind, kind in [('face', TopAbs_FACE), ('edge', TopAbs_EDGE), ('vertex', TopAbs_VERTEX)]:
            explorer = TopExp_Explorer(solid, kind)
            ordinal = 0
            while explorer.More():
                ordinal += 1
                subshape = explorer.Current()
                if not check.IsValid(subshape):
                    status = [str(item) for item in check.Result(subshape).Status()]
                    invalid.append({'kind': label_kind, 'ordinal': ordinal, 'status': status})
                explorer.Next()
        transform = world.Transformation()
        matrix = [[transform.Value(i, j) for j in range(1, 5)] for i in range(1, 4)]
        rows.append({'path': path, 'worldMatrix': matrix, 'volumeMm3SameKernel': props.Mass(),
                     'valid': check.IsValid(), 'invalidSubshapes': invalid})
        solids.Next()

for i in range(1, roots.Length() + 1):
    walk(roots.Value(i), TopLoc_Location(), [])
print(json.dumps({'source': str(source), 'sourceSha256': sha, 'reader': f'OCP {OCP.__version__} STEPCAFControl_Reader',
                  'scope': 'source occurrence graph before OneCAD import/heal; same-kernel diagnostic, not independent mass oracle',
                  'sourceEntityLinks': {'pcba': [1692,1693,1694,1685,1672], 'frame': [8155,8156,8157,8150,8137],
                      'nut': [8442,8443,8444,8438,8425], 'screw': [9502,9503,9504,9498,9485],
                      'nutOccurrences': [8445,8458], 'screwOccurrences': [9505,9518]},
                  'occurrenceCount': len(rows), 'occurrences': rows}, indent=2))
