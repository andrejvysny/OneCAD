"""Compare posed source XBF solids to a flattened STEP using an independent OCP reader.

This uses the same OCCT family as the worker, so it is not an independent kernel.
It reports measurements and provenance; it does not invent a pass tolerance.
"""
import argparse
import hashlib
import itertools
import json
import math
from pathlib import Path

import OCP
from OCP.BinXCAFDrivers import BinXCAFDrivers
from OCP.BRepAlgoAPI import BRepAlgoAPI_Common
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepBuilderAPI import BRepBuilderAPI_Transform
from OCP.BRepCheck import BRepCheck_Analyzer
from OCP.BRepGProp import BRepGProp
from OCP.Bnd import Bnd_Box
from OCP.collections import Sequence_TDF_Label, Sequence_TCollection_AsciiString
from OCP.GProp import GProp_GProps
from OCP.gp import gp_Ax1, gp_Dir, gp_Pnt, gp_Trsf, gp_Vec
from OCP.STEPControl import STEPControl_Reader
from OCP.TCollection import TCollection_ExtendedString
from OCP.TDocStd import TDocStd_Application, TDocStd_Document
from OCP.TopAbs import TopAbs_FACE, TopAbs_SOLID
from OCP.TopExp import TopExp_Explorer
from OCP.XCAFDoc import XCAFDoc_DocumentTool, XCAFDoc_ShapeTool

HASHES = {
    '010': 'f643cfdf9ec480a23d14888bf896cad30e02bd05be079a0baf11f7fc88bd1992',
    '020': '932cc0452078850d9059ed6ad0716dd3658d35b8117c5598c2f0699e2b14b21c',
    '030': 'd9880bc3f2d82b2e9cf49919603ada70af7e3f4c7d9d60caeb3f81a26fc28d8e',
    '040': '2e780afa05c59f10fb3a8c4acf1cc0c2289d45312dedc0172bc7fa83ecbeffc0',
}


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_sources(root: Path) -> dict[str, object]:
    app = TDocStd_Application()
    BinXCAFDrivers.DefineFormat_s(app)
    result = {}
    for code, expected in HASHES.items():
        path = root / f'part-{code}.xbf'
        assert sha(path) == expected, f'source digest mismatch: {path}'
        document = TDocStd_Document(TCollection_ExtendedString('BinXCAF'))
        status = app.Open(TCollection_ExtendedString(str(path)), document)
        assert str(status).endswith('PCDM_RS_OK'), str(status)
        document = app.GetDocument(app.NbDocuments())
        labels = Sequence_TDF_Label()
        XCAFDoc_DocumentTool.ShapeTool_s(document.Main()).GetFreeShapes(labels)
        assert labels.Length() == 1
        result[code] = XCAFDoc_ShapeTool.GetShape_s(labels.Value(1))
        app.Close(document)
    return result


def placed(shape: object, radians: float, dx: float) -> object:
    transform = gp_Trsf()
    transform.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), radians)
    transform.SetTranslationPart(gp_Vec(dx, 0, 0))
    builder = BRepBuilderAPI_Transform(shape, transform, False)
    assert builder.IsDone()
    return builder.Shape()


def metrics(shape: object) -> dict[str, object]:
    box = Bnd_Box()
    BRepBndLib.AddOptimal_s(shape, box, False, False)
    lo, hi = box.CornerMin(), box.CornerMax()
    properties = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, properties)
    volume = properties.Mass()
    center = properties.CentreOfMass()
    centroid = [center.X(), center.Y(), center.Z()]
    adaptive = GProp_GProps()
    estimate = BRepGProp.VolumeProperties_s(shape, adaptive, 1e-9)
    surface = GProp_GProps()
    BRepGProp.SurfaceProperties_s(shape, surface)
    explorer = TopExp_Explorer(shape, TopAbs_FACE)
    count = 0
    while explorer.More():
        count += 1
        explorer.Next()
    return {'boundsMm': [[lo.X(), lo.Y(), lo.Z()], [hi.X(), hi.Y(), hi.Z()]],
            'volumeMm3': volume, 'adaptiveVolumeMm3': adaptive.Mass(),
            'adaptiveRelativeEstimate': estimate, 'centroidMm': centroid,
            'surfaceMm2': surface.Mass(), 'faces': count,
            'valid': BRepCheck_Analyzer(shape).IsValid()}


