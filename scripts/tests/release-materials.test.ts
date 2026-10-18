/** Synthetic local repositories only; these fixtures do not establish license compliance. */
import { afterEach, expect, test } from 'bun:test';
import { chmodSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { prepareMaterials, type PreparationOptions } from '../prepare-release-materials';
import { checkOcct, hash, occtPin, json, occtSchema, requiredSource, run } from '../release-materials';
import { verifyArchive, verifyDirectory } from '../verify-release-materials';

const roots: string[] = [];
afterEach(() => roots.splice(0).forEach(root => rmSync(root, { recursive: true, force: true })));
const save = (root: string, name: string, text: string): void => {
  mkdirSync(dirname(join(root, name)), { recursive: true });
  writeFileSync(join(root, name), text);
};

const fixture = (): PreparationOptions => {
  const root = mkdtempSync(join(tmpdir(), 'onecad-materials-test-'));
  roots.push(root);
  const source = join(root, 'source');
  const occtPrefix = join(root, 'occt-prefix');
  const eigenPrefix = join(root, 'eigen');
  const boostPrefix = join(root, 'boost');
  mkdirSync(source);
  for (const name of requiredSource) save(source, name, `synthetic ${name}\n`);
  for (const name of ['Cargo.toml', 'Cargo.lock', 'crates/onecad-app/src/lib.rs', 'crates/onecad-ui/src/lib.rs', 'crates/onecad-render/src/lib.rs', 'worker/src/main.cpp']) {
    save(source, name, `synthetic ${name}\n`);
  }
  save(source, '.gitignore', 'ignored-file\n');
  save(source, 'ignored-file', 'excluded by policy\n');
  save(root, 'occt-archive', 'synthetic OCCT cached archive bytes\n');
  const archiveSha256 = hash(readFileSync(join(root, 'occt-archive')));
  save(source, 'scripts/build-pinned-occt.sh', `8.0.1 | V8_0_1)
COMMIT="b8f597c677811d1f9f4d8a97f5ae2825c0353a42"
ARCHIVE_SHA256="${archiveSha256}"
;;
readonly NORMALIZED_OPTIONS="BUILD_LIBRARY_TYPE=Shared"
`);
  save(occtPrefix, 'share/onecad/occt-build.json', json({ schemaVersion: 1, version: '8.0.1', tag: 'V8_0_1',
    sourceCommit: 'b8f597c677811d1f9f4d8a97f5ae2825c0353a42', archiveSha256, buildId: 'synthetic-test-build',
    kernelPolicyVersion: 1, normalizedBuildOptions: 'BUILD_LIBRARY_TYPE=Shared' }));
  save(occtPrefix, 'share/onecad/occt-build.cmake', `set(ONECAD_OCCT_SOURCE_COMMIT "b8f597c677811d1f9f4d8a97f5ae2825c0353a42")
set(ONECAD_OCCT_NORMALIZED_BUILD_OPTIONS "BUILD_LIBRARY_TYPE=Shared")
set(ONECAD_OCCT_BUILT_ID "synthetic-test-build")
`);
  for (const name of ['LICENSE_LGPL_21.txt', 'OCCT_LGPL_EXCEPTION.txt']) {
    save(occtPrefix, `share/doc/opencascade/${name}`, `synthetic ${name}\n`);
  }
  save(eigenPrefix, 'share/eigen3/cmake/Eigen3ConfigVersion.cmake', 'set(PACKAGE_VERSION "5.0.1")\n');
  for (const name of ['README', 'MPL2', 'BSD', 'APACHE', 'MINPACK']) save(eigenPrefix, `COPYING.${name}`, `synthetic ${name}\n`);
  save(boostPrefix, 'include/boost/version.hpp', '#define BOOST_VERSION 109000\n');
  save(root, 'boost-license', 'synthetic Boost license fixture\n');
  save(root, 'worker', 'synthetic bundled worker bytes\n');
  save(root, 'worker-manifest.json', json({ manifestVersion: 1, binarySha256: hash(readFileSync(join(root, 'worker'))),
    occt: { version: '8.0.1', fingerprint: 'fixture-fingerprint' } }));
  run(['git', 'init', '--quiet', source]);
  run(['git', '-C', source, 'add', '.']);
  run(['git', '-C', source, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
    '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'Synthetic source fixture']);
  return { source, worker: join(root, 'worker'), workerManifest: join(root, 'worker-manifest.json'), occtPrefix,
    occtArchive: join(root, 'occt-archive'), eigenPrefix, boostPrefix, boostLicense: join(root, 'boost-license'),
    output: join(root, 'output') };
};
const metadata = (options: PreparationOptions): Record<string, unknown> =>
  JSON.parse(readFileSync(join(options.output, 'materials/release-materials.json'), 'utf8')) as Record<string, unknown>;

test('clean snapshot and final worker verify; source, OCCT mirror and dependency evidence present', () => {
  const options = fixture();
  const archive = prepareMaterials(options);
  verifyArchive(archive, options.worker);
  expect(metadata(options).source).toMatchObject({ dirty: false, exactTags: [], snapshotPolicy: 'tracked-and-untracked-nonignored' });
  expect(metadata(options).dependencies).toEqual({ eigen: '5.0.1', boost: '1.90.0' });
  expect(metadata(options).validation).toEqual({ buildProvenance: 'unverified', relink: 'unverified',
    publication: 'unverified', transitiveLicenseInventory: 'incomplete' });
  expect(existsSync(join(options.output, 'materials/source/ignored-file'))).toBe(false);
  expect(readFileSync(join(options.output, 'materials/occt/OCCT-V8_0_1.tar.gz'))).toEqual(readFileSync(options.occtArchive));
  expect(readFileSync(join(options.output, 'SHA256SUMS'), 'utf8')).toContain(hash(readFileSync(archive)));
});

test('dirty snapshot includes staged, unstaged, untracked bytes and excludes deleted files', () => {
  const options = fixture();
  run(['git', '-C', options.source, 'tag', 'fixture-head']);
  save(options.source, 'worker/src/main.cpp', 'staged worker change\n');
  run(['git', '-C', options.source, 'add', 'worker/src/main.cpp']);
  save(options.source, 'worker/src/main.cpp', 'unstaged worker change\n');
  save(options.source, 'untracked source.txt', 'new untracked bytes\n');
  rmSync(join(options.source, 'src/main.ts'));
  save(options.source, 'src/replacement.ts', 'replacement source\n');
  prepareMaterials(options);
  expect(metadata(options).source).toMatchObject({ dirty: true, exactTags: ['fixture-head'] });
  expect(readFileSync(join(options.output, 'materials/source/worker/src/main.cpp'), 'utf8')).toBe('unstaged worker change\n');
  expect(readFileSync(join(options.output, 'materials/source/untracked source.txt'), 'utf8')).toBe('new untracked bytes\n');
  expect(existsSync(join(options.output, 'materials/source/src/main.ts'))).toBe(false);
  expect(readFileSync(join(options.output, 'materials/source-changes.patch'), 'utf8')).toContain('unstaged worker change');
});

test.each(['LICENSE-MIT', 'worker/third_party/planegcs/LICENSE', 'Cargo.lock', 'scripts/package-macos.sh'])(
  'missing source or license fails closed: %s', missing => {
    const options = fixture();
    rmSync(join(options.source, missing));
    expect(() => prepareMaterials(options)).toThrow('missing regular material');
    expect(existsSync(options.output)).toBe(false);
  });

test.each(['occtArchive', 'boostLicense'] as const)('missing external material fails closed: %s', missing => {
  const options = fixture();
  rmSync(options[missing]);
  expect(() => prepareMaterials(options)).toThrow('missing regular material');
  expect(existsSync(options.output)).toBe(false);
});

test('missing OCCT license fails closed', () => {
  const options = fixture();
  rmSync(join(options.occtPrefix, 'share/doc/opencascade/OCCT_LGPL_EXCEPTION.txt'));
  expect(() => prepareMaterials(options)).toThrow('missing regular material');
  expect(existsSync(options.output)).toBe(false);
});

test('OCCT archive content mismatch fails even with expected filename', () => {
  const options = fixture();
  writeFileSync(options.occtArchive, 'wrong bytes');
  expect(() => prepareMaterials(options)).toThrow('OCCT archive SHA-256 differs');
});

test('OCCT metadata cannot replace the actual pinned archive hash', () => {
  const options = fixture();
  const path = join(options.occtPrefix, 'share/onecad/occt-build.json');
  const data = occtSchema.parse(JSON.parse(readFileSync(path, 'utf8')));
  data.archiveSha256 = hash('wrong bytes');
  writeFileSync(path, json(data));
  writeFileSync(options.occtArchive, 'wrong bytes');
  expect(() => checkOcct(options.source, data, options.occtArchive)).toThrow('pinned build policy');
});

test('changed worker, manifest and source payload rejected', () => {
  const options = fixture();
  const archive = prepareMaterials(options);
  writeFileSync(options.worker, 'another binary');
  expect(() => verifyArchive(archive, options.worker)).toThrow('worker or worker manifest SHA-256');
  save(options.output, 'materials/source/worker/src/main.cpp', 'tampered source');
  expect(() => verifyDirectory(join(options.output, 'materials'), options.worker)).toThrow('inventory hash, size or mode mismatch');
});

test('missing or extra payload rejected', () => {
  const options = fixture();
  prepareMaterials(options);
  save(options.output, 'materials/unlisted', 'extra file');
  expect(() => verifyDirectory(join(options.output, 'materials'), options.worker)).toThrow('inventory file set differs');
});

test('source symlink rejected; outside output required; existing output preserved', () => {
  const options = fixture();
  expect(() => prepareMaterials({ ...options, output: join(options.source, 'output') })).toThrow('outside source repository');
  mkdirSync(options.output);
  expect(() => prepareMaterials(options)).toThrow('refusing to overwrite');
  rmSync(options.output, { recursive: true });
  symlinkSync(options.worker, join(options.source, 'linked-source'));
  expect(() => prepareMaterials(options)).toThrow('unsupported source type');
});

test('archive links rejected before extraction', () => {
  const options = fixture();
  prepareMaterials(options);
  symlinkSync('/etc/passwd', join(options.output, 'materials/link'));
  const archive = join(dirname(options.output), 'links.tar.gz');
  run(['tar', '-czf', archive, '-C', options.output, 'materials']);
  expect(() => verifyArchive(archive, options.worker)).toThrow('unsupported links or special files');
});


test('live build policy is the known pinned OCCT archive, not a filename assertion', () => {
  expect(occtPin(join(import.meta.dir, '../..')).archiveSha256)
    .toBe('0d6913eae4bcc09a3653ceced6dda1aec11c35a1513d4c06762c9b002092c68a');
});

test('mismatched OCCT CMake metadata fails closed', () => {
  const options = fixture();
  save(options.occtPrefix, 'share/onecad/occt-build.cmake', 'set(ONECAD_OCCT_BUILT_ID "wrong")\n');
  expect(() => prepareMaterials(options)).toThrow('OCCT CMake metadata differs');
});

test('missing dependency version evidence and mismatched worker manifest fail closed', () => {
  const options = fixture();
  save(options.boostPrefix, 'include/boost/version.hpp', '// no version macro\n');
  expect(() => prepareMaterials(options)).toThrow('missing Eigen or Boost version evidence');
  writeFileSync(options.worker, 'different worker bytes');
  expect(() => prepareMaterials(options)).toThrow('worker SHA-256 differs from worker manifest');
});

test('executable source modes preserved and checked', () => {
  const options = fixture();
  chmodSync(join(options.source, 'scripts/package-macos.sh'), 0o755);
  prepareMaterials(options);
  const script = join(options.output, 'materials/source/scripts/package-macos.sh');
  expect(Boolean(lstatSync(script).mode & 0o111)).toBe(true);
  chmodSync(script, 0o644);
  expect(() => verifyDirectory(join(options.output, 'materials'), options.worker)).toThrow('inventory hash, size or mode mismatch');
});

test('dangling untracked source links fail closed', () => {
  const options = fixture();
  symlinkSync('/no/such/onecad-material', join(options.source, 'dangling-source'));
  expect(() => prepareMaterials(options)).toThrow('unsupported source type');
});
