# Repository tooling

Run `cargo xtask COMMAND` from this repository. Cargo uses the root workspace's
pinned toolchain. No Bun, Node, Tauri or WebDriver installation is required.
CMake and shell scripts still build the production C++ OCCT/PlaneGCS sidecar.

## Command migration

The former `bun`/`node scripts/<name>.mjs` or `.ts` entry points are retired.
Their Rust replacements keep the same command-specific inputs:

| Former script | Rust command |
|---|---|
| `check-rust-layers.mjs` | `cargo xtask check-rust-layers` |
| `verify-modeling-coverage.mjs` | `cargo xtask verify-modeling-coverage` |
| `verify-modeling-contracts.mjs` | `cargo xtask verify-modeling-contracts` |
| `native-host-gate.mjs OUTPUT_DIRECTORY` | `cargo xtask native-host-gate OUTPUT_DIRECTORY` |
| `verify-render-captures.mjs DIRECTORY` | `cargo xtask verify-render-captures DIRECTORY` |
| `kernelbench-manifest.mjs MODE DIRECTORY SUITE MANIFEST` | `cargo xtask kernelbench-manifest MODE DIRECTORY SUITE MANIFEST` |
| `prepare-release-materials.ts OPTIONS` | `cargo xtask prepare-release-materials OPTIONS` |
| `verify-release-materials.ts --archive ARCHIVE --worker WORKER` | `cargo xtask verify-release-materials --archive ARCHIVE --worker WORKER` |
| `bake-cad-icons.ts` | `cargo xtask bake-cad-icons` |
| `bun test scripts/tests` | `cargo test -p xtask` |

`cargo xtask ios-simulator-gate [--provenance-only | --verify-only] DIRECTORY`
is new (no former script): the iPad simulator gate and its verifier,
[docs/ios/SIMULATOR_GATE.md](../docs/ios/SIMULATOR_GATE.md).

`cargo xtask python-oracles` executes all six required Python observer and
reference-oracle files in isolated Python processes. Each process prints the
discovered test inventory. A missing file, empty suite, duplicate test identity,
failed test, or skipped test fails the command.

## Automated macOS qualification

Run `cargo xtask qualify-macos --out /absolute/new/directory` on macOS. The output
must be new and outside the repository. The command builds pinned OCCT 8.0.1,
the Release worker and the real Bambu addon, then runs formatting, workspace
Clippy and tests, CTest, architecture/modeling/reference checks, the Python
suites, bounded admission fuzzing, strict native lifecycle tests, Metal captures
and the packaged worker selftest. Install the worker's existing CMake, Ninja,
Boost, Eigen and nlohmann-json prerequisites first. `ONECAD_OCCT_ROOT` selects
the pinned installation prefix; it defaults to the ignored repository path
`worker/build-qualification-occt-prefix/8.0.1`.

Every phase retains its command, exit status and log. The campaign records the
source commit, tree, worktree status, tracked diff hash and untracked file
hashes, plus OS/build, hardware, graphics adapter, architecture, SDK, Xcode and
Rust toolchain output. The native gate retains its exact parent inventories;
the campaign separately accounts for the three required fresh-process repair
children. CTest and Metal discovery must match their executed inventories with
no skips. Workspace discovery must be covered by observed harness records;
additional child records and declared ignored platform tiers remain separate
from native and Metal qualification. Workspace test functions run serially;
explicit concurrency inside tests remains exercised. Packaging uses the existing
signing and integrity policy. The final
app is archived as `OneCAD.app.tar.gz` to retain executable modes and symlinks
when evidence is uploaded and downloaded.

Renderer capture environment is scoped to the Metal phase. Inherited capture
settings are removed from other phases so workspace and native diagnostics cannot
contaminate the strict Metal receipt inventory. The native gate owns its own captures.

The first failed phase stops execution and leaves an incomplete campaign and
its logs. Missing prerequisites never produce a pass. Source changes during
execution invalidate the campaign. Recheck downloaded evidence with
`cargo xtask verify-qualification DIRECTORY` in the same source checkout; it
requires every phase exactly once, successful command statuses, matching child
accounting and an unchanged artifact inventory and SHA-256 hashes. The manual
`Automated macOS qualification` workflow performs this download check.

An automated available-host pass does not establish physical input, presentation
latency, latest-stable macOS acceptance or final integration approval. A dirty
worktree is recorded explicitly; final qualification still requires the exact
authorized integration commit and its separate acceptance evidence.

`make check-tooling` runs the live guards and icon equivalence check.
`make test-tooling` runs their regression tests. `check-hygiene` rejects retired
setup and duplicate management files, including known ignored root paths.

The dependency guard reads Cargo package identities across normal, development,
build, optional, renamed and target-specific declarations. Tauri, Wry and WebKit
are forbidden even in host crates. Application dependency layering remains
separate from this global ban.

Kernelbench modes are `record`, `compare`, `semantic-record`, `semantic-compare`.
Exit statuses remain 1 for mismatch, 2 for usage, 3 for missing evidence/baseline.
`ONECAD_BENCH_PLATFORM` retains the explicit platform override. Record modes
change baselines deliberately; never record over a failure to make it pass.

Release preparation requires `--source`, `--worker`, `--worker-manifest`,
`--occt-prefix`, `--occt-archive`, `--eigen-prefix`, `--boost-prefix`,
`--boost-license`, and `--output`. Output must be new and outside the repository.
The archive verifier retains source, license, OCCT pin, inventory, executable
mode, path and hash checks. Local preparation is not distribution compliance or
successful relinking.

## Worker and packaging helpers

`scripts/build-worker.sh` calls `worker-manifest FRAME BINARY MANIFEST` after
capturing the actual OCW1 hello. `scripts/package-macos.sh` uses
`target-directory`, `rehash-worker-manifest BINARY MANIFEST`, and
`manifest-hash MANIFEST`. Packaging rewrites and signs the worker before its
final hash is embedded in the host. Never re-sign the nested worker afterward.

## Retained data and provenance

- `assets/native/tokens.css` is live Rust-embedded theme data. Its values and
  light/dark behavior were retained unchanged.
- `assets/native/cad-icons.json` contains ordered unprojected icon geometry;
  Rust generates the existing widget SVG registries. `bake-cad-icons --check`
  checks both files byte for byte. See [the source format](../assets/native/cad-icons.md).
- `assets/native/app-icons` and `assets/native/brand` hold brand icons generated
  from the SVG masters by `scripts/brand-icons.sh`. Packaging bundles
  `brand/macos/OneCAD.icns`.
- `docs/qa/expressions.fixture.json` and modeling coverage/contracts/residual JSON
  remain executable versioned inputs despite their documentation directory.
- Protocol schemas/fixtures, render fixtures/goldens, the corpus, robustness
  baselines, STEP ingest recipe and optional `spikes/inproc-worker` remain live.
- Third-party C++ sources and notices are preserved; extension-based retirement
  applies to repository automation, not vendored files.

Management pointers and completed toolkit/render investigations were retired
against reviewed revision `ed9ed33f6f394897612e0a40edee015edd775ef8`.
Their source remains in Git history; current decisions, historical conclusions
and validation outcomes belong in the existing Plane project and canonical
Pages, indexed from the root README. Cleanup establishes no product acceptance.
