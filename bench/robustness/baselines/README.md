# Kernelbench digest baselines

`digests.json` freezes `(suite, caseId, backend, variant) → {inputDigest, normalizedDigest}`
for the fillet suites, so a refactor can be asked the question the suites cannot
answer on their own: **did this change move a digest?**

## Why the existing signals are not enough

- **Replay** (`campaign.rs`) runs each case twice in the same process tree and
  compares. That proves determinism *within* a build. It retains nothing, so it
  cannot see across a code change.
- **The report** (`gatingFailures: 0`, `136/136`) is an aggregate. A case can
  change shape — different geometry, different validators run, a different
  publication decision — and still pass. Counts stay identical; the digest does not.

The manifest is the missing half: committed, per-record, and diffed.

## Use

```bash
K=src-tauri/target/release/onecad-kernelbench
R=worker/build/onecad-kernelbench-runner

ONECAD_KERNELBENCH_RUNNER=$R $K run --suite fillet/foundation --preset t0 --backend both --out-dir <dir>
node scripts/kernelbench-manifest.mjs compare <dir> fillet/foundation:t0 bench/robustness/baselines/digests.json
```

`record` replaces one suite's rows in place (so a removed case does not linger) and
leaves other suites untouched. `compare` exits non-zero and names every changed,
missing and unexpected row.

**Re-recording is a decision, not a chore.** A moved digest means the geometry or
the case definition changed; establish *why* before overwriting, and say so in the
`TODO.md` gate entry. Overwriting to make a gate green destroys the only evidence
that the change was unintended.

## What is pinned today

| Suite | Rows | Recorded |
|---|---|---|
| `fillet/foundation:t0` | 136 | 2026-08-09, GH-0 WP0.3 |
| `fillet/matrix:m1` | 120 | 2026-08-09, GH-0 WP0.3 |

Both were captured with the worker built from `~/.onecad-occt/8.0.1` (pinned OCCT
8.0.1). A different OCCT build is expected to move digests — that is the point of
the pin, and `kernel`/`modeler` fields in each record carry the provenance.

The comparator itself is proven by mutating one saved digest and watching the
compare fail; do that again if you change the script.

## Platform scope (2026-08-10)

`digests.json` is keyed
`suite:preset|caseId|backend|variant|platform` — `darwin-arm64`, `linux-x64`.
`semantics.json` is keyed by suite alone and is deliberately NOT platform-scoped.

The split is measured, not precautionary. Running T0 on macOS/arm64/AppleClang
and Linux/x86_64/gcc 14 against the same pinned OCCT 8.0.1 — identical build id
and identical 16-hex kernel fingerprint — 182 of 272 digest values differ:

| what | changed |
|---|---|
| `translated` inputDigest | 0 / 32 — translation is exact in floating point |
| `rotated` inputDigest | 32 / 32 — the transform goes through trig |
| `base` inputDigest | 20 / 72 — exactly the trig-built shapes (all 8 `valence4`, `overflow-02/-03`); every box and `valence3` is bit-identical |
| `normalizedDigest` | 130 / 136 — OCCT's own arithmetic diverges |

The 1e-9 quantization does not make a digest portable: rounding to a grid
narrows but never closes boundary straddles, and with thousands of quantized
values per record a straddle is near-certain. A digest is therefore a SAME-HOST
regression tripwire. Comparing one across hosts would report a kernel regression
every time.

What IS portable is behaviour, and `semantics.json` pins it: records,
gatingFailures, counts by verdict/domain/failureClass/backend/generator,
differential classification, replay stability, metamorph outcome, critical
transitions. Both hosts satisfy the same row. Timing and tolerance distributions
are excluded — they describe the machine, not the kernel.

Recording a new platform:

```bash
node scripts/kernelbench-manifest.mjs record          <out-dir> <suite:preset> bench/robustness/baselines/digests.json
node scripts/kernelbench-manifest.mjs semantic-record <out-dir> <suite:preset> bench/robustness/baselines/semantics.json
```

`ONECAD_BENCH_PLATFORM` overrides the detected platform, for recording from an
artifact produced on another host. `compare` on a platform with no rows exits 3
rather than reporting every row as missing.
