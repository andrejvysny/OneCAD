#!/usr/bin/env node
// Adapted from rust-desktop-app xtask/src/layers.rs; see ATTRIBUTION.md.
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const LAYERS = Object.freeze({
  'onecad-core': [],
  'onecad-protocol': [],
  'onecad-library': [],
  'onecad-app': ['onecad-core', 'onecad-protocol', 'onecad-library'],
  'onecad-scene': ['onecad-core', 'onecad-protocol'],
  'onecad-interaction': ['onecad-app', 'onecad-scene', 'onecad-core'],
  'onecad-render': ['onecad-scene', 'onecad-protocol', 'onecad-core'],
  'onecad-ui': [
    'onecad-app', 'onecad-scene', 'onecad-interaction', 'onecad-render',
    'onecad-core', 'onecad-protocol', 'onecad-library',
  ],
  'onecad-mesh-fixture': ['onecad-app', 'onecad-core', 'onecad-protocol'],
  'onecad-regen': ['onecad-app', 'onecad-core'],
  'onecad-library-ingest': ['onecad-app'],
  'onecad-kernelbench': [],
  'onecad-worker-stub': ['onecad-protocol'],
});

const HOSTS = new Set(['onecad-ui']);
const TOOLKITS = [
  'tauri', 'egui', 'eframe', 'epaint', 'emath', 'ecolor', 'winit', 'rfd',
  'bevy', 'arboard', 'muda', 'tray-icon', 'objc', 'objc2', 'cocoa', 'core-graphics',
  'core-text', 'windows', 'gtk', 'gtk4', 'gdk', 'gdk4', 'gdk-pixbuf', 'gio',
  'glib', 'gobject', 'gsk4', 'pango', 'iced', 'slint', 'i-slint', 'fltk', 'wry',
  'web-sys', 'js-sys', 'wasm-bindgen', 'accesskit', 'sdl2', 'dioxus',
];
const GPU = ['wgpu', 'naga', 'ash', 'vulkano', 'vulkan', 'metal', 'glow', 'glutin', 'glium'];
const NETWORK = ['reqwest', 'hyper', 'ureq', 'isahc', 'surf', 'curl', 'tonic'];
const family = (name, prefixes) => prefixes.some((prefix) =>
  name === prefix || name.startsWith(`${prefix}-`) || name.startsWith(`${prefix}_`));

function requireArray(value, label) {
  if (!Array.isArray(value)) throw new Error(`metadata: missing ${label} array`);
  return value;
}

function requireString(value, label) {
  if (typeof value !== 'string' || !value) throw new Error(`metadata: missing ${label}`);
  return value;
}

function workspacePackages(metadata) {
  const packages = requireArray(metadata.packages, 'packages');
  const members = requireArray(metadata.workspace_members, 'workspace_members');
  if (!members.length) throw new Error('metadata: empty workspace_members');
  const result = members.map((id) => {
    const matches = packages.filter((pkg) => pkg.id === id);
    if (matches.length !== 1) throw new Error(`metadata: workspace member ${id} must have one package`);
    const pkg = matches[0];
    requireString(pkg.name, 'package name');
    requireString(pkg.manifest_path, 'manifest_path');
    requireArray(pkg.dependencies, `${pkg.name} dependencies`);
    return pkg;
  });
  if (new Set(result.map((pkg) => pkg.name)).size !== result.length) {
    throw new Error('metadata: duplicate workspace package names');
  }
  return result;
}

function checkDependency(pkg, dep, packages) {
  const name = requireString(dep.name, 'dependency name');
  const kind = dep.kind ?? 'normal';
  if (!['normal', 'dev', 'build'].includes(kind)) throw new Error(`metadata: unknown dependency kind ${kind}`);
  const context = `${pkg.name} -> ${name} (${kind}${dep.target ? `, ${dep.target}` : ''}${dep.optional ? ', optional' : ''}${dep.rename ? `, renamed ${dep.rename}` : ''})`;
  const target = packages.find((entry) => entry.name === name);
  if (dep.path != null) {
    requireString(dep.path, 'dependency path');
    if (!target || resolve(dep.path) !== resolve(dirname(target.manifest_path))) {
      return `${context}: local dependency is not a workspace member at its declared path`;
    }
    if (!LAYERS[pkg.name].includes(name)) return `${context}: workspace dependency forbidden`;
  } else if (target || Object.hasOwn(LAYERS, name) || name.startsWith('onecad-')) {
    return `${context}: OneCAD dependency must point to a registered workspace member`;
  }
  if (!HOSTS.has(pkg.name) && family(name, TOOLKITS)) return `${context}: UI/platform toolkit forbidden`;
  // Naga validates the renderer's WGSL without opening a device in unit tests.
  const rendererGpu = pkg.name === 'onecad-render'
    && (family(name, ['wgpu']) || (name === 'naga' && kind === 'dev'));
  if (!HOSTS.has(pkg.name) && family(name, GPU) && !rendererGpu) {
    return `${context}: GPU dependency forbidden`;
  }
  if (pkg.name === 'onecad-library' && family(name, NETWORK)) return `${context}: network dependency forbidden`;
  return null;
}

export function checkMetadata(metadata) {
  const packages = workspacePackages(metadata);
  const violations = [];
  for (const pkg of packages) {
    if (!Object.hasOwn(LAYERS, pkg.name)) {
      violations.push(`${pkg.name}: unknown workspace package; register it in scripts/check-rust-layers.mjs`);
      continue;
    }
    for (const dep of pkg.dependencies) {
      const violation = checkDependency(pkg, dep, packages);
      if (violation) violations.push(violation);
    }
  }
  return [...new Set(violations)].sort();
}

function main() {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const result = spawnSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], {
    cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr.trim() || `cargo metadata exited ${result.status}`);
  const metadata = JSON.parse(result.stdout);
  const violations = checkMetadata(metadata);
  if (violations.length) throw new Error(`Rust dependency layering failed:\n${violations.join('\n')}`);
  console.log(`Rust layers: ${metadata.workspace_members.length} workspace packages, no violations`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
