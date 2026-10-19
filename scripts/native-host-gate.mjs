#!/usr/bin/env bun
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

export const hostTests = [
  'occluded_app_logic_delivers_new_and_protects_close_without_ui',
  'document_lifecycle_tests::recovered_document_remains_saveable_and_protected_after_capture_refusal',
  'document_lifecycle_tests::late_clean_display_cannot_replace_authoritative_dirty_lifecycle',
  'document_lifecycle_tests::discard_quit_cleans_recovery_while_normal_stop_retains_dirty_recovery',
  'overlay_routing_tests::overlays_preserve_sketch_draft_selection_and_history_through_raw_replay',
  'diagnostic_delivery_tests::file_failure_callbacks_remain_visible_until_retry_or_document_replacement',
  'diagnostic_delivery_tests::rejected_sketch_mutation_and_worker_failure_reach_editor_diagnostics',
  'diagnostic_delivery_tests::stale_same_runtime_model_terminal_does_not_replace_current_diagnostic',
  'dimension_delivery_tests::circle_dimension_modal_edits_existing_id_and_real_solver_radius',
  'plate_delivery_tests::from_new_plate_constraints_edit_repair_history_finish_and_extrude',
  'numeric_delivery_tests::routed_tab_numeric_fields_enter_once_and_ime_cancel_preserve_authority',
  'profile_delivery_tests::finish_refusal_reenters_fresh_service_visit_for_repair',
  'selection_delivery_tests::marquee_survives_redraw_and_cancels_when_camera_changes',
  'point_center_delivery_tests::point_tab_before_pointer_signed_zero_and_repeated_construction_clicks',
  'point_center_delivery_tests::rectangle_method_chooser_center_numeric_full_dimensions_and_history',
  'point_center_delivery_tests::numeric_modal_refuses_enter_and_reopen_after_viewport_failure',
  'gesture_delivery_tests::viewport_corner_drag_early_release_commits_one_edit_and_local_history',
  'gesture_delivery_tests::viewport_radius_drag_and_escape_preserve_curve_identity_and_authority',
  'gesture_delivery_tests::viewport_line_body_drag_and_camera_change_cancel_without_edit',
  'gesture_delivery_tests::viewport_camera_change_before_drag_threshold_rejects_stale_start',
  'gesture_delivery_tests::viewport_escape_before_drag_threshold_cancels_only_pointer_owner',
  'gesture_delivery_tests::stationary_pointer_release_completes_on_service_repaint_wakes',
  'arc_gesture_delivery_tests::prepared_arc_center_drag_translates_circle_and_preserves_local_history',
  'arc_gesture_delivery_tests::prepared_arc_endpoint_drag_preserves_roles_and_cancel_keeps_authority',
  'arc_gesture_delivery_tests::prepared_arc_accepted_end_capture_failure_preserves_history_and_blocks_stale_authoring',
].map(name => `app_logic_tests::${name}`);
export const missingWorkerTest = hostTests[1];

function requireExactNames(names, expected) {
  if (JSON.stringify([...names].sort()) !== JSON.stringify([...expected].sort())) {
    throw new Error('required native host test inventory changed, duplicated, or incomplete');
  }
}

export function verifyHostInventory(output) {
  const names = output.split(/\r?\n/).filter(line => line.endsWith(': test')).map(line => line.slice(0, -6));
  requireExactNames(names, hostTests);
}

function executionRecords(output) {
  const starts = [...output.matchAll(/^test (\S+) \.\.\. /gm)];
  return starts.map((start, index) => {
    const endings = [starts[index + 1]?.index, output.indexOf('\ntest result:', start.index), output.indexOf('\nfailures:', start.index)]
      .filter(end => end !== undefined && end > start.index);
    const end = endings.length ? Math.min(...endings) : undefined;
    return { name: start[1], text: output.slice(start.index, end).trim() };
  });
}

export function verifyHostOutput(output) {
  const records = executionRecords(output);
  requireExactNames(records.map(record => record.name), hostTests);
  for (const record of records) {
    if (!/(?:\.\.\. |\n)ok$/.test(record.text)) throw new Error(`required test did not pass: ${record.name}`);
  }
  const summaries = [...output.matchAll(/^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored;/gm)];
  if (summaries.length !== 1 || summaries[0][1] !== 'ok' || Number(summaries[0][2]) !== hostTests.length
      || summaries[0][3] !== '0' || summaries[0][4] !== '0') {
    throw new Error(`native host gate must run exactly ${hostTests.length} tests with no skips`);
  }
  if ((output.match(/Native host gate adapter: .*backend: Metal/g) ?? []).length < hostTests.length) {
    throw new Error('required Metal adapter evidence missing');
  }
}

