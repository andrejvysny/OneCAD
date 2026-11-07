"""Measure the pinned upstream mesh without OCCT or OneCAD geometry output."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
from collections import Counter
from pathlib import Path

MAX_BYTES = 16 * 1024 * 1024
SOURCE_SHA = "720e305f4a783a706a08091f62a4ac0eef89335e7d0c78fcb6bdae4f58cada73"
Point = tuple[float, float, float]


def cross(a: Point, b: Point) -> Point:
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def subtract(a: Point, b: Point) -> Point:
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def measure(data: bytes) -> dict[str, object]:
    if not 84 <= len(data) <= MAX_BYTES:
        raise ValueError("STL byte budget or header refused")
    count = struct.unpack_from("<I", data, 80)[0]
    if count == 0 or len(data) != 84 + 50 * count:
        raise ValueError("STL triangle count and exact byte length disagree")
    triangles: list[tuple[Point, Point, Point]] = []
    edges: Counter[tuple[Point, Point]] = Counter()
    for index in range(count):
        row = struct.unpack_from("<12fH", data, 84 + 50 * index)
        points = tuple(tuple(row[start:start + 3]) for start in (3, 6, 9))
        if not all(math.isfinite(value) for point in points for value in point):
            raise ValueError("non-finite vertex")
        a, b, c = points
        if math.hypot(*cross(subtract(b, a), subtract(c, a))) == 0:
            raise ValueError("degenerate triangle")
        triangles.append((a, b, c))
        edges.update(((a, b), (b, c), (c, a)))
    if any(edges[(b, a)] != uses or uses != 1 for (a, b), uses in edges.items()):
        raise ValueError("mesh is not closed with opposite edge winding")
    vertices = {point for triangle in triangles for point in triangle}
    low = tuple(min(point[axis] for point in vertices) for axis in range(3))
    high = tuple(max(point[axis] for point in vertices) for axis in range(3))
    origin = tuple((a + b) / 2 for a, b in zip(low, high, strict=True))
    volumes = [signed_tetrahedron(a, b, c, origin) for a, b, c in triangles]
    area = math.fsum(
        math.hypot(*cross(subtract(b, a), subtract(c, a))) / 2
        for a, b, c in triangles
    )
    volume = math.fsum(volumes)
    if volume <= 0:
        raise ValueError("mesh is not outward wound with positive signed volume")
    components = connected_components(triangles)
    euler = len(vertices) - len(edges) // 2 + count
    return {
        "connectedComponents": components,
        "eulerCharacteristic": euler,
        "triangleCount": count,
        "uniqueVertices": len(vertices),
        "closedOppositeWinding": True,
        "boundsMm": [low, high],
        "signedVolumeMm3": volume,
        "surfaceAreaMm2": area,
    }


def signed_tetrahedron(a: Point, b: Point, c: Point, origin: Point) -> float:
    product = cross(subtract(b, origin), subtract(c, origin))
    return sum(x * y for x, y in zip(subtract(a, origin), product, strict=True)) / 6


def connected_components(triangles: list[tuple[Point, Point, Point]]) -> int:
    owners: dict[tuple[Point, Point], int] = {}
    neighbors: list[set[int]] = [set() for _ in triangles]
    for index, (a, b, c) in enumerate(triangles):
        for edge in ((a, b), (b, c), (c, a)):
            other = owners.get((edge[1], edge[0]))
            if other is not None:
                neighbors[index].add(other)
                neighbors[other].add(index)
            owners[edge] = index
    unseen = set(range(len(triangles)))
    components = 0
    while unseen:
        components += 1
        pending = [unseen.pop()]
        while pending:
            for index in neighbors[pending.pop()] & unseen:
                unseen.remove(index)
                pending.append(index)
    return components


def source_report(data: bytes, source_sha: str) -> dict[str, object]:
    return {
        "schemaVersion": 1,
        "sourceSha256": source_sha,
        "sourceRevision": "853bc30c4b10190f1d669ed6d0a567e333c28f21",
        "reader": "Python standard library binary STL and oriented triangle sums; no OCCT",
        "units": "mm by upstream project convention; not embedded STL metadata",
        "observation": measure(data),
        "comparisonLimits": {"boundsMmTrial": 0.15, "relativeVolumeTrial": 0.02},
        "posePolicy": "Coarse sorted-span comparison assumes axis-permuted STL/STEP frames; that alignment is unqualified. Only volume is invariant under arbitrary rigid motion.",
        "limits": [
            "Upstream tessellation is independent of OneCAD output, not exact BRep or physical metrology.",
            "Meshing deflection and export engine are not certified.",
            "Closed opposite winding and connectivity do not establish absence of self-intersections.",
            "Trial comparison bounds reuse existing Prusa diagnostic values; they do not ratify physical or exact-surface accuracy.",
            "Bounds and volume do not uniquely identify a shape or prove hole geometry.",
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-sha256", default=SOURCE_SHA)
    parser.add_argument("source", type=Path)
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    with args.source.open("rb") as source:
        data = source.read(MAX_BYTES + 1)
    if hashlib.sha256(data).hexdigest() != args.expected_sha256:
        raise ValueError("upstream STL hash mismatch")
    report = source_report(data, args.expected_sha256)
    with args.report.open("x", encoding="utf-8") as output:
        json.dump(report, output, indent=2)
        output.write("\n")


if __name__ == "__main__":
    main()
