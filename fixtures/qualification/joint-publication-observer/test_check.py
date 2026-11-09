from __future__ import annotations

import copy
import json
import math
import tempfile
import unittest
from pathlib import Path

import check


def point(x: float, y: float = 0., z: float = 0.) -> dict[str, float]:
    return {'x': x, 'y': y, 'z': z}


def example(scale: float = 1., offset: float = 0.) -> dict:
    bodies = []
    for index in range(20):
        p = point(offset + index * scale, offset / 2, -offset / 3)
        bodies.append({'id': str(index), 'parent': None, 'grounded': index == 0,
                       'local': {'translation': p, 'rotation': {'w': 1., 'x': 0., 'y': 0., 'z': 0.}}})
    joints = []
    for index in range(1, 20):
        parent = 0 if index == 1 else 1
        def origin(n: int) -> dict:
            p = bodies[n]['local']['translation']
            return {'occurrence': str(n), 'frame': {'origin': point(-p['x'], -p['y'], -p['z']),
                    'x': point(0., 1., 0.), 'z': point(1., 0., 0.)}}
        joints.append({'id': str(index), 'kind': 'slider' if index == 1 else 'rigid',
                       'a': origin(parent), 'b': origin(index), 'suppressed': False,
                       'limits': {'min': -2., 'max': 2.} if index == 1 else None, 'value': 0.})
    after = copy.deepcopy(bodies)
    for body in after[1:]:
        body['local']['translation']['x'] += .5
    after_joints = copy.deepcopy(joints)
    after_joints[0]['value'] = .5
    return {'index': 0, 'status': 'published', 'input': {'occurrences': bodies, 'joints': joints,
            'drives': [{'joint': '1', 'value': .5}], 'persistedGeneration': 1},
            'output': {'occurrences': after, 'joints': after_joints, 'persistedGeneration': 2},
            'maxResidual': 0., 'remainingDof': 0}


class ObserverTests(unittest.TestCase):
    def test_translated_and_scaled_trees(self) -> None:
        for scale, offset in [(1., 0.), (.001, 0.), (1000., 10000.)]:
            row = check.sample(example(scale, offset))
            self.assertEqual(row['structuralDof'], 0)
            # Large coordinates may legitimately require an inconclusive numeric guard.
            self.assertIn(row['status'], ('passed', 'inconclusive_numeric_replay'))

    def test_common_world_rotation(self) -> None:
        row = example(10., 20.)
        q = {'w': math.sqrt(.5), 'x': 0., 'y': 0., 'z': math.sqrt(.5)}
        for side in ('input', 'output'):
            for body in row[side]['occurrences']:
                p = check.rotate(q, check.vec(body['local']['translation']))
                body['local'] = {'translation': point(*p), 'rotation': q.copy()}
        self.assertEqual(check.sample(row)['status'], 'passed')

    def test_bad_pose_refused(self) -> None:
        row = example(); row['output']['occurrences'][3]['local']['translation']['x'] += .01
        self.assertNotEqual(check.sample(row)['status'], 'passed')

    def test_bad_rotation_refused(self) -> None:
        row = example(); q = row['output']['occurrences'][4]['local']['rotation']
        q.update({'w': math.cos(.01), 'z': math.sin(.01)})
        self.assertNotEqual(check.sample(row)['status'], 'passed')

    def test_bad_limit_refused(self) -> None:
        row = example(); row['output']['occurrences'][1]['local']['translation']['x'] += 3.
        result = check.sample(row)
        self.assertGreater(result['maxLimitViolation'], 0.)
        self.assertNotEqual(result['status'], 'passed')

    def test_false_report_refused(self) -> None:
        row = example(); row['maxResidual'] = .001
        self.assertEqual(check.sample(row)['status'], 'failed_report_mismatch')

    def test_borderline_is_inconclusive(self) -> None:
        row = example()
        position = row['output']['occurrences'][2]['local']['translation']
        previous = position['x']
        position['x'] += 1e-9
        row['maxResidual'] = position['x'] - previous
        self.assertEqual(check.sample(row)['status'], 'inconclusive_numeric_replay')

    def test_occurrence_ownership_mutations_refused(self) -> None:
        for field in ('def', 'name', 'unknownAuthored'):
            row = example()
            row['input']['occurrences'][1][field] = 'original'
            row['output']['occurrences'][1][field] = 'replacement'
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'ownership changed'):
                check.sample(row)

    def test_joint_ownership_mutations_refused(self) -> None:
        for index, field in ((2, 'value'), (2, 'name'), (0, 'name'), (0, 'unknownAuthored')):
            row = example()
            if field != 'value':
                row['input']['joints'][index][field] = 'original'
            row['output']['joints'][index][field] = 42 if field == 'value' else 'replacement'
            with self.subTest(index=index, field=field), self.assertRaisesRegex(ValueError, 'authored joint changed'):
                check.sample(row)

    def test_false_dof_refused(self) -> None:
        row = example(); row['remainingDof'] = 1
        self.assertEqual(check.sample(row)['status'], 'failed_dof')

    def test_unknown_joint_and_nonfinite_refused(self) -> None:
        row = example(); row['input']['joints'][2]['kind'] = 'revolute'
        with self.assertRaises(ValueError): check.sample(row)
        row = example(); row['output']['occurrences'][2]['local']['translation']['x'] = math.nan
        with self.assertRaises(ValueError): check.sample(row)

    def test_duplicate_and_ungrounded_refused(self) -> None:
        row = example(); row['input']['occurrences'][0]['grounded'] = False
        with self.assertRaises(ValueError): check.sample(row)
        row = example(); row['input']['joints'][3]['id'] = row['input']['joints'][2]['id']
        with self.assertRaises(ValueError): check.sample(row)

    def test_full_report_and_truncation_refusal(self) -> None:
        rows = [example() for _ in range(105)]
        for index, row in enumerate(rows):
            row['index'] = index
        payload = {'instrumentationVersion': 1, 'complete': True,
                   'warmupsRaw': rows[:5], 'samplesRaw': rows[5:]}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input.json'
            path.write_text(json.dumps(payload))
            self.assertEqual(len(check.check(path)['checks']), 105)
            payload['unused'] = float('inf')
            path.write_text(json.dumps(payload))
            with self.assertRaises(ValueError):
                check.check(path)
            del payload['unused']
            payload['samplesRaw'].pop()
            path.write_text(json.dumps(payload))
            with self.assertRaisesRegex(ValueError, '100 measured'):
                check.check(path)


if __name__ == '__main__':
    unittest.main()