export function verifyMissingWorkerOutput(status, output) {
  const records = executionRecords(output);
  requireExactNames(records.map(record => record.name), [missingWorkerTest]);
  if (status === 0 || !output.includes('native host gate requires existing worker')
      || !/(?:\.\.\. |\n)FAILED$/.test(records[0].text)
      || !/^test result: FAILED\. 0 passed; 1 failed; 0 ignored;/m.test(output)) {
    throw new Error('missing-worker negative check did not execute and fail as required');
  }
}

function command(directory, name, program, args, env = process.env, requireSuccess = true) {
  const result = spawnSync(program, args, { env, encoding: 'utf8', timeout: 600_000, maxBuffer: 16 * 1024 * 1024 });
  const output = `${result.stdout ?? ''}${result.stderr ?? ''}`;
  writeFileSync(resolve(directory, `${name}.log`), output);
  const environment = Object.fromEntries([
    'ONECAD_REQUIRE_WORKER', 'ONECAD_REQUIRE_GPU', 'ONECAD_WORKER_PATH', 'WGPU_BACKEND',
    'CARGO_BUILD_JOBS', 'CARGO_TARGET_DIR', 'CARGO_PROFILE_TEST_DEBUG', 'CARGO_INCREMENTAL',
  ].filter(key => env[key] !== undefined).map(key => [key, env[key]]));
  writeFileSync(resolve(directory, `${name}.command.json`), JSON.stringify({ program, args, environment, status: result.status, signal: result.signal }, null, 2));
  process.stdout.write(output);
  if (result.error) throw result.error;
  if (requireSuccess && result.status !== 0) throw new Error(`${name} failed with status ${result.status}`);
  return { output, status: result.status };
}

function verifyWorker(directory, run) {
  const worker = resolve(process.env.ONECAD_WORKER_PATH ?? '');
  if (!process.env.ONECAD_WORKER_PATH) throw new Error('explicit ONECAD_WORKER_PATH required');
  const manifest = JSON.parse(readFileSync('packaging/binaries/onecad-worker-manifest.json', 'utf8'));
  const hash = createHash('sha256').update(readFileSync(worker)).digest('hex');
  if (manifest.binarySha256 !== hash || manifest.occt.version !== '8.0.1'
      || manifest.occt.fingerprint !== '0a6a1dce34181289') throw new Error('staged worker manifest mismatch');
  writeFileSync(resolve(directory, 'worker-manifest.json'), JSON.stringify(manifest, null, 2));
  const selftest = run('worker-selftest', worker, ['--selftest'], { ...process.env, ONECAD_WORKER_LOG: 'debug' }).output;
  for (const expected of ['selftest OK (occt 8.0.1,', 'build onecad-occt-8.0.1-b8f597c67781-kp1,', 'fingerprint 0a6a1dce34181289']) {
    if (!selftest.includes(expected)) throw new Error(`worker selftest missing ${expected}`);
  }
  return { worker, hash };
}

function main() {
  const directory = resolve(process.argv[2] ?? '');
  if (!process.argv[2]) throw new Error('usage: bun scripts/native-host-gate.mjs OUTPUT_DIRECTORY');
  mkdirSync(directory, { recursive: true });
  rmSync(resolve(directory, 'acceptance.json'), { force: true });
  const run = (name, program, args, env, success) => command(directory, name, program, args, env, success);
  const source = run('source', 'git', ['rev-parse', 'HEAD']).output.trim();
  run('source-status', 'git', ['status', '--porcelain=v1']);
  run('toolchain', 'rustc', ['-Vv']);
  run('cargo', 'cargo', ['-V']);
  run('platform', 'sw_vers', []);
  const { worker, hash } = verifyWorker(directory, run);
  const env = { ...process.env, ONECAD_REQUIRE_WORKER: '1', ONECAD_REQUIRE_GPU: '1', ONECAD_WORKER_PATH: worker };
  const args = ['test', '--locked', '-p', 'onecad-ui', '--lib', 'app_logic_tests::', '--'];
  const inventory = run('inventory', 'cargo', [...args, '--ignored', '--list'], env).output;
  verifyHostInventory(inventory);
  const accepted = run('lifecycle', 'cargo', [...args, '--ignored', '--nocapture', '--test-threads=1'], env);
  verifyHostOutput(accepted.output);
  const negative = run('missing-worker', 'cargo', [
    'test', '--locked', '-p', 'onecad-ui', '--lib', missingWorkerTest, '--', '--exact', '--ignored', '--nocapture', '--test-threads=1',
  ], { ...env, ONECAD_WORKER_PATH: resolve(directory, 'missing-worker') }, false);
  verifyMissingWorkerOutput(negative.status, negative.output);
  writeFileSync(resolve(directory, 'acceptance.json'), JSON.stringify({
    classification: 'automated native-host lifecycle with production OCCT worker and Metal; no physical input or AppKit teardown qualification',
    source, workerSha256: hash, tests: hostTests, testCount: hostTests.length,
    inventoryVerified: true, executedExactlyOnce: true, missingWorkerRejected: true,
  }, null, 2));
}

if (import.meta.main) main();
