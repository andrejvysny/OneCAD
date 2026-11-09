"""Fail-closed tests that do not invoke a CAD kernel or read the STEP model."""
import hashlib
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

import audit


def wire(v0: tuple[float, float], v1: tuple[float, float]) -> list[dict]:
    def edge(values: tuple[float, float]) -> dict:
        return {"pcurve": {"degree": 1, "rational": False, "periodic": False,
            "poles": [[0.2, values[0]], [0.8, values[1]]],
            "knots": [{"value": 0.0, "multiplicity": 2}, {"value": 1.0, "multiplicity": 2}]},
            "pcurveRange": [0.0, 1.0]}
    first = edge(v0)
    first["curve3dKind"] = "GeomAbs_Circle"
    edges = [first, {}, edge(v1), {}]
    return [{"outer": True, "edges": edges}]


class Refusals(unittest.TestCase):
    def test_changed_bytes_refused(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "input"
            path.write_bytes(b"original")
            expected = hashlib.sha256(b"original").hexdigest()
            audit.verify(path, expected)
            path.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                audit.verify(path, expected)

    def test_existing_output_is_never_overwritten(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "output.json"
            path.write_text("preserve", encoding="utf-8")
            with patch.object(audit.sys, "argv", ["audit.py", str(path)]), patch.object(audit, "run") as run:
                with self.assertRaises(FileExistsError):
                    audit.main()
                run.assert_not_called()
            self.assertEqual(path.read_text(encoding="utf-8"), "preserve")

    def test_new_output_written_exclusively(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "output.json"
            with patch.object(audit.sys, "argv", ["audit.py", str(path)]), patch.object(audit, "run", return_value={"ok": True}):
                audit.main()
            self.assertEqual(path.read_text(encoding="utf-8").strip(), '{\n  "ok": true\n}')

    def test_supported_endpoint_relations(self) -> None:
        result = audit.validate_endpoint_hypothesis(wire((0.00003, 0.00006), (1.00001, 1.00002)))
        self.assertFalse(result["vZeroIsEdge0"])
        self.assertFalse(result["vOneIsEdge2"])

    def test_zero_boundary_is_not_reported_as_outside(self) -> None:
        with self.assertRaises(ValueError):
            audit.validate_endpoint_hypothesis(wire((0.0, 0.00006), (1.00001, 1.00002)))

    def test_one_boundary_is_not_reported_as_interior(self) -> None:
        with self.assertRaises(ValueError):
            audit.validate_endpoint_hypothesis(wire((0.00003, 0.00006), (1.0, 1.00002)))

    def test_extrapolated_range_refused(self) -> None:
        data = wire((0.00003, 0.00006), (1.00001, 1.00002))
        data[0]["edges"][0]["pcurveRange"] = [-0.001, 1.0]
        with self.assertRaises(ValueError):
            audit.validate_endpoint_hypothesis(data)

    def test_invalid_knots_and_poles_refused(self) -> None:
        for mutation in ("nan", "multiplicity", "reversed", "missing"):
            data = wire((0.00003, 0.00006), (1.00001, 1.00002))
            edge = data[0]["edges"][2]
            if mutation == "nan":
                edge["pcurve"]["poles"][0][0] = float("nan")
            elif mutation == "multiplicity":
                edge["pcurve"]["knots"][0]["multiplicity"] = 1
            elif mutation == "reversed":
                edge["pcurveRange"] = [1.0, 0.0]
            else:
                del edge["pcurve"]["knots"]
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                audit.validate_endpoint_hypothesis(data)

    def test_wrong_boundary_type_or_wire_graph_refused(self) -> None:
        data = wire((0.00003, 0.00006), (1.00001, 1.00002))
        data[0]["edges"][0]["curve3dKind"] = "GeomAbs_Line"
        with self.assertRaises(ValueError):
            audit.validate_endpoint_hypothesis(data)
        with self.assertRaises(ValueError):
            audit.validate_endpoint_hypothesis([])


if __name__ == "__main__":
    unittest.main()
