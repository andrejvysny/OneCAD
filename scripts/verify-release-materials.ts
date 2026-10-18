/** Verify local release materials against the final worker bytes. */
import { existsSync, lstatSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { archiveNames, checkOcct, checkOcctCmake, checkSource, files, hash, inventorySchema, json,
  metadataSchema, parseArgs, read, requireFile, run, safePath, workerSchema } from './release-materials';

export const verifyDirectory = (root: string, worker: string): void => {
  const inventory = inventorySchema.parse(JSON.parse(requireFile(join(root, 'inventory.json')).toString()));
  const actual = files(root).filter(name => name !== 'inventory.json').sort();
  const listed = inventory.files.map(entry => entry.path);
  if (new Set(listed).size !== listed.length || json([...listed].sort()) !== json(actual)) {
    throw new Error('inventory file set differs from payload');
  }
  for (const entry of inventory.files) {
    safePath(entry.path);
    const path = join(root, entry.path);
    const bytes = read(path);
    if (hash(bytes) !== entry.sha256 || bytes.length !== entry.bytes ||
        Boolean(lstatSync(path).mode & 0o111) !== entry.executable) {
      throw new Error(`inventory hash, size or mode mismatch: ${entry.path}`);
    }
  }
  const metadata = metadataSchema.parse(JSON.parse(requireFile(join(root, 'release-materials.json')).toString()));
  const manifestPath = join(root, 'worker-manifest.json');
  const manifest = workerSchema.parse(JSON.parse(requireFile(manifestPath).toString()));
  if (hash(requireFile(worker)) !== metadata.worker.binarySha256 ||
      manifest.binarySha256 !== metadata.worker.binarySha256 ||
      hash(read(manifestPath)) !== metadata.worker.manifestSha256) {
    throw new Error('worker or worker manifest SHA-256 differs from release materials');
  }
  const source = join(root, 'source');
  checkSource(source);
  if (files(source).length !== metadata.source.fileCount) throw new Error('source file count differs');
  checkOcct(source, metadata.occt, join(root, 'occt/OCCT-V8_0_1.tar.gz'));
  checkOcctCmake(metadata.occt, join(root, 'occt/occt-build.cmake'));
  const recordedOcct = JSON.parse(requireFile(join(root, 'occt/occt-build.json')).toString()) as unknown;
  if (json(recordedOcct) !== json(metadata.occt)) throw new Error('OCCT metadata differs from inventory');
  for (const name of ['RELINK.md', 'source-status.z', 'source-changes.patch', 'occt/occt-build.cmake',
    'licenses/OCCT-LICENSE_LGPL_21.txt', 'licenses/OCCT-LGPL_EXCEPTION.txt',
    'licenses/Boost-LICENSE_1_0.txt', 'dependencies/eigen-version.cmake', 'dependencies/boost-version.hpp',
    'licenses/Eigen-COPYING.README', 'licenses/Eigen-COPYING.MPL2', 'licenses/Eigen-COPYING.BSD',
    'licenses/Eigen-COPYING.APACHE', 'licenses/Eigen-COPYING.MINPACK']) {
    // A clean snapshot legitimately has empty status and patch files.
    if (name.startsWith('source-')) {
      if (!existsSync(join(root, name))) throw new Error(`missing source evidence: ${name}`);
    } else requireFile(join(root, name));
  }
  const dirty = read(join(root, 'source-status.z')).length !== 0;
  if (dirty !== metadata.source.dirty) throw new Error('dirty state differs from source status evidence');
};

export const verifyArchive = (archive: string, worker: string): void => {
  archiveNames(archive);
  const temporary = mkdtempSync(join(tmpdir(), 'onecad-materials-verify-'));
  try {
    run(['tar', '-xzf', archive, '-C', temporary]);
    verifyDirectory(join(temporary, 'materials'), worker);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
};

if (import.meta.main) {
  try {
    const args = parseArgs(process.argv.slice(2), ['--archive', '--worker']);
    verifyArchive(args['--archive'], args['--worker']);
    console.log('Release material hashes and required files verified; build, relink and publication remain unverified.');
  } catch (error) {
    console.error(`verify-release-materials: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
