#!/usr/bin/env python3
"""Independent, bounded check of captured rigid and driven-slider world frames."""
from __future__ import annotations

import hashlib
import json
import math
import sys
from pathlib import Path
from typing import Any

MAX_BYTES = 64 * 1024 * 1024
TOL = 1e-9
REPLAY_FACTOR = 128 * sys.float_info.epsilon


def number(value: Any) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError('expected finite number')
    return float(value)


def finite_tree(value: Any) -> None:
    if isinstance(value, float) and not math.isfinite(value):
        raise ValueError('nonfinite JSON number')
    if isinstance(value, dict):
        for child in value.values():
            finite_tree(child)
    elif isinstance(value, list):
        for child in value:
            finite_tree(child)


def vec(value: Any) -> tuple[float, float, float]:
    if not isinstance(value, dict) or set(value) != {'x', 'y', 'z'}:
        raise ValueError('expected vector')
    return tuple(number(value[k]) for k in ('x', 'y', 'z'))


def add(a: tuple[float, ...], b: tuple[float, ...]) -> tuple[float, ...]:
    return tuple(x + y for x, y in zip(a, b, strict=True))


def sub(a: tuple[float, ...], b: tuple[float, ...]) -> tuple[float, ...]:
    return tuple(x - y for x, y in zip(a, b, strict=True))


def dot(a: tuple[float, ...], b: tuple[float, ...]) -> float:
    return sum(x * y for x, y in zip(a, b, strict=True))


def cross(a: tuple[float, ...], b: tuple[float, ...]) -> tuple[float, float, float]:
    return (a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0])


def unit(v: tuple[float, ...]) -> tuple[float, ...]:
    length = math.sqrt(dot(v, v))
    if not math.isfinite(length) or length < 1e-12:
        raise ValueError('degenerate vector')
    return tuple(x / length for x in v)


def rotate(q: Any, v: tuple[float, ...]) -> tuple[float, ...]:
    if not isinstance(q, dict) or set(q) != {'w', 'x', 'y', 'z'}:
        raise ValueError('expected Hamilton quaternion')
    w = number(q['w'])
    u = tuple(number(q[k]) for k in ('x', 'y', 'z'))
    if abs(w*w + dot(u, u) - 1) > 1e-12:
        raise ValueError('non-unit quaternion')
    uv = cross(u, v)
    return add(add(v, tuple(2*w*x for x in uv)), tuple(2*x for x in cross(u, uv)))


def frame(origin: dict[str, Any], pose: dict[str, Any]) -> tuple[tuple[float, ...], list[tuple[float, ...]]]:
    frame_value = origin['frame']
    z = unit(vec(frame_value['z']))
    raw_x = unit(vec(frame_value['x']))
    x = unit(sub(raw_x, tuple(dot(raw_x, z) * t for t in z)))
    axes = [x, cross(z, x), z]
    rotation = pose['rotation']
    point = add(rotate(rotation, vec(frame_value['origin'])), vec(pose['translation']))
    return point, [rotate(rotation, axis) for axis in axes]


def rotation_error(a: list[tuple[float, ...]], b: list[tuple[float, ...]]) -> tuple[float, ...]:
    matrix = [[dot(a[i], b[j]) for j in range(3)] for i in range(3)]
    sine_vector = ((matrix[2][1]-matrix[1][2])/2, (matrix[0][2]-matrix[2][0])/2, (matrix[1][0]-matrix[0][1])/2)
    sine = math.sqrt(dot(sine_vector, sine_vector))
    cosine = max(-1.0, min(1.0, (sum(matrix[i][i] for i in range(3))-1)/2))
    if cosine < 0.5:
        raise ValueError('unsupported large relative rotation')
    scale = math.atan2(sine, cosine) / sine if sine else 1.0
    return tuple(scale*x for x in sine_vector)


def occurrences(rows: Any) -> dict[str, dict[str, Any]]:
    if not isinstance(rows, list) or len(rows) != 20:
        raise ValueError('expected exactly 20 occurrences')
    result = {row['id']: row for row in rows}
    if len(result) != 20 or sum(row['grounded'] is True for row in rows) != 1:
        raise ValueError('duplicate occurrence or invalid ground')
    if any(type(row['grounded']) is not bool for row in rows):
        raise ValueError('invalid ground flag')
    for row in rows:
        if row['parent'] is not None:
            raise ValueError('nested occurrences unsupported')
        vec(row['local']['translation'])
        rotate(row['local']['rotation'], (1., 0., 0.))
    return result


def validate_tree(joints: Any, bodies: dict[str, dict[str, Any]], drives: Any) -> tuple[str, float]:
    if not isinstance(joints, list) or len(joints) != 19 or not isinstance(drives, list) or len(drives) != 1:
        raise ValueError('expected 19 joints and one drive')
    slider = drives[0]['joint']
    drive = number(drives[0]['value'])
    graph = {key: [] for key in bodies}
    ids = set()
    for joint in joints:
        if joint['id'] in ids or joint['suppressed'] is not False or joint['kind'] not in ('rigid', 'slider'):
            raise ValueError('duplicate, suppressed, or unsupported joint')
        ids.add(joint['id'])
        a, b = joint['a']['occurrence'], joint['b']['occurrence']
        if a not in graph or b not in graph or a == b:
            raise ValueError('invalid joint endpoints')
        graph[a].append(b); graph[b].append(a)
        if joint['kind'] == 'slider':
            if joint['id'] != slider:
                raise ValueError('unexpected slider')
            limits = joint['limits']
            if not isinstance(limits, dict) or not number(limits['min']) <= drive <= number(limits['max']):
                raise ValueError('drive outside limits')
        elif joint['limits'] is not None:
            raise ValueError('rigid limits unsupported')
        number(joint['value'])
    if sum(j['kind'] == 'slider' for j in joints) != 1:
        raise ValueError('expected one slider')
    seen = set(); todo = [next(iter(graph))]
    while todo:
        node = todo.pop()
        if node not in seen:
            seen.add(node); todo.extend(graph[node])
    if len(seen) != 20:
        raise ValueError('joint graph is not a tree')
    return slider, drive


