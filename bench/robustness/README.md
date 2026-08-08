# OneCAD kernel robustness benchmark

KBR-0 is the first end-to-end robustness benchmark for OneCAD's OCCT modeling
kernel. It is intentionally separate from the product worker protocol and from
the frozen legacy `corpus/` oracle.

## Architecture

`onecad-kernelbench` is the public Rust supervisor. It validates cases, expands
deterministic suite variants, launches one disposable child process per
backend, enforces resource bounds, writes compact JSONL records, and produces
differential summaries. Child completion order never changes result order.
Linux/Unix applies `RLIMIT_AS` before `exec`. macOS rejects address-space
rlimits and maps a multi-terabyte shared region into normal processes, so the
parent enforces the configured bound against `proc_pidinfo` resident size.
Windows resource limits are deferred.

`onecad-kernelbench-runner` is an internal C++ executable. It reads one bounded
JSON execution request from stdin and writes exactly one result JSON line to
stdout. Diagnostic text belongs on stderr. The runner is not an OCW1 worker and
does not share `onecad-worker`'s framed stdout contract.

The `raw-occt` backend calls `BRepFilletAPI_MakeFillet` directly. It has no
OneCAD healing, retry, radius clamp, or fallback. The `onecad` backend calls the
production `FilletBuilder`; benchmark code does not copy production modeling
logic. Both backends regenerate the same semantic input and report an
`inputDigest` so the supervisor can prove that fact.

Production `audit_shape()` behavior remains unchanged. Benchmark-only deep
checks compose around it: exact `BRepCheck_Analyzer`, full single-threaded
self-interference checking, closed-manifold edge use, mass properties,
tolerance distributions, and normalized micro-topology metrics. A
topologically-empty solid (no faces) is treated as invalid even though
`BRepCheck_Analyzer` alone reports it valid.

A case's semantic selector resolves against freshly generated geometry by
nearest-anchor matching: recipe-local anchor points (edge midpoints, or a
common vertex for corner-incident selection) are matched to candidate
edges/vertices within a `1e-6` tolerance, and a runner-up within tolerance is
an ambiguous match, not a guess. No ordinal ever crosses the wire.

Metamorphic (translated/rotated) variants are checked against real evidence,
not proxies. On success the runner emits a `shapeSignature`: deterministic
output-face centroid samples tagged by surface kind, plus solid
classification of a handful of case-radius-scaled probe points anchored on
the shape's own centroid, alongside the shared pre-transform `rotationCenter`
pivot. The supervisor inverse-transforms the variant's points using the
wire-reported translation/rotation and compares them against the base
variant's within the case's declared `pointTolerance`. Absence of evidence on
both sides (e.g. an expected-limit case refusing identically under every
variant) reports `notRun`, not a fabricated pass.

## Verdict and exit policy

Supported cases gate semantic success, deterministic replay, deep audit, and
raw-success/OneCAD-failure regressions. Expected-limit cases pass when refusal
is safe and deterministic. Exploratory rejection is characterization; crash,
timeout, nondeterminism, and invalid OneCAD publication still fail.

Supervisor exit codes are:

- `0`: all gating verdicts passed.
- `1`: benchmark regression.
- `2`: CLI or schema error.
- `3`: runner missing or execution environment unsupported.

OCCT 8.0.1 T0 is the required shipping gate. OCCT 7.9.3 is an informational
safety characterization and cannot replace the 8.0.1 gate. Results must always
identify the actual kernel build.

`worker/tools/filletbench` remains the dedicated performance microbenchmark.
KBR-0 records timing distributions but introduces no relative performance
threshold.

## Data layout

- `schemas/case-v1.schema.json`: strict public case format.
- `schemas/result-v1.schema.json`: compact JSONL result format, including the
  nullable `shapeSignature` metamorphic-evidence block.
- `schemas/preset-v1.schema.json`: strict frozen-preset format.
- `presets/fillet-foundation-t0.json`: frozen T0 generation contract.
- `regressions/`: reviewed, replayable cases. It is not the legacy corpus.

Cases and artifact paths use restricted identifiers. A selector records its
generator provenance, topology role, recipe-local anchors, surface descriptors,
and adjacency relationship. Generator and recipe fields in selector provenance
must match the enclosing case. Raw OCCT ordinals are never persisted.

Quality bands are benchmark observations, not product policy. Only explicit
per-case ceilings gate. KBR-0 must not mutate tolerances or establish a generic
production tolerance threshold.
