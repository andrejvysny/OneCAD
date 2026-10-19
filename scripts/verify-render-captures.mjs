import { createHash } from 'node:crypto';
import { lstatSync, readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';

export function verifyCaptures(directory) {
  const files = readdirSync(directory).filter(name => name !== 'manifest.json').sort();
  const pngs = files.filter(name => name.endsWith('.png'));
  if (!pngs.length) throw new Error('renderer evidence contains no images');
  const records = [];
  for (const name of files) {
    if (!/^[a-zA-Z0-9_-]+\.(png|txt)$/.test(name)
        || !lstatSync(resolve(directory, name)).isFile()) throw new Error(`unsafe capture path: ${name}`);
    const peer = name.replace(/\.(png|txt)$/, name.endsWith('.png') ? '.txt' : '.png');
    if (!files.includes(peer)) throw new Error(`missing capture pair: ${name}`);
    const bytes = readFileSync(resolve(directory, name));
    if (!bytes.length) throw new Error(`empty capture evidence: ${name}`);
    records.push({ name, bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') });
  }
  const cases = new Set();
  const sources = new Set();
  for (const name of pngs) {
    const image = readFileSync(resolve(directory, name));
    if (image.length < 33 || image.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a'
        || image.subarray(12, 16).toString() !== 'IHDR'
        || image.readUInt32BE(16) === 0 || image.readUInt32BE(20) === 0) throw new Error(`invalid PNG: ${name}`);
    const metadata = readFileSync(resolve(directory, name.replace(/\.png$/, '.txt')), 'utf8');
    const source = metadata.match(/^source_commit: ([a-f0-9]{40})$/m)?.[1];
    if (!source || !metadata.includes('features:') || !metadata.includes('backend: Metal')
        || !metadata.includes('source_status:')) throw new Error(`missing capture provenance: ${name}`);
    cases.add(metadata.match(/^case: "([^"]+)"$/m)?.[1]);
    sources.add(source);
  }
  if (sources.size !== 1) throw new Error('capture source revisions differ');
  for (const depth of ['fallback', 'stencil']) {
    for (const projection of ['perspective', 'orthographic']) {
      for (const theme of ['dark', 'light']) {
        const name = `${depth}-${projection}-${theme}-shaded-edges`;
        if (!cases.has(name)) throw new Error(`missing renderer configuration: ${name}`);
      }
    }
  }
  return { source: [...sources][0], images: pngs.length, files: records };
}

if (import.meta.main) console.log(JSON.stringify(verifyCaptures(resolve(process.argv[2])), null, 2));
