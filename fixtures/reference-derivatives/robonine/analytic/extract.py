"""Extract four pinned Robonine leaf parts; excludes all other assembly bodies.

Copyright (c) 2026 OneCAD contributors. SPDX-License-Identifier: MIT
Generated hardware remains Copyright (c) 2025 Robonine, CERN-OHL-P-2.0.
"""
import hashlib
import json
import sys
from pathlib import Path
import OCP
from OCP.STEPCAFControl import STEPCAFControl_Reader
from OCP.TCollection import TCollection_ExtendedString as S
from OCP.TDocStd import TDocStd_Document, TDocStd_Application, TDocStd_FormatVersion
from OCP.BinXCAFDrivers import BinXCAFDrivers
from OCP.TDataStd import TDataStd_Name
from OCP.TDF import TDF_Label
from OCP.XCAFDoc import XCAFDoc_DocumentTool, XCAFDoc_ShapeTool, XCAFDoc_ColorTool, XCAFDoc_ColorGen, XCAFDoc_ColorSurf, XCAFDoc_ColorCurv
from OCP.Quantity import Quantity_ColorRGBA
from OCP.collections import Sequence_TDF_Label, IndexedMap_TopoDS_Shape_TopTools_ShapeMapHasher
from OCP.TopAbs import TopAbs_SOLID, TopAbs_FACE
from OCP.TopExp import TopExp, TopExp_Explorer
from OCP.BRepGProp import BRepGProp
from OCP.GProp import GProp_GProps
from OCP.Bnd import Bnd_Box
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepCheck import BRepCheck_Analyzer
from OCP.BRepBuilderAPI import BRepBuilderAPI_Transform
from OCP.TopLoc import TopLoc_Location
from OCP.TopoDS import TopoDS_Shape
import tempfile
if not __debug__:
    raise RuntimeError('run without Python optimization; extraction assertions are required')
if OCP.__version__ != '8.0.1.1':
    raise RuntimeError(f'OCP 8.0.1.1 required, found {OCP.__version__}')
if len(sys.argv) != 3:
    raise ValueError('usage: extract.py SOURCE.STEP NEW_OUTPUT_DIRECTORY')
original = Path(sys.argv[1])
output = Path(sys.argv[2])
if output.exists():
    raise FileExistsError(output)
if original.stat().st_size != 5731780:
    raise ValueError('pinned source size mismatch')
with original.open('rb') as stream:
    data = stream.read(5731781)
sha = hashlib.sha256(data).hexdigest()
if len(data) != 5731780 or sha != '3d10caca74f4e82e8d0b26c0ef2254e12dc1495c027eba1eae207944521d2104':
    raise ValueError('pinned source size or SHA-256 mismatch')
output.mkdir(parents=False)
captured = tempfile.TemporaryDirectory(prefix='onecad-robonine-captured-')
src = Path(captured.name) / 'source.step'
src.write_bytes(data)
del data
r = STEPCAFControl_Reader()
r.SetNameMode(True)
r.SetColorMode(True)
assert 'RetDone' in str(r.ReadFile(str(src)))
source = TDocStd_Document(S('MDTV-XCAF'))
assert r.Transfer(source)
tool = XCAFDoc_DocumentTool.ShapeTool_s(source.Main())
colors = XCAFDoc_DocumentTool.ColorTool_s(source.Main())
roots = Sequence_TDF_Label()
tool.GetFreeShapes(roots)
assert roots.Length() == 1
children = Sequence_TDF_Label()
XCAFDoc_ShapeTool.GetComponents_s(roots.Value(1), children)

def name(l: TDF_Label) -> str:
    a = TDataStd_Name()
    return a.Get().ToExtString() if l.FindAttribute(TDataStd_Name.GetID_s(), a) else ''

def faces(s: TopoDS_Shape) -> list[TopoDS_Shape]:
    m = IndexedMap_TopoDS_Shape_TopTools_ShapeMapHasher()
    TopExp.MapShapes_s(s, TopAbs_FACE, m)
    return [m.FindKey(i) for i in range(1, m.Extent() + 1)]

def stats(s: TopoDS_Shape) -> dict[str, object]:
    it = TopExp_Explorer(s, TopAbs_SOLID)
    count = 0
    while it.More():
        count += 1
        it.Next()
    p = GProp_GProps()
    err = BRepGProp.VolumeProperties_s(s, p, 1e-09)
    b = Bnd_Box()
    BRepBndLib.AddOptimal_s(s, b, False, False)
    x, y = (b.CornerMin(), b.CornerMax())
    return {'solids': count, 'faces': len(faces(s)), 'valid': BRepCheck_Analyzer(s).IsValid(), 'volume_mm3': p.Mass(), 'volume_relative_estimate': err, 'volume_integration_eps': 1e-09, 'bounds_mm': [[x.X(), x.Y(), x.Z()], [y.X(), y.Y(), y.Z()]]}

def color_value(c: Quantity_ColorRGBA) -> list[float]:
    rgb = c.GetRGB()
    return [rgb.Red(), rgb.Green(), rgb.Blue(), c.Alpha()]
