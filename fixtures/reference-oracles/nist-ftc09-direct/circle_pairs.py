#!/usr/bin/env python3
"""Exact rational diagnostics for FTC09's traced two-edge circular loops."""
from __future__ import annotations

import json
from collections import Counter
from fractions import Fraction
from pathlib import Path

import trace as source_trace

Vector = tuple[Fraction, Fraction, Fraction]


def rational(value: object) -> Fraction:
    if isinstance(value, bool):
        raise ValueError("boolean is not an exact scalar")
    return Fraction(value)


def vector(values: object) -> Vector:
    if not isinstance(values, (tuple, list)) or len(values) != 3:
        raise ValueError("expected three coordinates")
    return (rational(values[0]), rational(values[1]), rational(values[2]))


def subtract(a: Vector, b: Vector) -> Vector:
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def dot(a: Vector, b: Vector) -> Fraction:
    return sum((x * y for x, y in zip(a, b, strict=True)), Fraction())


def cross(a: Vector, b: Vector) -> Vector:
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def fraction(value: Fraction) -> str:
    return str(value.numerator) if value.denominator == 1 else f"{value.numerator}/{value.denominator}"


def analyze_pair(pair: dict[str, object]) -> dict[str, object]:
    circles = pair["circles"]
    edges = pair["edges"]
    if not isinstance(circles, list) or not isinstance(edges, list) or len(circles) != 2 or len(edges) != 2:
        raise ValueError("expected exactly two circles and edges")
    a, b = circles
    ea, eb = edges
    ca, cb = a["placement"], b["placement"]
    center, axis = vector(ca["point"]), vector(ca["axis"])
    radius = rational(a["radius"])
    frames = [(vector(c["placement"]["axis"]), vector(c["placement"]["reference"])) for c in circles]
    other_axis = vector(cb["axis"])
    checks = {
        "same_center": center == vector(cb["point"]),
        "same_positive_radius": radius > 0 and radius == rational(b["radius"]),
        "same_oriented_axis": cross(axis, other_axis) == (0, 0, 0) and dot(axis, other_axis) > 0,
        "nonzero_axis_and_reference": all(dot(n, n) > 0 and dot(x, x) > 0 for n, x in frames),
        "axis_reference_orthogonal": all(dot(n, x) == 0 for n, x in frames),
        "curve_links": [e["curve"] for e in edges] == [c["id"] for c in circles],
        "reversed_vertex_ids": ea["vertices"] == list(reversed(eb["vertices"])),
        "reversed_vertex_coordinates": ea["vertexPointsInch"] == list(reversed(eb["vertexPointsInch"])),
        "same_edge_and_loop_sense": ea["sameSense"] == eb["sameSense"] and ea["orientation"] == eb["orientation"] and ea["sameSense"] in (".T.", ".F.") and ea["orientation"] in (".T.", ".F."),
        "distinct_endpoints": vector(ea["vertexPointsInch"][0]) != vector(ea["vertexPointsInch"][1]),
    }
    residuals = []
    for edge, circle in zip(edges, circles, strict=True):
        c = vector(circle["placement"]["point"])
        n = vector(circle["placement"]["axis"])
        r = rational(circle["radius"])
        for point in edge["vertexPointsInch"]:
            delta = subtract(vector(point), c)
            residuals.append({"plane_dot_inch": fraction(dot(delta, n)),
                              "squared_radius_error_inch2": fraction(dot(delta, delta) - r * r),
                              "nonzero_radial": dot(delta, delta) != 0})
    checks["vertices_on_circle_exact"] = all(
        row["plane_dot_inch"] == "0" and row["squared_radius_error_inch2"] == "0" for row in residuals
    )
    plane_radial_projection = all(row["plane_dot_inch"] == "0" and row["nonzero_radial"] for row in residuals)
    first = subtract(vector(ea["vertexPointsInch"][0]), center)
    second = subtract(vector(ea["vertexPointsInch"][1]), center)
    distinct_projected = cross(first, second) != (0, 0, 0) or dot(first, second) < 0
    all_nonmembership_checks = all(value for name, value in checks.items() if name != "vertices_on_circle_exact")
    return {"loop": pair["loop"], "checks": checks, "vertex_residuals": residuals,
            "frame_squared_norms": [{"axis": fraction(dot(n, n)), "reference": fraction(dot(x, x)), "axis_dot_reference": fraction(dot(n, x))} for n, x in frames],
            "exact_conditions": all(checks.values()),
            "conditional_radial_projection": all_nonmembership_checks and plane_radial_projection and distinct_projected}


def report(data: bytes) -> dict[str, object]:
    traced = source_trace.main_trace(data)
    pairs = [analyze_pair(pair) for pair in traced["twoEdgeCirclePairs"]]
    counts = Counter(name for pair in pairs for name, ok in pair["checks"].items() if not ok)
    return {"source_sha256": source_trace.SHA256, "pair_count": len(pairs),
            "exact_condition_pairs": sum(p["exact_conditions"] for p in pairs),
            "conditional_radial_projection_pairs": sum(p["conditional_radial_projection"] for p in pairs),
            "failed_check_counts": dict(sorted(counts.items())), "pairs": pairs,
            "scope": "Exact input decimals; STEP direction ratios are normalized. Complementary arcs depend on directed single-interval edge-curve trim semantics. Radial projection changes source points and is not a tolerance or source-geometry proof."}


def main() -> None:
    source = Path(__file__).resolve().parents[2] / "public-cad/nist/nist_ftc_09_asme1_rd.stp"
    print(json.dumps(report(source_trace.read_source(source)), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
