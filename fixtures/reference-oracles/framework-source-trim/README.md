# Framework face 66 endpoint-trim diagnostic

This optional source-admission diagnostic examines the endpoint assumptions used
for the first experimental negative-side pipe. It does not modify the pinned STEP,
extractor, oracle, or model geometry. It is not a source-equivalence,
material-ownership, manufacturing-accuracy, or complete Framework qualification.

The input is Framework Computer Inc.'s `ExpansionCard_ThreadedInsert.stp`, SHA-256
`6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09`. Its source
attribution and CC BY 4.0 notice are in `fixtures/public-cad/framework`. The existing
STEPCAF helper resolves the product `FW_EXP_1USBC_FRAME_CLIP_BC_229_` and applies
`(x,y,z) → (x,-z,y)`. The diagnostic checks the source and helper scripts by SHA-256 before and after reading. Face 66 is a zero-based
`TopExp_Explorer` index after transfer, not a stable STEP entity identifier.

Run from the repository root in a virtual environment containing the recorded
x86_64 OCP 8.0.1.1.0 wheel (SHA-256
`24d2beabb3ad0fa9366a3532a08beecf210280c5f39fe1ef80a9e0f4f3e5b800`). The script
checks the imported version and process architecture, but cannot attest the wheel
bytes after installation. Unsupported environments, changed inputs and unexpected
face or pcurve topology are refused. The JSON output is exclusively created: an existing file, symlink or hard link is refused. Choose a new path outside the repository. A failed run may leave an empty output file; success requires exit zero and valid JSON.

```sh
python fixtures/reference-oracles/framework-source-trim/audit.py /tmp/framework-face66-trim.json
python -m unittest discover -s fixtures/reference-oracles/framework-source-trim -v
```

In transferred wire traversal, edges 0 and 2 have degree-one, two-pole pcurves
whose v-coordinates are respectively above 0 and above 1 throughout their checked
clamped parameter intervals. Thus `v=0` is not edge 0 and `v=1` is not edge 2.
This does not establish that those edges are intended opposite endpoints. The three classifier queries at each v are supplemental observations,
not a full-domain proof. The source boundary circle is described as transferred. The transfer and stored
source tolerances do not establish which coordinates represent intended physical
material, nor do they justify a tolerance change or a geometry correction.