def read_step(path: Path) -> tuple[list[object], dict[str, list[str]]]:
    reader = STEPControl_Reader()
    assert 'RetDone' in str(reader.ReadFile(str(path)))
    length, angle, solid_angle = (Sequence_TCollection_AsciiString() for _ in range(3))
    reader.FileUnits(length, angle, solid_angle)
    units = {'length': [length.Value(i).ToCString() for i in range(1, length.Length() + 1)],
             'angle': [angle.Value(i).ToCString() for i in range(1, angle.Length() + 1)]}
    reader.SetSystemLengthUnit(1.0)
    assert reader.TransferRoots() > 0
    explorer = TopExp_Explorer(reader.OneShape(), TopAbs_SOLID)
    result = []
    while explorer.More():
        result.append(explorer.Current())
        explorer.Next()
    return result, units


def score(expected: dict[str, object], actual: dict[str, object]) -> float:
    return max(abs(expected['boundsMm'][side][axis] - actual['boundsMm'][side][axis])
               for side in range(2) for axis in range(3))


def common_volume(a: object, b: object) -> dict[str, object]:
    boolean = BRepAlgoAPI_Common(a, b)
    boolean.Build()
    assert boolean.IsDone(), 'same-kernel Boolean comparison failed'
    properties = GProp_GProps()
    BRepGProp.VolumeProperties_s(boolean.Shape(), properties)
    return {'volumeMm3': properties.Mass(), 'valid': BRepCheck_Analyzer(boolean.Shape()).IsValid()}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument('step', type=Path)
    parser.add_argument('source_dir', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    assert OCP.__version__ == '8.0.1.1', OCP.__version__
    source = load_sources(args.source_dir)
    expected = [placed(source['010'], 0, 0), placed(source['020'], 0, -1),
                placed(source['030'], 0, -1), placed(source['040'], -0.2, 0),
                placed(source['020'], math.pi, 2), placed(source['030'], math.pi, 2)]
    actual, units = read_step(args.step)
    assert len(actual) == 6, f'expected 6 solids; got {len(actual)}'
    assert units['length'] and all(unit.lower() == 'millimetre' for unit in units['length']), units
    expected_metrics = [metrics(s) for s in expected]
    actual_metrics = [metrics(s) for s in actual]
    assert all(m['valid'] for m in expected_metrics + actual_metrics), 'invalid source or STEP solid'
    assignment = min(itertools.permutations(range(6)),
                     key=lambda p: sum(score(expected_metrics[i], actual_metrics[p[i]]) for i in range(6)))
    rows = []
    for i, j in enumerate(assignment):
        common = common_volume(expected[i], actual[j])
        ev, av = expected_metrics[i]['volumeMm3'], actual_metrics[j]['volumeMm3']
        bounds_error = score(expected_metrics[i], actual_metrics[j])
        assert bounds_error <= 0.001, f'occurrence {i} bounds error {bounds_error} mm'
        rows.append({'occurrence': i, 'sourceCode': ['010','020','030','040','020','030'][i],
                     'stepSolidDiagnosticIndex': j, 'expected': expected_metrics[i],
                     'actual': actual_metrics[j], 'maxBoundsErrorMm': bounds_error,
                     'commonBooleanValid': common['valid'], 'commonVolumeMm3': common['volumeMm3'],
                     'nonadaptiveVolumeConservationMm3': ev + av - 2 * common['volumeMm3']})
    report = {'scope': 'separate OCP reader, same OCCT family; bounds tolerance 0.001 mm; mass and Boolean values are diagnostics',
              'boundsToleranceMm': 0.001, 'ocpVersion': OCP.__version__, 'stepSha256': sha(args.step),
              'sourceSha256': HASHES, 'declaredUnits': units, 'solidCount': len(actual), 'rows': rows}
    with args.output.open('x') as stream:
        json.dump(report, stream, indent=2)
        stream.write('\n')


if __name__ == '__main__':
    main()
