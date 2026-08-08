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
- `schemas/case-v2.schema.json`: the expanded matrix format. A SEPARATE format,
  not an extension: v1 is frozen, and neither version is readable as the other
  (`schemaVersion` is a hard const on both sides, in the schema, in Rust, and in
  the C++ parser).
- `presets/fillet-foundation-t0.json`: frozen T0 generation contract.
- `examples/`: illustrative cases. NOT contractual — `regressions/` is.
- `regressions/`: reviewed, replayable cases. It is not the legacy corpus.

## Suites

| Suite | Preset | Case format | What it sweeps |
| --- | --- | --- | --- |
| `fillet/foundation` | `t0` | v1 | Box / valence / overflow recipes; edges between two PLANES. |
| `fillet/matrix` | `m1` | v2 | Support-surface pairs: plane↔plane, plane↔cylinder, cylinder↔cylinder, plane↔cone, swept over the dihedral angle. |

`supportPair` geometry has two constructions. The prismatic pairs
(plane|cylinder × plane|cylinder) are one 2D profile extruded along Z, so the
shared edge is a straight vertical line through the origin and the dihedral is
free over `(0, 180)`. The plane↔cone pair is a frustum's base circle, where the
dihedral is `90 - halfAngle` by construction — the generator rebuilds it from
the half angle and REFUSES a case that declares anything else, rather than
quietly generating different geometry than the file describes.

A blend needs room. For a prismatic pair the usable throat is
`R·(1 - cos θ)`, where `R` is the tightest curved support (or half a planar
support's width when neither is curved). Measured against OCCT 8.0.1: every pair
blends at 0.20 of that throat, and the cylinder↔cylinder lens refuses at 0.40.
Supported cases sit at 0.04–0.16.

Not every validator is recipe-agnostic. `cylindricalRadius` counts EVERY
cylindrical face in the output against the requested radius, so a cylindrical
support reads as a blend of the wrong radius, and `g1BoundaryTangency` only
recognises plane↔cylinder tangency pairs. Both are emitted only for all-planar
pairs. Their general replacements are expressible in case-v2 but not implemented
in the runner; an unimplemented validator reports `notApplicable`, which fails a
required check rather than passing it.

Cases and artifact paths use restricted identifiers. A selector records its
generator provenance, topology role, recipe-local anchors, surface descriptors,
and adjacency relationship. Generator and recipe fields in selector provenance
must match the enclosing case. Raw OCCT ordinals are never persisted.

Quality bands are benchmark observations, not product policy. Only explicit
per-case ceilings gate. KBR-0 must not mutate tolerances or establish a generic
production tolerance threshold.