app = TDocStd_Application()
BinXCAFDrivers.DefineFormat_s(app)
results = []
for occ, code, entity in [('NAUO1', '010', 48712), ('NAUO2', '020', 34184), ('NAUO3', '030', 16015), ('NAUO4', '040', 38241)]:
    label = next((children.Value(i) for i in range(1, children.Length() + 1) if name(children.Value(i)) == occ))
    ref = TDF_Label()
    assert XCAFDoc_ShapeTool.GetReferredShape_s(label, ref)
    nested = Sequence_TDF_Label()
    XCAFDoc_ShapeTool.GetComponents_s(ref, nested)
    assert nested.Length() == 0
    shape = XCAFDoc_ShapeTool.GetShape_s(label)
    local = XCAFDoc_ShapeTool.GetShape_s(ref)
    assert shape.ShapeType() == TopAbs_SOLID
    worldfaces, localfaces = (faces(shape), faces(local))
    assert len(worldfaces) == len(localfaces)
    before = stats(shape)
    assert before['solids'] == 1 and before['valid']
    assert before['faces'] == {'010': 181, '020': 168, '030': 108, '040': 96}[code]
    base = shape.Located(TopLoc_Location())
    mover = BRepBuilderAPI_Transform(base, shape.Location().Transformation(), True)
    baked = mover.Shape()
    assert baked.Location().IsIdentity()
    bakedfaces = faces(baked)
    assert len(bakedfaces) == len(worldfaces)
    outdoc = app.NewDocument(S('BinXCAF'), TDocStd_Document(S('BinXCAF')))[0]
    outdoc.ChangeStorageFormatVersion(TDocStd_FormatVersion.TDocStd_FormatVersion_VERSION_12)
    outtool = XCAFDoc_DocumentTool.ShapeTool_s(outdoc.Main())
    outcolors = XCAFDoc_DocumentTool.ColorTool_s(outdoc.Main())
    outlabel = outtool.AddShape(baked, False)
    TDataStd_Name.Set_s(outlabel, S(name(ref)))
    source_colors = []
    for typ in (XCAFDoc_ColorGen, XCAFDoc_ColorSurf, XCAFDoc_ColorCurv):
        c = Quantity_ColorRGBA()
        found = XCAFDoc_ColorTool.GetColor_s(label, typ, c) or XCAFDoc_ColorTool.GetColor_s(ref, typ, c)
        if found:
            outcolors.SetColor(outlabel, c, typ)
        source_colors.append(color_value(c) if found else None)
    facecolors = []
    for f, localface, bakedface in zip(worldfaces, localfaces, bakedfaces):
        sub = outtool.AddSubShape(outlabel, bakedface)
        assert not sub.IsNull(), (occ, worldfaces.index(f))
        cs = []
        for typ in (XCAFDoc_ColorGen, XCAFDoc_ColorSurf, XCAFDoc_ColorCurv):
            c = Quantity_ColorRGBA()
            found = colors.GetColor(f, typ, c) or colors.GetColor(localface, typ, c)
            if found:
                outcolors.SetColor(sub, c, typ)
            cs.append(color_value(c) if found else None)
        facecolors.append(cs)
    out = output / f'part-{code}.xbf'
    status = app.SaveAs(outdoc, S(str(out)))
    assert str(status).endswith('PCDM_SS_OK')
    app.Close(outdoc)
    data = out.read_bytes()
    assert data.startswith(b'BINFILE')
    empty = TDocStd_Document(S('BinXCAF'))
    status = app.Open(S(str(out)), empty)
    assert str(status).endswith('PCDM_RS_OK')
    read = app.GetDocument(app.NbDocuments())
    assert int(read.StorageFormatVersion()) == 12
    readtool = XCAFDoc_DocumentTool.ShapeTool_s(read.Main())
    readcolors = XCAFDoc_DocumentTool.ColorTool_s(read.Main())
    labels = Sequence_TDF_Label()
    readtool.GetFreeShapes(labels)
    assert labels.Length() == 1
    rl = labels.Value(1)
    recovered = XCAFDoc_ShapeTool.GetShape_s(rl)
    after = stats(recovered)
    assert after['solids'] == before['solids'] and after['faces'] == before['faces'] and after['valid']
    assert after['bounds_mm'] == before['bounds_mm']
    assert abs(after['volume_mm3'] - before['volume_mm3']) < 1e-09
    rc = []
    for typ in (XCAFDoc_ColorGen, XCAFDoc_ColorSurf, XCAFDoc_ColorCurv):
        c = Quantity_ColorRGBA()
        rc.append(color_value(c) if XCAFDoc_ColorTool.GetColor_s(rl, typ, c) else None)
    assert rc == source_colors
    rf = []
    for f in faces(recovered):
        cs = []
        for typ in (XCAFDoc_ColorGen, XCAFDoc_ColorSurf, XCAFDoc_ColorCurv):
            c = Quantity_ColorRGBA()
            cs.append(color_value(c) if readcolors.GetColor(f, typ, c) else None)
        rf.append(cs)
    assert rf == facecolors
    subs = Sequence_TDF_Label()
    XCAFDoc_ShapeTool.GetSubShapes_s(rl, subs)
    assert subs.Length() == len(worldfaces)
    app.Close(read)
    m = XCAFDoc_ShapeTool.GetLocation_s(label).Transformation()
    matrix = [[m.Value(i, j) for j in range(1, 5)] for i in range(1, 4)]
    results.append({'occurrence': occ, 'source_product_entity': entity, 'product_name': name(ref), 'source_referred_nested_components': 0, 'source_occurrence_matrix': matrix, 'first_pose': 'identity; source occurrence root geometry baked', 'before': before, 'after': after, 'face_subshape_labels': subs.Length(), 'root_color_slots': sum((x is not None for x in source_colors)), 'face_color_slots': sum((x is not None for row in facecolors for x in row)), 'colors_roundtrip_equal': True, 'output': out.name, 'output_bytes': len(data), 'output_sha256': hashlib.sha256(data).hexdigest()})
(output / 'observations.json').write_text(json.dumps({'source_sha256': sha, 'revision': 'e7cb78a7a1cd073ea8f845486398a23549343c7b', 'ocp': OCP.__version__, 'format': 'BinXCAF v12', 'parts': results}, indent=2) + '\n')
captured.cleanup()
