import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { checkMetadata, LAYERS } from '../check-rust-layers.mjs';

function fixture(from, dependencies = [], extra = []) {
  const packages = [from, ...extra].map((name) => ({
    id: `path+file:///fixture/${name}#0.1.0`, name,
    manifest_path: `/fixture/${name}/Cargo.toml`,
    dependencies: name === from ? dependencies : [],
  }));
  return { packages, workspace_members: packages.map((pkg) => pkg.id) };
}

const local = (name, fields = {}) => ({ name, path: `/fixture/${name}`, kind: null, ...fields });
const external = (name, fields = {}) => ({ name, source: 'registry+https://github.com/rust-lang/crates.io-index', kind: null, ...fields });

test('vendored viewport paths stay confined to the renderer', () => {
  for (const [name, folder] of [['solidcraft-gpu', 'gpu'], ['solidcraft-render', 'render']]) {
    const path = fileURLToPath(new URL(`../../vendor/solidcraft-viewport/crates/${folder}`, import.meta.url));
    const dependency = { name, path, kind: null };
    assert.deepEqual(checkMetadata(fixture('onecad-render', [dependency])), []);
    for (const from of ['onecad-app', 'onecad-core', 'onecad-scene', 'onecad-ui']) {
      assert.equal(checkMetadata(fixture(from, [dependency])).length, 1);
    }
    for (const fields of [{ path: '/outside/vendor' }, { kind: 'build' }, { kind: 'dev' }]) {
      assert.equal(checkMetadata(fixture('onecad-render', [{ ...dependency, ...fields }])).length, 1);
    }
  }
});

test('each explicit workspace edge passes for every dependency kind', () => {
  for (const [from, allowed] of Object.entries(LAYERS)) {
    for (const to of allowed) {
      for (const kind of [null, 'dev', 'build']) {
        assert.deepEqual(checkMetadata(fixture(from, [local(to, { kind })], [to])), []);
      }
    }
  }
});

test('upward, concrete interaction and host dependencies fail across dependency kinds', () => {
  for (const [from, to] of [
    ['onecad-core', 'onecad-app'], ['onecad-app', 'onecad-interaction'],
    ['onecad-scene', 'onecad-render'], ['onecad-render', 'onecad-ui'],
    ['onecad-regen', 'onecad'], ['onecad-library-ingest', 'onecad'],
  ]) {
    for (const kind of [null, 'dev', 'build']) {
      assert.match(checkMetadata(fixture(from, [local(to, { kind })], [to]))[0], /workspace dependency forbidden/);
    }
  }
});

test('toolkit and GPU families fail in all pure layers, all declaration kinds', () => {
  for (const from of Object.keys(LAYERS).filter((name) => !['onecad', 'onecad-ui'].includes(name))) {
    for (const name of ['tauri', 'tauri-build', 'egui-wgpu', 'eframe', 'winit', 'objc2-app-kit', 'gtk4', 'slint', 'wgpu']) {
      if (from === 'onecad-render' && name === 'wgpu') continue;
      for (const kind of [null, 'dev', 'build']) {
        const violations = checkMetadata(fixture(from, [external(name, { kind })]));
        assert.equal(violations.length, 1, `${from} -> ${name} (${kind})`);
      }
    }
  }
});

test('renames, optional declarations and inactive targets cannot hide forbidden dependencies', () => {
  for (const kind of [null, 'dev', 'build']) {
    const dep = external('egui', { rename: 'safe_math', optional: true, kind, target: 'cfg(target_os = "windows")' });
    const [violation] = checkMetadata(fixture('onecad-app', [dep]));
    assert.match(violation, /onecad-app -> egui/);
    assert.match(violation, /renamed safe_math/);
    assert.match(violation, /optional/);
    assert.match(violation, /target_os = "windows"/);
  }
  assert.match(checkMetadata(fixture('onecad-app', [local('onecad-interaction', { rename: 'safe' })], ['onecad-interaction']))[0], /workspace dependency forbidden/);
});

test('render accepts wgpu without admitting toolkit or alternate GPU stacks', () => {
  assert.deepEqual(checkMetadata(fixture('onecad-render', [external('wgpu'), external('wgpu-types')])), []);
  for (const name of ['egui', 'egui-wgpu', 'naga', 'metal', 'ash']) {
    assert.equal(checkMetadata(fixture('onecad-render', [external(name)])).length, 1);
  }
});

test('composition hosts accept external UI and GPU dependencies', () => {
  for (const from of ['onecad', 'onecad-ui']) {
    assert.deepEqual(checkMetadata(fixture(from, [external('tauri-build'), external('egui'), external('wgpu')])), []);
  }
});

test('only renderer development dependencies admit the Naga shader validator', () => {
  assert.deepEqual(checkMetadata(fixture('onecad-render', [external('naga', { kind: 'dev' })])), []);
  for (const kind of [null, 'build']) {
    assert.equal(checkMetadata(fixture('onecad-render', [external('naga', { kind })])).length, 1);
  }
  for (const from of ['onecad-app', 'onecad-scene', 'onecad-interaction', 'onecad-core']) {
    assert.equal(checkMetadata(fixture(from, [external('naga', { kind: 'dev' })])).length, 1);
  }
});

test('external local crates and mismatched workspace paths fail closed', () => {
  for (const dep of [local('hidden-ui'), local('onecad-core', { path: '/outside/onecad-core' })]) {
    assert.match(checkMetadata(fixture('onecad-app', [dep], ['onecad-core']))[0], /local dependency is not a workspace member/);
  }
  assert.match(checkMetadata(fixture('onecad-app', [external('onecad-core')], ['onecad-core']))[0], /must point to a registered workspace member/);
});

test('unknown workspace crates fail even with no dependencies', () => {
  assert.match(checkMetadata(fixture('onecad-surprise'))[0], /unknown workspace package/);
});

test('library keeps network access outside its boundary', () => {
  assert.match(checkMetadata(fixture('onecad-library', [external('reqwest', { kind: 'dev' })]))[0], /network dependency forbidden/);
});

test('malformed or incomplete metadata fails instead of silently passing', () => {
  for (const metadata of [{}, { packages: [], workspace_members: [] }, { packages: [], workspace_members: ['missing'] }]) {
    assert.throws(() => checkMetadata(metadata), /metadata:/);
  }
  const metadata = fixture('onecad-app', [external('serde', { kind: 'surprise' })]);
  assert.throws(() => checkMetadata(metadata), /unknown dependency kind/);
});
