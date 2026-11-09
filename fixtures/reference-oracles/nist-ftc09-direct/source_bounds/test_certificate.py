import copy
import hashlib
import tempfile
import unittest
from pathlib import Path
from fractions import Fraction as F
import certificate as c


class BoundsTests(unittest.TestCase):
    def test_inward_projection_strict_bound(self) -> None:
        r = F(5, 64)
        q = F(-547042907199, 25000000000000000000000000)
        b = c.radial_bound_mm(r, r*r+q)
        self.assertEqual(b, F(69474449214273, 9765625000000000000000000))
        self.assertLess(b, F(1, 100000000))
        # With inward perturbation d < r, r+d > r and |r-d| < |r²-d²|/r.
        self.assertLess(r*r+q, r*r)
        self.assertGreater(r*r+q, 0)

    def test_outward_projection_bound_and_degenerate_refusal(self) -> None:
        self.assertEqual(c.radial_bound_mm(F(1), F(4)), F(381, 5))
        for r, d in [(F(0), F(1)), (F(1), F(0)), (F(-1), F(1))]:
            with self.assertRaises(ValueError):
                c.radial_bound_mm(r, d)

    def test_malformed_loop_rejected(self) -> None:
        with self.assertRaises((ValueError, KeyError)):
            c.pair_report({'loop': 1, 'circles': [], 'edges': []})


class TraversalTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.source = c.DEFAULT_SOURCE
        cls.traced = c.source_trace.main_trace(c.source_trace.read_source(cls.source))
        cls.pair = next(p for p in cls.traced['twoEdgeCirclePairs'] if p['loop'] == 2676)

    def test_exact_and_conditional_source_census(self) -> None:
        result = c.certificate(self.source)
        self.assertEqual(result['exactComplementaryCircleLocusCount'], 28)
        self.assertEqual(result['conditionalLocusLoops'], [2794])
        row = next(p for p in result['pairs'] if p['loop'] == 2794)
        self.assertEqual(row['circleLocusProofStatus'], 'conditional_on_endpoint_reconciliation')
        self.assertEqual(c.pair_report(self.pair)['circleLocusProofStatus'], 'exact_complementary_locus_inference')

    def test_generator_source_hash_is_recorded(self) -> None:
        result = c.certificate(self.source)
        self.assertEqual(result['certificateScriptSha256'], hashlib.sha256(Path(c.__file__).read_bytes()).hexdigest())

    def test_non_antipodal_endpoints_refused(self) -> None:
        p = copy.deepcopy(self.pair)
        old = p['edges'][0]['vertexPointsInch'][0]
        changed = (old[0] + 1, old[1], old[2])
        p['edges'][0]['vertexPointsInch'][0] = changed
        p['edges'][1]['vertexPointsInch'][1] = changed
        with self.assertRaises(ValueError):
            c.pair_report(p)

    def test_reversed_same_sense_refused_even_when_both_match(self) -> None:
        p = copy.deepcopy(self.pair)
        for edge in p['edges']:
            edge['sameSense'] = '.F.'
        with self.assertRaises(ValueError):
            c.pair_report(p)

    def test_reversed_oriented_edges_refused_even_when_both_match(self) -> None:
        p = copy.deepcopy(self.pair)
        for edge in p['edges']:
            edge['orientation'] = '.F.'
        with self.assertRaises(ValueError):
            c.pair_report(p)

    def test_different_bound_orientation_refused(self) -> None:
        p = copy.deepcopy(self.pair)
        p['boundOrientation'] = '.T.'
        with self.assertRaises(ValueError):
            c.pair_report(p)

    def test_opposite_axis_refused(self) -> None:
        p = copy.deepcopy(self.pair)
        p['circles'][1]['placement']['axis'] = [0, 1, 0]
        with self.assertRaises(ValueError):
            c.pair_report(p)

    def test_changed_source_hash_refused(self) -> None:
        altered = self.source.read_bytes() + b' '
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'modified.stp'
            path.write_bytes(altered)
            with self.assertRaisesRegex(ValueError, 'SHA-256'):
                c.certificate(path)


if __name__ == '__main__':
    unittest.main()
