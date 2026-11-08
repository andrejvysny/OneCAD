"""Regression checks for exact circular edge-pair conditions."""
from __future__ import annotations

import copy
import unittest
from pathlib import Path

import circle_pairs
import trace


def valid_pair() -> dict[str, object]:
    circle = {"id": 1, "placement": {"point": ["0", "0", "0"], "axis": ["0", "0", "1"], "reference": ["1", "0", "0"]}, "radius": "1"}
    other = copy.deepcopy(circle)
    other["id"] = 2
    first = {"curve": 1, "vertices": [1, 2], "vertexPointsInch": [["1", "0", "0"], ["0", "1", "0"]], "sameSense": ".T.", "orientation": ".T."}
    second = {"curve": 2, "vertices": [2, 1], "vertexPointsInch": list(reversed(first["vertexPointsInch"])), "sameSense": ".T.", "orientation": ".T."}
    return {"loop": 1, "circles": [circle, other], "edges": [first, second]}


class CirclePairTests(unittest.TestCase):
    def test_non_diametric_complementary_arcs(self) -> None:
        result = circle_pairs.analyze_pair(valid_pair())
        self.assertTrue(result["exact_conditions"])
        self.assertTrue(result["conditional_radial_projection"])

    def test_wrong_axis(self) -> None:
        pair = valid_pair()
        pair["circles"][1]["placement"]["axis"] = ["0", "0", "-1"]
        self.assertFalse(circle_pairs.analyze_pair(pair)["checks"]["same_oriented_axis"])

    def test_positive_scaled_direction_ratios(self) -> None:
        pair = valid_pair()
        pair["circles"][0]["placement"]["axis"] = ["0", "0", "2"]
        pair["circles"][1]["placement"]["axis"] = ["0", "0", "3"]
        pair["circles"][1]["placement"]["reference"] = ["5", "0", "0"]
        result = circle_pairs.analyze_pair(pair)
        self.assertTrue(result["exact_conditions"])
        self.assertEqual(result["frame_squared_norms"][0]["axis"], "4")

    def test_reversed_relative_orientation(self) -> None:
        pair = valid_pair()
        pair["edges"][1]["sameSense"] = ".F."
        self.assertFalse(circle_pairs.analyze_pair(pair)["checks"]["same_edge_and_loop_sense"])

    def test_coincident_endpoints(self) -> None:
        pair = valid_pair()
        a, b = pair["edges"]
        a["vertexPointsInch"][1] = a["vertexPointsInch"][0]
        b["vertexPointsInch"] = list(reversed(a["vertexPointsInch"]))
        self.assertFalse(circle_pairs.analyze_pair(pair)["checks"]["distinct_endpoints"])

    def test_wrong_radius(self) -> None:
        pair = valid_pair()
        for circle in pair["circles"]:
            circle["radius"] = "2"
        result = circle_pairs.analyze_pair(pair)
        self.assertFalse(result["checks"]["vertices_on_circle_exact"])
        self.assertTrue(result["conditional_radial_projection"])

    def test_offcurve_vertex(self) -> None:
        pair = valid_pair()
        a, b = pair["edges"]
        a["vertexPointsInch"][0] = ["1", "0", "1"]
        b["vertexPointsInch"] = list(reversed(a["vertexPointsInch"]))
        result = circle_pairs.analyze_pair(pair)
        self.assertFalse(result["checks"]["vertices_on_circle_exact"])
        self.assertFalse(result["conditional_radial_projection"])

    def test_radial_projection_must_not_merge_endpoints(self) -> None:
        pair = valid_pair()
        a, b = pair["edges"]
        a["vertexPointsInch"][1] = ["2", "0", "0"]
        b["vertexPointsInch"] = list(reversed(a["vertexPointsInch"]))
        self.assertFalse(circle_pairs.analyze_pair(pair)["conditional_radial_projection"])

    def test_boolean_coordinate_and_radius_are_refused(self) -> None:
        pair = valid_pair()
        pair["circles"][0]["placement"]["point"][0] = True
        with self.assertRaisesRegex(ValueError, "boolean"):
            circle_pairs.analyze_pair(pair)
        pair = valid_pair()
        pair["circles"][0]["radius"] = True
        with self.assertRaisesRegex(ValueError, "boolean"):
            circle_pairs.analyze_pair(pair)

    def test_pinned_source_diagnostics(self) -> None:
        path = Path(__file__).resolve().parents[2] / "public-cad/nist/nist_ftc_09_asme1_rd.stp"
        result = circle_pairs.report(trace.read_source(path))
        self.assertEqual(result["pair_count"], 29)
        self.assertEqual(result["exact_condition_pairs"], 28)
        self.assertEqual(result["conditional_radial_projection_pairs"], 29)
        failure = next(pair for pair in result["pairs"] if not pair["exact_conditions"])
        self.assertEqual(failure["loop"], 2794)
        self.assertEqual(failure["vertex_residuals"][0]["squared_radius_error_inch2"], "-547042907199/25000000000000000000000000")


if __name__ == "__main__":
    unittest.main()
