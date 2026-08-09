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
