/** Local source-material inventory; no publication or reproducibility attestation. */
import { createHash } from 'node:crypto';
import { existsSync, lstatSync, readFileSync, readdirSync, realpathSync } from 'node:fs';
import { basename, dirname, join, posix, relative, resolve } from 'node:path';
import { z } from 'zod';

export const hash = (bytes: Uint8Array | string): string =>
  createHash('sha256').update(bytes).digest('hex');
export const read = (file: string): Buffer => readFileSync(file);
export const requireFile = (file: string): Buffer => {
  if (!existsSync(file) || !lstatSync(file).isFile()) throw new Error(`missing regular material: ${file}`);
  const bytes = read(file);
  if (bytes.length === 0) throw new Error(`empty material: ${file}`);
  return bytes;
};
export const run = (args: string[], cwd?: string): Buffer => {
  const result = Bun.spawnSync(args, { cwd, stdout: 'pipe', stderr: 'pipe' });
  if (result.exitCode !== 0) throw new Error(`${args[0]} failed: ${result.stderr.toString().trim()}`);
  return Buffer.from(result.stdout);
};
export const git = (root: string, ...args: string[]): Buffer => run(['git', '-C', root, ...args]);
export const json = (value: unknown): string => `${JSON.stringify(value, null, 2)}\n`;
export const canonicalPath = (file: string): string => {
  const absolute = resolve(file);
  return existsSync(absolute) ? realpathSync(absolute) : join(canonicalPath(dirname(absolute)), basename(absolute));
};
export const inside = (parent: string, file: string): boolean => {
  const name = relative(resolve(parent), resolve(file));
  return name === '' || (!name.startsWith('../') && name !== '..' && !name.startsWith('/'));
};
export const safePath = (name: string): void => {
  if (!name || /[\x00-\x1f\x7f\\]/.test(name) || name.startsWith('/') ||
      name !== posix.normalize(name) || name.split('/').some(part => part === '..')) {
    throw new Error(`unsafe material path: ${JSON.stringify(name)}`);
  }
};
export const files = (root: string, prefix = ''): string[] => readdirSync(join(root, prefix))
  .sort().flatMap(name => {
    const path = posix.join(prefix, name);
    safePath(path);
    const stat = lstatSync(join(root, path));
    if (stat.isDirectory()) return files(root, path);
    if (!stat.isFile()) throw new Error(`unsupported material type (including symlink): ${path}`);
    return [path];
  });

const digest = z.string().regex(/^[0-9a-f]{64}$/);
export const workerSchema = z.object({
  manifestVersion: z.literal(1), binarySha256: digest,
  occt: z.object({ version: z.literal('8.0.1'), fingerprint: z.string().min(1) }),
}).passthrough();
export const occtSchema = z.object({
  schemaVersion: z.literal(1), version: z.literal('8.0.1'), tag: z.literal('V8_0_1'),
  sourceCommit: z.literal('b8f597c677811d1f9f4d8a97f5ae2825c0353a42'),
  archiveSha256: digest, buildId: z.string().regex(/^[A-Za-z0-9._+-]+$/),
  kernelPolicyVersion: z.literal(1), normalizedBuildOptions: z.string().min(1),
}).strict();
export const metadataSchema = z.object({
  schemaVersion: z.literal(1), preparedAt: z.string().datetime(),
  scope: z.literal('local-source-material-preparation'),
  source: z.object({ revision: z.string().regex(/^[0-9a-f]{40,64}$/), dirty: z.boolean(),
    exactTags: z.array(z.string()), snapshotPolicy: z.literal('tracked-and-untracked-nonignored'),
    fileCount: z.number().int().positive() }).strict(),
  worker: z.object({ binarySha256: digest, manifestSha256: digest }).strict(),
  occt: occtSchema,
  dependencies: z.object({ eigen: z.string().min(1), boost: z.string().min(1) }).strict(),
  validation: z.object({ buildProvenance: z.literal('unverified'),
    relink: z.literal('unverified'), publication: z.literal('unverified'),
    transitiveLicenseInventory: z.literal('incomplete') }).strict(),
}).strict();
export const inventorySchema = z.object({ schemaVersion: z.literal(1),
  files: z.array(z.object({ path: z.string(), sha256: digest, bytes: z.number().int().nonnegative(),
    executable: z.boolean() }).strict()) }).strict();
export type MaterialMetadata = z.infer<typeof metadataSchema>;
export type Inventory = z.infer<typeof inventorySchema>;

