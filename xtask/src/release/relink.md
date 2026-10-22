# Source and relink materials

This is a local preparation artifact, not evidence of successful relinking,
reproducibility, public source availability, or release compliance. Review
release-materials.json and inventory.json. Source includes the current tracked
and untracked nonignored working-tree files, including dirty modifications.
Ignored files and Git history are excluded; deleted files remain absent.
source-status.z is Git porcelain v1 with NUL separators; source-changes.patch
records tracked changes against the recorded revision. The inventory binds
all captured source bytes, including untracked files and executable modes.
The final worker hash is an association with these materials, not proof that
that worker was built from this snapshot. No clean tag or reproducible build
is claimed. Exact tags describe HEAD only and do not describe dirty changes.

## Rebuild and relink

Use macOS with Xcode command-line tools, CMake, Ninja, Rust/Cargo.
Install the guest target with `rustup target add wasm32-unknown-unknown`.
Eigen and Boost versions and their license texts are recorded in dependencies/
and licenses/. Install those versions and make their CMake package directories
available through CMAKE_PREFIX_PATH. Their complete header distributions and
Cargo dependency caches are not included. The transitive dependency license
inventory is incomplete. This archive is not a qualified offline build kit.

From source/, build the unchanged mirrored OCCT with the existing pinned script:

```sh
export ONECAD_OCCT_DOWNLOAD_DIR="$PWD/../occt"
scripts/build-pinned-occt.sh --version 8.0.1 \
  --prefix "$PWD/../relinked-occt" --build-dir "$PWD/../occt-build" \
  --build-id "@BUILD_ID@"
export ONECAD_OCCT_ROOT="$PWD/../relinked-occt"
export ONECAD_OCCT_BUILD_ID="@BUILD_ID@"
```

Modify worker/third_party/planegcs as needed, then run scripts/package-macos.sh.
Its native procedure builds the worker and source addon components, bundles dylibs,
re-stages the rewritten worker SHA-256, builds the host with that manifest,
bundles the generated addon packages under Contents/Resources/addons,
and verifies the bundled worker. Addon source, manifests, WIT, Rust SDK and
build tooling are included in source/; ignored compiled guest and staged package
outputs are regenerated during packaging. The release app pins the worker SHA-256, so
rebuilding the app is required after replacing or relinking that worker.
Read docs/PACKAGING.md before running this locally; package-macos.sh performs
local ad-hoc signing. This preparation tool never runs it.

For a compatible modified OCCT build, replace OCCT dylibs under the rebuilt
app's Contents/Frameworks and re-sign the changed dylibs and outer bundle
ad hoc. The worker hash does not cover those dylibs. A notarized/hardened-runtime
replacement procedure remains unverified. Do not re-sign the worker after the
app embeds its hash; if worker bytes change, repeat the bundle, hash and host-build procedure.

## Before distribution

These materials must actually accompany the binary at its distribution location,
or a reviewed qualifying source offer must be in place before distribution,
including beta distribution. Mirror occt/OCCT-V8_0_1.tar.gz with each release.
Review notices, dependency materials, actual source availability and relinking.
Preparation and hash verification do not prove those release gates passed.
