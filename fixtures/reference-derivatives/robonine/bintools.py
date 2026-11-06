from guards import check_source, check_version, require_new
from typing import Any
import hashlib, json, sys
from pathlib import Path
import OCP
from OCP.TopoDS import TopoDS_Shape
from OCP.BRepTools import BRepTools
from OCP.BRep import BRep_Builder
from OCP.BinTools import BinTools, BinTools_FormatVersion
from OCP.TopAbs import TopAbs_SOLID, TopAbs_SHELL, TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX
from OCP.TopExp import TopExp
from OCP.collections import IndexedMap_TopoDS_Shape_TopTools_ShapeMapHasher as TopTools_IndexedMapOfShape
from OCP.BRepCheck import BRepCheck_Analyzer
from OCP.BRepGProp import BRepGProp
from OCP.GProp import GProp_GProps
from OCP.Bnd import Bnd_Box
from OCP.BRepBndLib import BRepBndLib
name = sys.argv[1]
work = Path(sys.argv[2])
check_version()
check_source(name, work / f"{name}.stl")
require_new(work / f"{name}.brep")
source = work / f'{name}-cavity.brep'
original = TopoDS_Shape()
assert BRepTools.Read_s(original, str(source), BRep_Builder())

def describe(shape: Any) -> dict[str, Any]:
    counts = {}
    for label, kind in [('solids', TopAbs_SOLID), ('shells', TopAbs_SHELL), ('faces', TopAbs_FACE), ('edges', TopAbs_EDGE), ('vertices', TopAbs_VERTEX)]:
        m = TopTools_IndexedMapOfShape()
        TopExp.MapShapes_s(shape, kind, m)
        counts[label] = m.Extent()
    props = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, props)
    box = Bnd_Box()
    BRepBndLib.AddOptimal_s(shape, box, False, False)
    return {'counts': counts, 'valid': BRepCheck_Analyzer(shape).IsValid(), 'volume_mm3': props.Mass(), 'bounds_mm': [[box.CornerMin().X(), box.CornerMin().Y(), box.CornerMin().Z()], [box.CornerMax().X(), box.CornerMax().Y(), box.CornerMax().Z()]]}
before = describe(original)
out = work / f'{name}.brep'
assert BinTools.Write_s(original, str(out), False, False, BinTools_FormatVersion.BinTools_FormatVersion_VERSION_4)
data = out.read_bytes()
header = b'\nOpen CASCADE Topology V4, (c) Open Cascade\n'
assert data.startswith(header)
readback = TopoDS_Shape()
assert BinTools.Read_s(readback, str(out))
after = describe(readback)
assert before['counts'] == after['counts'] == {'solids': 1, 'shells': 3, 'faces': 8808, 'edges': 13212, 'vertices': 4400}
assert before['valid'] and after['valid'] and (abs(before['volume_mm3'] - after['volume_mm3']) < 1e-08)
assert all((abs(a - b) < 1e-10 for x, y in zip(before['bounds_mm'], after['bounds_mm']) for a, b in zip(x, y)))
print(json.dumps({'ocp': OCP.__version__, 'format': 'BinTools topology V4; triangles=false, normals=false', 'input_sha256': hashlib.sha256(source.read_bytes()).hexdigest(), 'before': before, 'after': after, 'output': str(out), 'output_bytes': len(data), 'output_sha256': hashlib.sha256(data).hexdigest(), 'note': 'same-kernel readback only; no self-intersection, physical, native-history or independent-kernel qualification'}, indent=2))
