/** Capture release source and relink inputs without downloading or publishing. */
import { copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, renameSync,
  rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { canonicalPath, checkOcct, checkOcctCmake, checkSource, files, git, hash, inside, json, metadataSchema,
  occtSchema, parseArgs, read, requireFile, safePath, workerSchema } from './release-materials';
import { verifyArchive } from './verify-release-materials';

export interface PreparationOptions {
  source: string; worker: string; workerManifest: string; occtPrefix: string;
  occtArchive: string; eigenPrefix: string; boostPrefix: string; boostLicense: string; output: string;
}
const save = (root: string, name: string, bytes: Uint8Array | string): void => {
  mkdirSync(dirname(join(root, name)), { recursive: true });
  writeFileSync(join(root, name), bytes);
};
const copy = (root: string, name: string, source: string): void => save(root, name, requireFile(source));

const sourceState = (root: string): { revision: string; status: Buffer; patch: Buffer; names: string[] } => {
  const submodules = git(root, 'ls-files', '--stage').toString().split('\n').some(line => line.startsWith('160000 '));
  if (submodules) throw new Error('submodules require explicit source capture support');
  const names = [...new Set(git(root, 'ls-files', '--cached', '--others', '--exclude-standard', '-z')
    .toString().split('\0').filter(Boolean))].sort().filter(name => {
    safePath(name);
    try {
      lstatSync(join(root, name));
      return true;
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') return false;
      throw error;
    }
  });
  return { revision: git(root, 'rev-parse', 'HEAD').toString().trim(),
    status: git(root, 'status', '--porcelain=v1', '-z', '--untracked-files=all'),
    patch: git(root, 'diff', '--binary', 'HEAD', '--'), names };
};

const snapshot = (root: string, destination: string, names: string[]): void => {
  for (const name of names) {
    const input = join(root, name);
    if (!lstatSync(input).isFile()) throw new Error(`unsupported source type (including symlink): ${name}`);
    if (read(input).subarray(0, 100).toString().startsWith('version https://git-lfs.github.com/spec/v1')) {
      throw new Error(`unexpanded Git LFS source: ${name}`);
    }
    mkdirSync(dirname(join(destination, name)), { recursive: true });
    copyFileSync(input, join(destination, name));
  }
  checkSource(destination);
};

const dependencies = (root: string, options: PreparationOptions): { eigen: string; boost: string } => {
  const eigenVersion = join(options.eigenPrefix, 'share/eigen3/cmake/Eigen3ConfigVersion.cmake');
  const boostVersion = join(options.boostPrefix, 'include/boost/version.hpp');
  const eigen = requireFile(eigenVersion).toString().match(/set\(PACKAGE_VERSION "([0-9.]+)"\)/)?.[1];
  const number = requireFile(boostVersion).toString().match(/^#define BOOST_VERSION\s+(\d+)$/m)?.[1];
  if (!eigen || !number) throw new Error('missing Eigen or Boost version evidence');
  const value = Number(number);
  const boost = `${Math.floor(value / 100000)}.${Math.floor(value / 100) % 1000}.${value % 100}`;
  copy(root, 'dependencies/eigen-version.cmake', eigenVersion);
  copy(root, 'dependencies/boost-version.hpp', boostVersion);
  for (const name of ['README', 'MPL2', 'BSD', 'APACHE', 'MINPACK']) {
    copy(root, `licenses/Eigen-COPYING.${name}`, join(options.eigenPrefix, `COPYING.${name}`));
  }
  copy(root, 'licenses/Boost-LICENSE_1_0.txt', options.boostLicense);
  return { eigen, boost };
};

const relinkInstructions = (buildId: string): string => `# Source and relink materials

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

Use macOS with Xcode command-line tools, CMake, Ninja, Rust/Cargo and Bun.
Eigen and Boost versions and their license texts are recorded in dependencies/
and licenses/. Install those versions and make their CMake package directories
available through CMAKE_PREFIX_PATH. Their complete header distributions and
Cargo/npm dependency caches are not included. The transitive dependency license
inventory is incomplete. This archive is not a qualified offline build kit.

From source/, build the unchanged mirrored OCCT with the existing pinned script:

\`\`\`sh
export ONECAD_OCCT_DOWNLOAD_DIR="$PWD/../occt"
scripts/build-pinned-occt.sh --version 8.0.1 \\
  --prefix "$PWD/../relinked-occt" --build-dir "$PWD/../occt-build" \\
  --build-id "${buildId}"
export ONECAD_OCCT_ROOT="$PWD/../relinked-occt"
export ONECAD_OCCT_BUILD_ID="${buildId}"
bun install --frozen-lockfile
\`\`\`

Modify worker/third_party/planegcs as needed, then run scripts/package-macos.sh
without --install. Its two-pass procedure rebuilds the worker, bundles dylibs,
re-stages the rewritten worker SHA-256, rebuilds the app with that manifest,
and verifies the bundled worker. The release app pins the worker SHA-256, so
rebuilding the app is required after replacing or relinking that worker.
Read docs/PACKAGING.md before running this locally; package-macos.sh performs
local ad-hoc signing. This preparation tool never runs it.

For a compatible modified OCCT build, replace OCCT dylibs under the rebuilt
app's Contents/Frameworks and re-sign the changed dylibs and outer bundle
ad hoc. The worker hash does not cover those dylibs. A notarized/hardened-runtime
replacement procedure remains unverified. Do not re-sign the worker after the
app embeds its hash; if worker bytes change, repeat the two-pass procedure.

## Before distribution

These materials must actually accompany the binary at its distribution location,
or a reviewed qualifying source offer must be in place before distribution,
including beta distribution. Mirror occt/OCCT-V8_0_1.tar.gz with each release.
Review notices, dependency materials, actual source availability and relinking.
Preparation and hash verification do not prove those release gates passed.
`;

const copyExternal = (materials: string, options: PreparationOptions): void => {
  copy(materials, 'worker-manifest.json', options.workerManifest);
  copy(materials, 'occt/occt-build.json', join(options.occtPrefix, 'share/onecad/occt-build.json'));
  copy(materials, 'occt/occt-build.cmake', join(options.occtPrefix, 'share/onecad/occt-build.cmake'));
  copy(materials, 'occt/OCCT-V8_0_1.tar.gz', options.occtArchive);
  const licenseRoot = join(options.occtPrefix, 'share/doc/opencascade');
  copy(materials, 'licenses/OCCT-LICENSE_LGPL_21.txt', join(licenseRoot, 'LICENSE_LGPL_21.txt'));
  copy(materials, 'licenses/OCCT-LGPL_EXCEPTION.txt', join(licenseRoot, 'OCCT_LGPL_EXCEPTION.txt'));
};

export const prepareMaterials = (options: PreparationOptions): string => {
  const root = canonicalPath(options.source);
  options = { ...options, output: canonicalPath(options.output) };
  if (git(root, 'rev-parse', '--show-toplevel').toString().trim() !== root) throw new Error('--source must be repository root');
  if (inside(root, options.output)) throw new Error('--output must be outside source repository');
  if (existsSync(options.output)) throw new Error('--output already exists; refusing to overwrite');
  const manifestBytes = requireFile(options.workerManifest);
  const manifest = workerSchema.parse(JSON.parse(manifestBytes.toString()));
  const workerHash = hash(requireFile(options.worker));
  if (manifest.binarySha256 !== workerHash) throw new Error('worker SHA-256 differs from worker manifest');
  const occt = occtSchema.parse(JSON.parse(requireFile(join(options.occtPrefix, 'share/onecad/occt-build.json')).toString()));
  checkOcct(root, occt, options.occtArchive);
  checkOcctCmake(occt, join(options.occtPrefix, 'share/onecad/occt-build.cmake'));
  const before = sourceState(root);
  mkdirSync(dirname(options.output), { recursive: true });
  const temporary = mkdtempSync(join(dirname(options.output), '.onecad-materials-'));
  try {
    const materials = join(temporary, 'materials');
    snapshot(root, join(materials, 'source'), before.names);
    copyExternal(materials, options);
    const versions = dependencies(materials, options);
    save(materials, 'source-status.z', before.status);
    save(materials, 'source-changes.patch', before.patch);
    save(materials, 'RELINK.md', relinkInstructions(occt.buildId));
    const metadata = metadataSchema.parse({ schemaVersion: 1, preparedAt: new Date().toISOString(),
      scope: 'local-source-material-preparation', source: { revision: before.revision,
        dirty: before.status.length > 0, exactTags: git(root, 'tag', '--points-at', 'HEAD').toString().trim().split('\n').filter(Boolean),
        snapshotPolicy: 'tracked-and-untracked-nonignored', fileCount: files(join(materials, 'source')).length },
      worker: { binarySha256: workerHash, manifestSha256: hash(manifestBytes) }, occt, dependencies: versions,
      validation: { buildProvenance: 'unverified', relink: 'unverified', publication: 'unverified',
        transitiveLicenseInventory: 'incomplete' } });
    save(materials, 'release-materials.json', json(metadata));
    assertStable(root, join(materials, 'source'), before);
    const entries = files(materials).map(path => ({ path, sha256: hash(read(join(materials, path))),
      bytes: lstatSync(join(materials, path)).size, executable: Boolean(lstatSync(join(materials, path)).mode & 0o111) }));
    save(materials, 'inventory.json', json({ schemaVersion: 1, files: entries }));
    const archive = join(temporary, 'onecad-source-materials.tar.gz');
    runTar(temporary, archive);
    verifyArchive(archive, options.worker);
    if (hash(requireFile(options.workerManifest)) !== hash(manifestBytes)) throw new Error('worker manifest changed during preparation');
    assertStable(root, join(materials, 'source'), before);
    save(temporary, 'SHA256SUMS', `${hash(read(archive))}  onecad-source-materials.tar.gz\n`);
    renameSync(temporary, options.output);
    return join(options.output, 'onecad-source-materials.tar.gz');
  } catch (error) {
    rmSync(temporary, { recursive: true, force: true });
    throw error;
  }
};

const assertStable = (root: string, snapshotRoot: string, before: ReturnType<typeof sourceState>): void => {
  const after = sourceState(root);
  if (before.revision !== after.revision || !before.status.equals(after.status) ||
      !before.patch.equals(after.patch) || json(before.names) !== json(after.names)) {
    throw new Error('source state changed during preparation');
  }
  for (const name of before.names) {
    if (hash(read(join(root, name))) !== hash(read(join(snapshotRoot, name))) ||
        Boolean(lstatSync(join(root, name)).mode & 0o111) !== Boolean(lstatSync(join(snapshotRoot, name)).mode & 0o111)) {
      throw new Error(`source changed during preparation: ${name}`);
    }
  }
};
const runTar = (temporary: string, archive: string): void => {
  const result = Bun.spawnSync(['tar', '-czf', archive, '-C', temporary, 'materials'], { stderr: 'pipe' });
  if (result.exitCode !== 0) throw new Error(`tar failed: ${result.stderr.toString()}`);
};

if (import.meta.main) {
  try {
    const args = parseArgs(process.argv.slice(2), ['--source', '--worker', '--worker-manifest', '--occt-prefix',
      '--occt-archive', '--eigen-prefix', '--boost-prefix', '--boost-license', '--output']);
    const archive = prepareMaterials({ source: args['--source'], worker: args['--worker'],
      workerManifest: args['--worker-manifest'], occtPrefix: args['--occt-prefix'], occtArchive: args['--occt-archive'],
      eigenPrefix: args['--eigen-prefix'], boostPrefix: args['--boost-prefix'], boostLicense: args['--boost-license'],
      output: args['--output'] });
    console.log(`Prepared local materials: ${archive}\nBuild, relink and publication remain unverified.`);
  } catch (error) {
    console.error(`prepare-release-materials: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
