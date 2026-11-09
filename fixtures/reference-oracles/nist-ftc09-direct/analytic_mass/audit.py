#!/usr/bin/env python3
"""Exact source-derived cross-section census, not a whole-solid mass oracle."""
from __future__ import annotations

import hashlib
import json
import sys
from collections import Counter
from fractions import Fraction
from pathlib import Path
from typing import Any

TRACE = Path(__file__).resolve().parent.parent
ROOT = TRACE.parents[2]
sys.path.insert(0, str(TRACE))
import circle_pairs  # noqa: E402
import trace  # noqa: E402

INCH_MM = Fraction(127, 5)
SOURCE = ROOT / 'fixtures/public-cad/nist/nist_ftc_09_asme1_rd.stp'


def fmt(value: Fraction) -> str:
    return str(value)


def xy_key(circle: dict[str, Any]) -> tuple[Fraction, Fraction]:
    point = circle['placement']['point']
    return Fraction(point[0]), Fraction(point[2])


def circles(records: dict[int, str], face: dict[str, Any]) -> dict[tuple[Fraction, Fraction], dict[str, Any]]:
    result: dict[tuple[Fraction, Fraction], dict[str, Any]] = {}
    for bound in face['bounds']:
        edges = bound['edges']
        if len(edges) != 2 or any(trace.entity(records, edge['curve']).name != 'CIRCLE' for edge in edges):
            continue
        pair = {'loop': bound['loop'], 'edges': edges,
                'circles': [trace.circle(records, edge['curve']) for edge in edges]}
        check = circle_pairs.analyze_pair(pair)
        if not check['conditional_radial_projection']:
            raise ValueError(f"unsupported circle pair at loop {bound['loop']}")
        circle = pair['circles'][0]
        plane_y = Fraction(face['placement']['point'][1])
        for member in pair['circles']:
            point = member['placement']['point']
            axis = member['placement']['axis']
            if Fraction(point[1]) != plane_y or axis[0] != 0 or axis[1] == 0 or axis[2] != 0:
                raise ValueError(f"unsupported circle plane at loop {bound['loop']}")
        center = xy_key(circle)
        if center in result:
            raise ValueError(f'duplicate circle center {center}')
        result[center] = {'loop': bound['loop'], 'radius': Fraction(circle['radius']),
                          'exact_endpoint_membership': check['checks']['vertices_on_circle_exact']}
    return result


def match_sections(bottom: dict[tuple[Fraction, Fraction], dict[str, Any]],
                   top: dict[tuple[Fraction, Fraction], dict[str, Any]]) -> list[dict[str, Any]]:
    if bottom.keys() != top.keys():
        raise ValueError('circular opening centers differ between top and bottom')
    rows = []
    for center in sorted(bottom):
        b, t = bottom[center], top[center]
        if t['radius'] < b['radius']:
            raise ValueError('unsupported top radius smaller than bottom radius')
        rows.append({'center_inch': [fmt(c) for c in center], 'bottom_loop': b['loop'],
                     'top_loop': t['loop'], 'bottom_radius_inch': fmt(b['radius']),
                     'top_radius_inch': fmt(t['radius']),
                     'bottom_exact_endpoints': b['exact_endpoint_membership'],
                     'top_exact_endpoints': t['exact_endpoint_membership']})
    return rows


def cone_groups(cones: list[dict[str, Any]]) -> dict[tuple[Fraction, Fraction], list[dict[str, Any]]]:
    groups: dict[tuple[Fraction, Fraction], list[dict[str, Any]]] = {}
    for cone in cones:
        groups.setdefault(xy_key(cone), []).append(cone)
    if len(groups) != 4 or any(len(group) != 2 for group in groups.values()):
        raise ValueError('unsupported cone patch census')
    for group in groups.values():
        if any(c['semiangleDegrees'] != 45 or c['placement']['axis'] != (0, 1, 0) for c in group):
            raise ValueError('unsupported cone axis or source angle')
        if group[0]['placement'] != group[1]['placement'] or group[0]['radiusInch'] != group[1]['radiusInch']:
            raise ValueError('inconsistent cone patch definitions')
    return groups


def frustum_excess(r: Fraction, upper: Fraction) -> Fraction:
    """Coefficient of pi for a 45-degree frustum over its cylindrical core."""
    if not 0 < r < upper:
        raise ValueError('invalid frustum radii')
    height = upper - r
    return height * (upper * upper + upper * r - 2 * r * r) / 3