def sample(row: dict[str, Any]) -> dict[str, Any]:
    if row['status'] != 'published':
        raise ValueError('sample did not publish')
    before, after = row['input'], row['output']
    initial, final = occurrences(before['occurrences']), occurrences(after['occurrences'])
    if initial.keys() != final.keys():
        raise ValueError('occurrence identity changed')
    slider, drive = validate_tree(before['joints'], initial, before['drives'])
    if after['persistedGeneration'] <= before['persistedGeneration']:
        raise ValueError('generation did not advance')
    for id, body in initial.items():
        published = final[id]
        if {k: v for k, v in body.items() if k != 'local'} != {k: v for k, v in published.items() if k != 'local'}:
            raise ValueError('occurrence ownership changed')
        if body['grounded'] and body['local'] != published['local']:
            raise ValueError('ground pose changed')
    output_joints = {j['id']: j for j in after['joints']}
    if len(output_joints) != 19:
        raise ValueError('output joint count mismatch')
    values = []; limits = []; details = []; scale = 1.0
    for joint in before['joints']:
        current = output_joints[joint['id']]
        allowed = {'value'} if joint['id'] == slider else set()
        if {k: v for k, v in current.items() if k not in allowed} != {k: v for k, v in joint.items() if k not in allowed}:
            raise ValueError('authored joint changed')
        a, ax = frame(joint['a'], final[joint['a']['occurrence']]['local'])
        b, bx = frame(joint['b'], final[joint['b']['occurrence']]['local'])
        displacement = sub(b, a)
        rotation = rotation_error(ax, bx)
        scale = max(scale, *(abs(v) for v in (*a, *b)), *(abs(v) for side in (joint['a'], joint['b']) for v in vec(side['frame']['origin'])))
        if joint['kind'] == 'rigid':
            values.extend((*displacement, *rotation))
            details.append({'joint': joint['id'], 'kind': 'rigid',
                            'linearComponentsMm': displacement, 'angularComponentsRad': rotation})
        else:
            along = dot(displacement, ax[2])
            transverse = (dot(displacement, ax[0]), dot(displacement, ax[1]))
            values.extend((*transverse, *rotation, along - drive))
            lower, upper = number(joint['limits']['min']), number(joint['limits']['max'])
            limits.append(max(lower - along, along - upper, 0.0))
            details.append({'joint': joint['id'], 'kind': 'slider', 'actualTravelMm': along,
                            'requestedTravelMm': drive, 'limitMm': [lower, upper],
                            'transverseComponentsMm': transverse, 'angularComponentsRad': rotation,
                            'driveResidualMm': along - drive, 'limitViolationMm': limits[-1]})
            if current['id'] != slider or number(current['value']) != drive:
                raise ValueError('published drive value mismatch')
    computed = max([abs(v) for v in values] + limits)
    if not math.isfinite(computed) or not math.isfinite(scale):
        raise ValueError('nonfinite derived residual')
    reported = number(row['maxResidual'])
    guard = REPLAY_FACTOR * scale
    if abs(computed - reported) > guard:
        verdict = 'failed_report_mismatch'
    elif computed - guard > TOL:
        verdict = 'failed_contract'
    elif computed + guard > TOL:
        verdict = 'inconclusive_numeric_replay'
    elif row['remainingDof'] != 0:
        verdict = 'failed_dof'
    else:
        verdict = 'passed'
    return {'index': row['index'], 'status': verdict, 'computedMaxResidual': computed,
            'reportedMaxResidual': reported, 'replayGuard': guard,
            'maxLimitViolation': max(limits), 'structuralDof': 0, 'jointChecks': details}


def check(path: Path) -> dict[str, Any]:
    hasher = hashlib.sha256(); chunks = []; size = 0
    with path.open('rb') as stream:
        while chunk := stream.read(65536):
            size += len(chunk)
            if size > MAX_BYTES:
                raise ValueError('input exceeds byte limit')
            hasher.update(chunk); chunks.append(chunk)
    data = json.loads(b''.join(chunks), parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite JSON')))
    finite_tree(data)
    if data['instrumentationVersion'] != 1 or data['complete'] is not True:
        raise ValueError('unsupported or incomplete instrumentation')
    warmups, measured = data['warmupsRaw'], data['samplesRaw']
    if len(warmups) != 5 or len(measured) != 100:
        raise ValueError('expected 5 warmups and 100 measured samples')
    rows = [sample(row) for row in warmups + measured]
    if {row['index'] for row in rows} != set(range(105)):
        raise ValueError('duplicate sample index')
    return {'sourceSha256': hasher.hexdigest(), 'checks': rows,
            'scope': 'Independent binary64 replay of captured constraints; guard is a conservative comparison heuristic, not a certified roundoff bound or physical accuracy claim.'}


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit('usage: check.py REPORT.json')
    try:
        result = check(Path(sys.argv[1]))
        print(json.dumps(result, sort_keys=True, indent=2))
        if any(row['status'] != 'passed' for row in result['checks']):
            raise SystemExit(1)
    except (ValueError, KeyError, TypeError, OSError, OverflowError, AttributeError, IndexError, RecursionError) as error:
        raise SystemExit(f'checker refused input: {error}') from error


if __name__ == '__main__':
    main()
