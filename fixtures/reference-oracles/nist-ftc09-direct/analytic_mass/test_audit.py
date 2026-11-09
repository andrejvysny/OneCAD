"""Checks exact arithmetic and fail-closed source observations."""
from __future__ import annotations

import unittest
from fractions import Fraction

import audit


class AnalyticAuditTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.report = audit.audit(audit.trace.read_source(audit.SOURCE))

    def test_pinned_levels_and_opening_census(self) -> None:
        result = self.report
        self.assertEqual(result['source_sha256'], audit.trace.SHA256)
        self.assertEqual(Fraction(result['thickness_mm']), Fraction(37973, 12500))
        self.assertEqual(Fraction(result['blind_pocket_depth_mm']), Fraction(127, 100))
        self.assertEqual(result['circle_count'], 29)
        self.assertEqual(result['other_bottom_bound_edge_counts'], [24, 4, 4, 4, 4, 16])
        self.assertEqual(result['blind_floor_edge_counts'], [20, 12])
        self.assertEqual(len(result['cone_groups']), 4)
        self.assertTrue(all(len(group['surface_ids']) == 2 for group in result['cone_groups']))

    def test_unreconciled_source_pair_remains_visible(self) -> None:
        rows = [row for row in self.report['matched_sections'] if not row['bottom_exact_endpoints'] or not row['top_exact_endpoints']]
        self.assertEqual([(row['bottom_loop'], row['top_loop']) for row in rows], [(2794, 3127)])
        self.assertIsNone(self.report['mass_or_area_of_solid'])

    def test_exact_conditional_coefficients(self) -> None:
        self.assertEqual(Fraction(self.report['conditional_circle_area_pi_coefficient_mm2']), Fraction(544627926871, 1600000000))
        self.assertEqual(Fraction(self.report['conditional_through_cylinder_volume_pi_coefficient_mm3']), Fraction(20681156267072483, 20000000000000))
        self.assertEqual(Fraction(self.report['conditional_four_45deg_frustum_excess_pi_coefficient_mm3']), Fraction(2341301769, 312500000))
        self.assertEqual(audit.frustum_excess(Fraction(1), Fraction(2)), Fraction(4, 3))
        with self.assertRaises(ValueError):
            audit.frustum_excess(Fraction(2), Fraction(1))

    def test_mismatched_section_centers_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, 'centers differ'):
            audit.match_sections({(Fraction(0), Fraction(0)): {}}, {})

    def test_mismatched_radius_direction_refused(self) -> None:
        point = (Fraction(0), Fraction(0))
        with self.assertRaisesRegex(ValueError, 'top radius smaller'):
            audit.match_sections({point: {'radius': Fraction(2)}}, {point: {'radius': Fraction(1)}})

    def test_bad_cone_census_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, 'cone patch census'):
            audit.cone_groups([])

    def test_wrong_source_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, 'SHA-256'):
            audit.audit(b'not the pinned STEP')


if __name__ == '__main__':
    unittest.main()
