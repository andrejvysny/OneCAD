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
| `fillet/foundation:t0` | 136 | linux-x64 2026-08-14 · darwin-arm64 2026-08-15, semantic-publication hardening |
| `fillet/matrix:m1` | 120 | 2026-08-09, GH-0 WP0.3 |

### 2026-08-14 T0 semantic update

The production tolerance-growth gate intentionally changed eight `onecad/base`
`valence4-*` rows from pass to characterization refusal. Their input maximum
B-Rep tolerance is ~`1e-7 mm`; OCCT's otherwise BRep-valid outputs inflate it to
`0.0086–0.4008 mm`, beyond the owned Fillet ceiling
`max(0.001 mm, 2 × input + 0.000001 mm)`. The suite remains 136 records with
128 passes, 8 characterizations, zero gating failures, zero differential
regressions, and stable replay. Only those eight Linux normalized digests and the
portable semantic counts were re-recorded from Actions run 48; this is the named
policy change the manifest is meant to expose, not a geometry-baseline refresh.

### 2026-08-15 T0 darwin-arm64 completion of the update above

**The 2026-08-14 update was half-applied.** It re-recorded the eight `valence4-*`
Linux rows and left their **darwin-arm64 twins at their pre-gate values from
`fc55419`**. The tolerance-growth gate is platform-independent, so those eight
darwin rows had been stale — and the local darwin digest compare red — from
`6a0cfb1` onward. Nothing caught it because no CI job compares darwin digests
(the macOS lane runs the portable semantics gate; only `linux-kernelbench`
compares digests), and the local compare is a manual step.

Re-recorded here from a darwin-arm64 run, with the cause established BEFORE
overwriting, per the rule above. The evidence that this is a stale baseline and
not a regression:

- **128 of the 136 darwin rows re-recorded to byte-identical values.** Only the
  eight known cases moved. A build or environment problem would not be that
  selective.
- **Three independent builds agree exactly with each other** and disagreed
  identically with the old baseline: a clean rebuild at branch HEAD, an
  incremental build at branch HEAD, and a clean build in a detached worktree at
  **`d7cd9f1` itself — the commit that recorded the baseline.** A baseline that
  does not reproduce from its own commit is stale by definition.
- **`semantic-compare` passed on darwin throughout**, before and after. The
  portable, meaning-carrying gate never moved; only the byte-level digest did.

This also **falsifies the build-hygiene hypothesis for darwin**: clean and
incremental produced identical digests, so a persistent build tree was not the
cause here.

**The Linux row is a separate and still-open question, with the opposite sign.**
CI at `8c00c4d` reports Linux now producing
`8287fbb35f3c2f3e1f5920d43a0e0580ee0a2963bb611ba83baf0bc42e01f9b3` for
`valence4-00` — which is the value that was current **before** `d7cd9f1`, i.e.
the **pre-gate** digest. Linux is behaving as though the tolerance-growth gate
were absent, even though Actions run 48 recorded it firing there. A stale binary
in the persistent `LINUX_WORKER_BUILD` tree fits that signature exactly. Do NOT
re-record the Linux rows: run `self-hosted.yml` `s3-worker` with
`clean_build: true`, then `s4-kernelbench`, and decide from the result.

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
values per record a straddle is near-certain.

It is narrower still than "same platform". A `darwin-arm64` baseline recorded on
a developer Mac did NOT reproduce on GitHub's `macos-14` image — same pinned OCCT
source, same build id, same architecture, but a different AppleClang — and the
same trig-heavy `valence4-*` family moved. A digest is therefore a **same-machine**
regression tripwire. It is gated only on the self-hosted runner, which is one
persistent machine; the ephemeral CI images gate semantics instead.

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
