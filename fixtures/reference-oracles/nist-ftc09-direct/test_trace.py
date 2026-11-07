"""Failure-oriented checks for the bounded FTC09 source tracer."""
from __future__ import annotations

import copy
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import trace

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parents[1] / "public-cad/nist/nist_ftc_09_asme1_rd.stp"


class DirectTraceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.source = SOURCE.read_bytes()
        cls.records = trace.parse_step(cls.source)

    def test_source_hash_and_reproducible_trace(self) -> None:
        self.assertEqual(hashlib.sha256(self.source).hexdigest(), trace.SHA256)
        actual = json.loads(json.dumps(trace.main_trace(self.source), default=trace.encode))
        self.assertEqual(actual, json.loads((HERE / "trace.json").read_text()))

    def test_wrong_source_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            trace.main_trace(self.source + b" ")

    def test_duplicate_or_malformed_records_rejected(self) -> None:
        for data in (b"DATA;#1=LINE();#1=LINE();ENDSEC;ENDSEC;",
                     b"DATA;#1=LINE();oops;ENDSEC;ENDSEC;"):
            with self.subTest(data=data), self.assertRaises(ValueError):
                trace.parse_step(data, require_hash=False)

    def test_missing_graph_reference_rejected(self) -> None:
        records = {5384: "MANIFOLD_SOLID_BREP('',#7)"}
        with self.assertRaisesRegex(ValueError, "missing entity"):
            trace.graph(records, 5384)

    def test_unsupported_reachable_entity_rejected(self) -> None:
        records = copy.copy(self.records)
        records[5384] = "UNSUPPORTED('',#5383)"
        with self.assertRaisesRegex(ValueError, "unsupported reachable"):
            trace.graph(records, 5384)
        with self.assertRaisesRegex(ValueError, "expected"):
            trace.entity(records, 5384, "MANIFOLD_SOLID_BREP")

    def test_circle_geometry_mismatch_refused(self) -> None:
        records = copy.copy(self.records)
        records[36] = records[36].replace("1.17E-1", "1.18E-1")
        with self.assertRaisesRegex(ValueError, "circle pair geometry mismatch"):
            trace.verify_pairs(records, trace.face(records, 2920))

    def test_circle_pair_vertex_mismatch_refused(self) -> None:
        records = copy.copy(self.records)
        records[2674] = records[2674].replace("#2024,#2023", "#2023,#2024")
        with self.assertRaisesRegex(ValueError, "reversed vertices"):
            trace.verify_pairs(records, trace.face(records, 2920))

    def test_unsupported_edge_orientation_refused(self) -> None:
        records = copy.copy(self.records)
        records[2673] = records[2673].replace(".T.", ".U.")
        with self.assertRaisesRegex(ValueError, "unsupported oriented edge"):
            trace.face(records, 2920)

    def test_token_and_size_limits(self) -> None:
        with self.assertRaisesRegex(ValueError, "unsupported STEP token"):
            trace.tokens("LINE(@)")
        with self.assertRaisesRegex(ValueError, "byte limit"):
            trace.parse_step(b"x" * (trace.MAX_BYTES + 1), require_hash=False)

    def test_oversized_source_file_refused(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "oversized.stp"
            source.write_bytes(b"x" * (trace.MAX_BYTES + 1))
            with self.assertRaisesRegex(ValueError, "source byte limit"):
                trace.read_source(source)


if __name__ == "__main__":
    unittest.main()
