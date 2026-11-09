# Joint publication observer

`check.py` independently replays world joint frames captured by version 1 of the application joint publication report. It accepts only the 20-occurrence connected tree with one grounded occurrence, 18 rigid joints, and one driven slider. It verifies input and output ownership, unchanged joint definitions and ground, the requested drive, limits, structural zero degrees of freedom, and per-component world-frame constraint residuals. The checker uses Hamilton `w,x,y,z` local-to-world occurrence quaternions. It reports each joint's computed components and compares the largest with the application's reported residual.

```sh
python3 fixtures/qualification/joint-publication-observer/check.py /path/to/report.json > /path/to/observer-result.json
python3 -m unittest discover -s fixtures/qualification/joint-publication-observer -p 'test_*.py' -v
```

Input is capped at 64 MiB and hashed while reading. Incomplete reports, unsupported structures, nonfinite values, and failed or cancelled samples are refused. A non-passing or inconclusive observation exits nonzero. Tests use synthetic trees and deliberately incorrect poses, rotation, limits, reports, and degrees of freedom; they are not captured benchmark goldens.

The `1e-9` per-component comparison reflects the existing solver contract, with the original units of each linear or angular component. A `128 × binary64 epsilon × coordinate scale` guard is used only for comparing independently computed and reported numbers and identifying borderline cases. It is a heuristic, not a certified rounding bound; borderline observations remain inconclusive. It does not establish physical accuracy or justify changing solver limits. Tree counting establishes degrees of freedom only for this supported fixture, not general constraint rank. The benchmark's global pose goals are separate and remain unchanged.
