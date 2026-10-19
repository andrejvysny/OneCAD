import { expect, test } from 'bun:test';
import { hostTests, missingWorkerTest, verifyHostInventory, verifyHostOutput, verifyMissingWorkerOutput } from '../native-host-gate.mjs';

const count = hostTests.length;
const summary = `test result: ok. ${count} passed; 0 failed; 0 ignored; 12 filtered out; finished in 1s\n`;
const records = hostTests.map(name => `test ${name} ... Native host gate adapter: AdapterInfo { backend: Metal }\nok\n`);
const accepted = records.join('') + summary;
const inventory = hostTests.map(name => `${name}: test\n`).join('');

test('inventory and execution require every declared host test exactly once', () => {
  expect(count).toBe(25);
  expect(() => verifyHostInventory(inventory)).not.toThrow();
  expect(() => verifyHostOutput(accepted)).not.toThrow();
  for (const output of [inventory + `${hostTests[0]}: test\n`, inventory.replace(hostTests[0], hostTests[1]), inventory.replace(hostTests[0], 'unknown')]) {
    expect(() => verifyHostInventory(output)).toThrow();
  }
});

test('zero tests, skips, wrong case, failed case, and missing Metal evidence fail closed', () => {
  for (const output of [
    'test result: ok. 0 passed; 0 failed; 0 ignored;',
    accepted.replace(`${count} passed; 0 failed; 0 ignored;`, `${count - 1} passed; 0 failed; 1 ignored;`),
    accepted.replace(hostTests[0], 'unrelated_test'),
    accepted.replace('\nok\n', '\nFAILED\n'),
    accepted.replace('backend: Metal', 'backend: Vulkan'),
  ]) expect(() => verifyHostOutput(output)).toThrow();
});

test('duplicate, missing, unexpected, and repeated-suite executions cannot pass', () => {
  for (const output of [
    records[0] + accepted,
    accepted.replace(records[0], records[1]),
    accepted.replace(records[0], ''),
    accepted.replace(hostTests[0], 'app_logic_tests::unexpected'),
    accepted + summary,
  ]) expect(() => verifyHostOutput(output)).toThrow();
});

test('separate stderr adapter evidence and repeated app setup remain valid', () => {
  const stdout = hostTests.map(name => `test ${name} ... ok\n`).join('') + summary;
  const stderr = 'Native host gate adapter: AdapterInfo { backend: Metal }\n'.repeat(count + 2);
  expect(() => verifyHostOutput(stdout + stderr)).not.toThrow();
});

test('missing worker requires exactly the selected failed test and decisive reason', () => {
  const failed = `test ${missingWorkerTest} ... FAILED\n\nfailures:\n\nfailures:\n    ${missingWorkerTest}\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;\nnative host gate requires existing worker\n`;
  expect(() => verifyMissingWorkerOutput(101, failed)).not.toThrow();
  for (const [status, output] of [
    [0, failed], [101, 'compile failed'], [101, 'native host gate requires existing worker'],
    [101, failed.replaceAll(missingWorkerTest, 'app_logic_tests::wrong_case')],
    [101, `test ${missingWorkerTest} ... FAILED\n` + failed],
  ]) expect(() => verifyMissingWorkerOutput(status, output)).toThrow();
});
