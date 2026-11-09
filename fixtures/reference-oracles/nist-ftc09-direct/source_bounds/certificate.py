#!/usr/bin/env python3
"""Exact rational source observations; no imported-kernel geometry assumptions."""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from fractions import Fraction
from pathlib import Path
from typing import Any

TRACE_DIR = Path(__file__).resolve().parent.parent
REPO_ROOT = TRACE_DIR.parents[2]
DEFAULT_SOURCE = REPO_ROOT / 'fixtures/public-cad/nist/nist_ftc_09_asme1_rd.stp'
sys.path.insert(0, str(TRACE_DIR))
import circle_pairs as pairs  # noqa: E402
import trace as source_trace  # noqa: E402

INCH_MM = Fraction(127, 5)
TARGET_MM = Fraction(1, 100_000_000)
TARGET_RAD = Fraction(1, 10_000_000_000)
EXPECTED_RESIDUAL = Fraction(-547042907199, 25000000000000000000000000)


def radial_bound_mm(radius: Fraction, squared_distance: Fraction) -> Fraction:
    """Bound exact radial projection by |d²-r²|/r without square roots."""
    if radius <= 0 or squared_distance <= 0:
        raise ValueError('radius and squared distance must be positive')
    return abs(squared_distance - radius * radius) / radius * INCH_MM


def traversal_conditions(pair: dict[str, Any], analysis: dict[str, Any]) -> dict[str, bool]:
    """Check the specific directed traversal pattern in this frozen source."""
    edges = pair['edges']
    return {
        'both_edge_curves_forward_parameter': all(edge['sameSense'] == '.T.' for edge in edges),
        'both_oriented_edges_forward': all(edge['orientation'] == '.T.' for edge in edges),
        'face_bound_reverses_traversal': pair['boundOrientation'] == '.F.',
        'reverse_vertex_ids': analysis['checks']['reversed_vertex_ids'],
        'reverse_vertex_coordinates': analysis['checks']['reversed_vertex_coordinates'],
        'same_oriented_circle_axis': analysis['checks']['same_oriented_axis'],
        'same_circle_center': analysis['checks']['same_center'],
        'same_positive_circle_radius': analysis['checks']['same_positive_radius'],
        'edge_curve_links': analysis['checks']['curve_links'],
    }


def pair_report(pair: dict[str, Any]) -> dict[str, Any]:
    analysis = pairs.analyze_pair(pair)
    if not analysis['conditional_radial_projection']:
        raise ValueError(f"loop {pair['loop']}: conditional assumptions not met")
    traversal = traversal_conditions(pair, analysis)
    if not all(traversal.values()):
        raise ValueError(f"loop {pair['loop']}: directed traversal conditions not met")
    circles = pair['circles']
    refs = [pairs.vector(c['placement']['reference']) for c in circles]
    if pairs.cross(*refs) != (0, 0, 0) or pairs.dot(*refs) >= 0:
        raise ValueError(f"loop {pair['loop']}: reference directions are not antiparallel")
    center = pairs.vector(circles[0]['placement']['point'])
    radius = pairs.rational(circles[0]['radius'])
    points = [pairs.subtract(pairs.vector(p), center) for p in pair['edges'][0]['vertexPointsInch']]
    if pairs.cross(*points) != (0, 0, 0) or pairs.dot(*points) >= 0:
        raise ValueError(f"loop {pair['loop']}: endpoints are not antiparallel")
    bounds = [radial_bound_mm(radius, pairs.dot(v, v)) for v in points]
    if any(bound >= TARGET_MM for bound in bounds):
        raise ValueError(f"loop {pair['loop']}: radial displacement bound exceeded")
    residual = [pairs.dot(v, v) - radius * radius for v in points]
    exact_membership = analysis['checks']['vertices_on_circle_exact']
    proof_status = 'exact_complementary_locus_inference' if exact_membership else 'conditional_on_endpoint_reconciliation'
    return {'loop': pair['loop'], 'circleLocusProofStatus': proof_status,
            'traversalConditions': traversal,
            'assumedEntitySemantics': 'ISO edge is a connected non-self-intersecting open curve, implicitly vertex-trimmed; face-bound reversal changes orientation, not locus.',
            'radiusInch': pairs.fraction(radius),
            'endpointSquaredRadiusResidualInch2': [pairs.fraction(x) for x in residual],
            'radialProjectionUpperMm': [pairs.fraction(x) for x in bounds],
            'normalizedReferencePairAngularSeparationFromAntiparallelRad': '0',
            'projectedEndpointSeparationFromAntiparallelRad': '0',
            'axisReferenceDot': [frame['axis_dot_reference'] for frame in analysis['frame_squared_norms']],
            'directionSquaredNorms': analysis['frame_squared_norms'],
            'edgeFlags': [{'sameSense': e['sameSense'], 'orientation': e['orientation']} for e in pair['edges']],
            'boundOrientation': pair['boundOrientation'], 'checks': analysis['checks']}


