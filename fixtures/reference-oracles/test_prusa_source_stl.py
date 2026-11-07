"""Analytic and malformed-input checks for the independent source mesh reader."""

import math
import struct
import unittest

from inspect_prusa_source_stl import measure

POINTS = ((0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0))
FACES = ((0, 2, 1), (0, 1, 3), (0, 3, 2), (1, 2, 3))


def tetrahedron(faces: tuple[tuple[int, int, int], ...] = FACES) -> bytes:
    records = []
    for face in faces:
        coordinates = [value for index in face for value in POINTS[index]]
        records.append(struct.pack("<12fH", 0.0, 0.0, 0.0, *coordinates, 0))
    return bytes(80) + struct.pack("<I", len(records)) + b"".join(records)


class MeshMeasurements(unittest.TestCase):
    def test_known_outward_tetrahedron(self) -> None:
        report = measure(tetrahedron())
        self.assertAlmostEqual(report["signedVolumeMm3"], 1 / 6)
        self.assertAlmostEqual(report["surfaceAreaMm2"], 1.5 + math.sqrt(3) / 2)
        self.assertEqual(report["connectedComponents"], 1)
        self.assertEqual(report["eulerCharacteristic"], 2)
        self.assertEqual(report["triangleCount"], 4)
        self.assertEqual(report["uniqueVertices"], 4)
        self.assertEqual(report["boundsMm"], [(0.0, 0.0, 0.0), (1.0, 1.0, 1.0)])

    def test_open_duplicate_and_inward_meshes_refused(self) -> None:
        for faces in (FACES[:3], FACES + (FACES[0],), tuple((a, c, b) for a, b, c in FACES)):
            with self.subTest(faces=faces), self.assertRaises(ValueError):
                measure(tetrahedron(faces))

    def test_inexact_length_and_nonfinite_vertex_refused(self) -> None:
        for data in (tetrahedron()[:-1], tetrahedron() + b"tail"):
            with self.assertRaises(ValueError):
                measure(data)
        bad = bytearray(tetrahedron())
        struct.pack_into("<f", bad, 84 + 12, math.nan)
        with self.assertRaises(ValueError):
            measure(bytes(bad))


if __name__ == "__main__":
    unittest.main()