// Read the existing build policy; changing it requires changing the checked-in build script.
export const occtPin = (root: string): { archiveSha256: string; options: string } => {
  const script = requireFile(join(root, 'scripts/build-pinned-occt.sh')).toString();
  const branch = script.match(/8\.0\.1 \| V8_0_1\)([\s\S]*?);;/)?.[1] ?? '';
  const archiveSha256 = branch.match(/ARCHIVE_SHA256="([0-9a-f]{64})"/)?.[1];
  const options = script.match(/readonly NORMALIZED_OPTIONS="([^"]+)"/)?.[1];
  if (!archiveSha256 || !options || !branch.includes('b8f597c677811d1f9f4d8a97f5ae2825c0353a42')) {
    throw new Error('missing pinned OCCT 8.0.1 policy in source snapshot');
  }
  return { archiveSha256, options };
};
export const checkOcctCmake = (metadata: z.infer<typeof occtSchema>, file: string): void => {
  const cmake = requireFile(file).toString();
  const expected = {
    ONECAD_OCCT_SOURCE_COMMIT: metadata.sourceCommit,
    ONECAD_OCCT_NORMALIZED_BUILD_OPTIONS: metadata.normalizedBuildOptions,
    ONECAD_OCCT_BUILT_ID: metadata.buildId,
  };
  for (const [key, value] of Object.entries(expected)) {
    const matches = [...cmake.matchAll(new RegExp(`^set\\(${key} "([^"\\n]+)"\\)$`, 'gm'))];
    if (matches.length !== 1 || matches[0][1] !== value) throw new Error(`OCCT CMake metadata differs: ${key}`);
  }
};
export const checkOcct = (root: string, metadata: z.infer<typeof occtSchema>, archive: string): void => {
  const pin = occtPin(root);
  if (metadata.archiveSha256 !== pin.archiveSha256 || hash(requireFile(archive)) !== pin.archiveSha256) {
    throw new Error('OCCT archive SHA-256 differs from pinned build policy');
  }
  if (metadata.normalizedBuildOptions !== pin.options) throw new Error('OCCT build options differ from pinned policy');
};
export const requiredSource = [
  'LICENSE-MIT', 'LICENSE-APACHE', 'THIRD_PARTY_NOTICES', 'ATTRIBUTION.md',
  'package.json', 'bun.lock', 'crates/onecad-ui/Cargo.toml',
  'assets/native/tokens.css', 'assets/native/cppIcons.generated.ts', 'assets/native/authored.ts',
  'worker/CMakeLists.txt', 'worker/cmake/Dependencies.cmake',
  'worker/third_party/planegcs/LICENSE', 'worker/third_party/planegcs/VENDOR.txt',
  'worker/third_party/planegcs/GCS.cpp', 'worker/third_party/planegcs/CMakeLists.txt',
  'worker/third_party/nlohmann/json.hpp', 'worker/third_party/nlohmann/json_fwd.hpp',
  'worker/third_party/nlohmann/VENDOR.txt', 'scripts/build-worker.sh',
  'scripts/build-pinned-occt.sh', 'scripts/occt-fingerprint.sh',
  'scripts/package-macos.sh', 'scripts/bundle-dylibs.sh', 'docs/PACKAGING.md',
];
export const checkSource = (root: string): void => {
  requiredSource.forEach(name => requireFile(join(root, name)));
  const cargoRoot = root;
  ['Cargo.toml', 'Cargo.lock'].forEach(name => requireFile(join(cargoRoot, name)));
  for (const directory of ['crates/onecad-app/src', 'crates/onecad-ui/src', 'crates/onecad-render/src', 'worker/src', 'worker/third_party/planegcs']) {
    if (!existsSync(join(root, directory)) || files(join(root, directory)).length === 0) {
      throw new Error(`missing source directory: ${directory}`);
    }
  }
};
export const archiveNames = (archive: string): string[] => {
  const names = run(['tar', '-tzf', archive]).toString().trim().split('\n');
  const seen = new Set<string>();
  for (const raw of names) {
    const name = raw.replace(/\/$/, '');
    safePath(name);
    if (name !== 'materials' && !name.startsWith('materials/')) throw new Error('archive has unexpected root');
    if (seen.has(name)) throw new Error(`duplicate archive path: ${name}`);
    seen.add(name);
  }
  const listing = run(['tar', '-tvzf', archive]).toString().trim().split('\n');
  if (listing.some(line => !line.startsWith('-') && !line.startsWith('d'))) {
    throw new Error('archive contains unsupported links or special files');
  }
  return names;
};
export const parseArgs = (argv: string[], required: string[]): Record<string, string> => {
  const args: Record<string, string> = {};
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    if (!required.includes(name) || args[name] || !argv[index + 1] || argv[index + 1].startsWith('--')) {
      throw new Error(`expected options: ${required.join(' ')}`);
    }
    args[name] = resolve(argv[index + 1]);
  }
  for (const name of required) if (!args[name]) throw new Error(`required option: ${name}`);
  return args;
};
