import { expect, test } from 'bun:test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { verifyCaptures } from '../verify-render-captures.mjs';

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'onecad-artifact-test-'));
  const png = Buffer.from('89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489', 'hex');
  for (const depth of ['fallback', 'stencil']) for (const projection of ['perspective', 'orthographic']) for (const theme of ['dark', 'light']) {
    const name = `${depth}-${projection}-${theme}-shaded-edges`;
    writeFileSync(join(root, `${name}.png`), png);
    writeFileSync(join(root, `${name}.txt`), `case: "${name}"\nfeatures: test\nadapter: backend: Metal\nsource_commit: ${'a'.repeat(40)}\nsource_status:\n`);
  }
  return root;
}

test('portable image pairs retain complete configurations and stable download hashes', () => {
  const root = fixture();
  try {
    const manifest = verifyCaptures(root);
    writeFileSync(join(root, 'manifest.json'), JSON.stringify(manifest));
    expect(verifyCaptures(root)).toEqual(manifest);
    expect(manifest.images).toBe(8);
    expect(manifest.files).toHaveLength(16);
  } finally { rmSync(root, { recursive: true }); }
});

test('unsafe names, missing metadata and missing configuration fail closed', () => {
  for (const change of [
    root => writeFileSync(join(root, 'invalid:name.txt'), 'bad'),
    root => rmSync(join(root, 'fallback-perspective-dark-shaded-edges.txt')),
    root => { for (const extension of ['png', 'txt']) rmSync(join(root, `fallback-perspective-dark-shaded-edges.${extension}`)); },
    root => writeFileSync(join(root, 'fallback-perspective-dark-shaded-edges.png'), 'not PNG'),
  ]) {
    const root = fixture();
    try { change(root); expect(() => verifyCaptures(root)).toThrow(); }
    finally { rmSync(root, { recursive: true }); }
  }
});
