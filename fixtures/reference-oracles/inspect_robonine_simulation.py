#!/usr/bin/env python3
"""Measure immutable source meshes and source-authored poses without a CAD kernel."""
from __future__ import annotations

import collections
import hashlib
import json
import math
from pathlib import Path
import struct
import sys
import xml.etree.ElementTree as ET

REVISION = "e7cb78a7a1cd073ea8f845486398a23549343c7b"
HASHES = {
    "clamp_1": "a9540533009f872423431d8405613ee52a609e2d296a287c17867f941aae50ca",
    "clamp_2": "0a1c2caf47b97853e4dc0974efc5ed89f9969b89cae3c0b8f8d9e8705bff3498",
}
XACRO_SHA = "7d54a2f5b86ecdeec340618c187d757cbe9cf5cbe5fd52c7689081fda52e062e"


def verified_bytes(path: Path, expected: str) -> bytes:
    if path.stat().st_size > 1 << 20:
        raise ValueError("source exceeds bounded inspection size")
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError(f"source digest mismatch: {path.name}")
    return data


def connected(graph: dict[bytes, set[bytes]]) -> bool:
    todo = [next(iter(graph))]
    seen: set[bytes] = set()
    while todo:
        vertex = todo.pop()
        if vertex not in seen:
            seen.add(vertex)
            todo.extend(graph[vertex] - seen)
    return len(seen) == len(graph)


def inspect_mesh(path: Path, expected: str) -> dict[str, object]:
    data = verified_bytes(path, expected)
    count = struct.unpack_from("<I", data, 80)[0]
    if count != 8808 or len(data) != 84 + count * 50:
        raise ValueError("frozen binary STL inventory changed")
    edges: dict[tuple[bytes, bytes], list[tuple[bytes, bytes]]] = collections.defaultdict(list)
    links: dict[bytes, dict[bytes, set[bytes]]] = collections.defaultdict(lambda: collections.defaultdict(set))
    vertices: dict[bytes, tuple[float, float, float]] = {}
    faces: set[tuple[bytes, ...]] = set()
    signed_volumes: list[float] = []
    for index in range(count):
        offset = 84 + index * 50
        normal = struct.unpack_from("<3f", data, offset)
        raw = [data[offset + 12 + j * 12:offset + 24 + j * 12] for j in range(3)]
        points = [struct.unpack("<3f", item) for item in raw]
        if not all(math.isfinite(value) for point in [normal, *points] for value in point):
            raise ValueError("non-finite source mesh")
        ab = [points[1][k] - points[0][k] for k in range(3)]
        ac = [points[2][k] - points[0][k] for k in range(3)]
        cross = [ab[1] * ac[2] - ab[2] * ac[1], ab[2] * ac[0] - ab[0] * ac[2], ab[0] * ac[1] - ab[1] * ac[0]]
        if sum(value * value for value in cross) == 0 or sum(a * b for a, b in zip(normal, cross)) <= 0:
            raise ValueError("degenerate or opposed source triangle")
        face = tuple(sorted(raw))
        if face in faces:
            raise ValueError("duplicate source triangle")
        faces.add(face)
        vertices.update(zip(raw, points))
        for j in range(3):
            a, b, c = raw[j], raw[(j + 1) % 3], raw[(j + 2) % 3]
            edges[tuple(sorted((a, b)))].append((a, b))
            links[a][b].add(c)
            links[a][c].add(b)
        a, b, c = points
        signed_volumes.append(sum(a[k] * (b[(k + 1) % 3] * c[(k + 2) % 3] - b[(k + 2) % 3] * c[(k + 1) % 3]) for k in range(3)) / 6)
    if any(len(pair) != 2 or pair[0] != tuple(reversed(pair[1])) for pair in edges.values()):
        raise ValueError("source is not an oriented closed edge manifold")
    if any(any(len(neighbors) != 2 for neighbors in link.values()) or not connected(link) for link in links.values()):
        raise ValueError("source vertex link is not one cycle")
    return {"sha256": expected, "bytes": len(data), "triangles": count,
        "vertices": len(vertices), "edges": len(edges), "closedOrientedEdgeManifold": True,
        "vertexLinksAreSingleCycles": True, "selfIntersectionChecked": False,
        "signedTriangleSumMm3": math.fsum(signed_volumes),
        "boundsMm": [[function(point[k] for point in vertices.values()) for k in range(3)] for function in (min, max)]}


def vector(element: ET.Element, attribute: str) -> list[float]:
    return [float(value) for value in element.attrib[attribute].split()]


def inspect_poses(path: Path) -> dict[str, object]:
    root = ET.fromstring(verified_bytes(path, XACRO_SHA))
    result: dict[str, object] = {}
    for name in ("right_clamp", "left_clamp"):
        joint = root.find(f"joint[@name='{name}']")
        assert joint is not None and joint.attrib["type"] == "prismatic"
        child = joint.find("child").attrib["link"]
        visual = root.find(f"link[@name='{child}']/visual")
        origin = joint.find("origin")
        local = visual.find("origin")
        assert vector(origin, "rpy") == vector(local, "rpy") == [0.0] * 3
        assert vector(visual.find("geometry/mesh"), "scale") == [0.001] * 3
        axis = vector(joint.find("axis"), "xyz")
        norm = math.sqrt(sum(value * value for value in axis))
        limit = joint.find("limit")
        mimic = joint.find("mimic")
        result[child] = {"joint": name, "parent": joint.find("parent").attrib["link"],
            "q0MeshToParentTranslationMm": [1000 * (a + b) for a, b in zip(vector(origin, "xyz"), vector(local, "xyz"))],
            "q0RotationRadians": [0.0] * 3, "axisParentRaw": axis,
            "axisParentNormalized": [value / norm for value in axis],
            "simulationRangeMm": [1000 * float(limit.attrib[key]) for key in ("lower", "upper")],
            "mimic": None if mimic is None else dict(mimic.attrib)}
    return result


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: inspect_robonine_simulation.py ROBONINE_DIR OUTPUT_JSON")
    root = Path(sys.argv[1])
    report = {"status": "source-input observations only; no OneCAD, physical or STEP-equivalence qualification",
        "revision": REVISION, "units": "mm; source xacro scales STL by 0.001 to metres",
        "method": "Python standard library, exact binary-f32 vertex keys, analytic source pose composition; no CAD kernel",
        "meshes": {name: inspect_mesh(root / f"simulation/so_arm_101_description/meshes/visual/{name}.stl", digest) for name, digest in HASHES.items()},
        "xacroSha256": XACRO_SHA,
        "sourceAuthoredPoses": inspect_poses(root / "simulation/so_arm_101_description/urdf/so_101.urdf.xacro"),
        "poseArithmeticToleranceMm": 1e-9,
        "limitations": ["No self-intersection or physical volume certification", "No native feature history or equivalence to the quarantined STEP", "Mimic coupling unsupported; independent actuation is a separate authored fixture", "README 84 mm versus simulation common coupled range 74 mm is unresolved for physical design"]}
    Path(sys.argv[2]).write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
