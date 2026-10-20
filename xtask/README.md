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
- `assets/native/app-icons` retains native packaging icon sources relocated
  from the retired setup. The current packaging script does not select them.
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