def audit(raw: bytes) -> dict[str, Any]:
    data = trace.main_trace(raw)
    records = trace.parse_step(raw)
    lower, upper, *floors = data['faces']
    y0 = Fraction(lower['placement']['point'][1])
    y1 = Fraction(upper['placement']['point'][1])
    floor_levels = [Fraction(face['placement']['point'][1]) for face in floors]
    thickness = y1 - y0
    if thickness <= 0 or len(floor_levels) != 2 or len(set(floor_levels)) != 1 or not y0 < floor_levels[0] < y1:
        raise ValueError('unsupported face level ordering')
    bottom, top = circles(records, lower), circles(records, upper)
    matched = match_sections(bottom, top)
    if len(matched) != 29:
        raise ValueError('unexpected circular opening census')
    changed = {center for center in bottom if bottom[center]['radius'] != top[center]['radius']}
    cones = cone_groups(data['cones'])
    if changed != cones.keys():
        raise ValueError('cone centers do not match changed circular openings')
    for center, group in cones.items():
        lower_radius, upper_radius = bottom[center]['radius'], top[center]['radius']
        reference = group[0]
        reference_y = Fraction(reference['placement']['point'][1])
        if (not 0 < upper_radius - lower_radius <= thickness
                or not y1 - (upper_radius - lower_radius) <= reference_y <= y1
                or Fraction(reference['radiusInch']) + y1 - reference_y != upper_radius):
            raise ValueError('source cone does not match idealized 45-degree cross-section')
    return summarize(data, lower, upper, floors, bottom, top, cones, matched, thickness, y1 - floor_levels[0])


def summarize(data: dict[str, Any], lower: dict[str, Any], upper: dict[str, Any], floors: list[dict[str, Any]],
              bottom: dict[tuple[Fraction, Fraction], dict[str, Any]], top: dict[tuple[Fraction, Fraction], dict[str, Any]],
              cones: dict[tuple[Fraction, Fraction], list[dict[str, Any]]], matched: list[dict[str, Any]],
              thickness: Fraction, depth: Fraction) -> dict[str, Any]:
    circle_area = sum((row['radius'] ** 2 for row in bottom.values()), Fraction())
    excess = sum((frustum_excess(bottom[c]['radius'], top[c]['radius']) for c in cones), Fraction())
    return {
        'source_sha256': data['sourceSha256'], 'audit_script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'trace_script_sha256': hashlib.sha256((TRACE / 'trace.py').read_bytes()).hexdigest(),
        'circle_pairs_script_sha256': hashlib.sha256((TRACE / 'circle_pairs.py').read_bytes()).hexdigest(),
        'thickness_mm': fmt(thickness * INCH_MM), 'blind_pocket_depth_mm': fmt(depth * INCH_MM),
        'source_faces': [lower['id'], upper['id'], *(face['id'] for face in floors)],
        'circle_count': len(matched), 'matched_sections': matched,
        'bottom_radius_counts_inch': dict(sorted(Counter(fmt(row['radius']) for row in bottom.values()).items())),
        'other_bottom_bound_edge_counts': [len(b['edges']) for b in lower['bounds'] if len(b['edges']) != 2],
        'blind_floor_edge_counts': [len(face['bounds'][0]['edges']) for face in floors],
        'cone_groups': [{'center_inch': [fmt(v) for v in center], 'surface_ids': [c['id'] for c in group],
                         'source_reference_radius_inch': fmt(Fraction(group[0]['radiusInch'])),
                         'source_reference_y_inch': fmt(Fraction(group[0]['placement']['point'][1])),
                         'source_angle_degrees': '45'} for center, group in sorted(cones.items())],
        'conditional_circle_area_pi_coefficient_mm2': fmt(circle_area * INCH_MM ** 2),
        'conditional_through_cylinder_volume_pi_coefficient_mm3': fmt(circle_area * thickness * INCH_MM ** 3),
        'conditional_four_45deg_frustum_excess_pi_coefficient_mm3': fmt(excess * INCH_MM ** 3),
        'mass_or_area_of_solid': None,
        'assumptions_and_limits': [
            'Matching source cross sections do not prove continuity, containment, disjointness, or through-holes.',
            'Circle locus inference assumes directed single-interval edge semantics; loops 2794 and 3127 require endpoint reconciliation.',
            'Circle-area and cylinder-volume coefficients assume complete disjoint through-circles; they are not whole-solid measures.',
            'Frustum excess assumes four complete 45-degree conical frusta and height equal to radius difference; source degree conversion is not exact pi/180 and cone trim domains are unproved.',
            'Outer boundary, slots, central complex opening, blind-pocket areas, cone trimming and global topology remain unresolved.',
            'No pi approximation, total volume, total surface area, transfer error, measurement uncertainty, or trial-tolerance qualification is supplied.'
        ]}


def main() -> None:
    print(json.dumps(audit(trace.read_source(SOURCE)), sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