def certificate(source: Path) -> dict[str, Any]:
    raw = source_trace.read_source(source)
    traced = source_trace.main_trace(raw)
    result = [pair_report(pair) for pair in traced['twoEdgeCirclePairs']]
    if len(result) != 29:
        raise ValueError('expected 29 traced circle pairs')
    nonzero = [row for row in result if any(v != '0' for v in row['endpointSquaredRadiusResidualInch2'])]
    if len(nonzero) != 1 or nonzero[0]['loop'] != 2794 or any(Fraction(v) != EXPECTED_RESIDUAL for v in nonzero[0]['endpointSquaredRadiusResidualInch2']):
        raise ValueError('unexpected exact residual census')
    exact = [row for row in result if row['circleLocusProofStatus'] == 'exact_complementary_locus_inference']
    conditional = [row for row in result if row['circleLocusProofStatus'] == 'conditional_on_endpoint_reconciliation']
    if len(exact) != 28 or len(conditional) != 1 or conditional[0]['loop'] != 2794:
        raise ValueError('unexpected circle locus proof census')
    maximum = max(Fraction(x) for row in result for x in row['radialProjectionUpperMm'])
    if not maximum < TARGET_MM or not Fraction(0) < TARGET_RAD:
        raise ValueError('bound comparison failed')
    return {'sourcePath': source.resolve().relative_to(REPO_ROOT).as_posix(), 'sourceSha256': hashlib.sha256(raw).hexdigest(),
            'certificateScriptSha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            'traceScriptSha256': hashlib.sha256((TRACE_DIR / 'trace.py').read_bytes()).hexdigest(),
            'circlePairsScriptSha256': hashlib.sha256((TRACE_DIR / 'circle_pairs.py').read_bytes()).hexdigest(),
            'pairCount': len(result), 'exactComplementaryCircleLocusCount': len(exact),
            'conditionalLocusLoops': [row['loop'] for row in conditional], 'nonzeroResidualLoop': 2794,
            'primarySemantics': {'topology': 'https://ap238.org/SMRL_v8_final/data/resource_docs/geometric_and_topological_representation/sys/5_schema.htm',
                                 'geometry': 'https://ap238.org/SMRL_v8_final/data/resource_docs/geometric_and_topological_representation/sys/4_schema.htm',
                                 'reference': 'ISO 10303-42:2021 sections 5.5.4–5.5.6, 5.5.9, 5.5.17, 4.5.14, 4.5.19, 4.5.26',
                                 'editionApplicability': 'Source declares 2015 AP203 MIM; exact applicability of the 2021 edition must be reviewed before formal qualification.'},
            'globalMaterialProved': False, 'globalTopologyProved': False, 'massPropertyAccuracyProved': False, 'trialToleranceRatified': False,
            'maxRadialProjectionUpperMm': pairs.fraction(maximum),
            'radialTargetMm': pairs.fraction(TARGET_MM), 'angularTargetRad': pairs.fraction(TARGET_RAD),
            'maxPairAxisDirectionErrorRad': '0', 'maxPairReferenceAntiparallelErrorRad': '0',
            'maxProjectedEndpointAntiparallelErrorRad': '0', 'pairs': result,
            'scope': 'Exact rational source decimals only. Bounds describe hypothetical radial projection of traced vertices, not source geometry error, CAD transfer error, or a certified mass-property oracle.',
            'unproved': ['The 28 exact-locus inferences depend on the cited STEP entity constraints and require verification that this standard edition applies to the declared AP203 MIM.',
                         'Loop 2794 endpoints are not exactly on either circle; source does not specify a reconciliation rule and no exact source full-circle locus is inferred.',
                         'Projection changes source vertices; source does not specify projection as its transfer or repair semantics.',
                         'No bound on kernel-evaluated points, transfer, or source measurements; no global coordinate, radius, depth, angle, topology or mass-property error bound.',
                         'Full analytic face domains, containment, all source loops and cone semantics remain outside this certificate.']}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument('source', type=Path, nargs='?', default=DEFAULT_SOURCE)
    args = parser.parse_args()
    print(json.dumps(certificate(args.source), indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
