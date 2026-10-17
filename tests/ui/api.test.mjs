import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const source = await readFile(new URL('../../ui/api.js', import.meta.url), 'utf8');
async function api(invoke, key) {
  globalThis.window = { __TAURI__: { core: { invoke } } };
  return import(`data:text/javascript;base64,${Buffer.from(`${source}\n// ${key}`).toString('base64')}`);
}

test('typed command errors retain fields and render their message', async () => {
  const detail = { code: 'RecoveryRequired', phase: 'Write', operation_id: 'op', step_id: 'step', item_id: 'item', message: 'Recover the interrupted operation.', retryable: false };
  const { call } = await api(async () => { throw detail; }, 'typed');
  await assert.rejects(call('apply_actions'), error => {
    assert.match(String(error), /Recover the interrupted operation\./);
    for (const [key, value] of Object.entries(detail)) assert.equal(error[key], value);
    return true;
  });
});

test('legacy string errors remain readable', async () => {
  const { call } = await api(async () => { throw 'Drive disconnected'; }, 'legacy');
  await assert.rejects(call('start_scan'), error => {
    assert.match(String(error), /Drive disconnected/);
    return true;
  });
});

test('status error text and normal result payloads remain unchanged', async () => {
  const status = { running: false, acting: false, error: 'Recovery is required.', result_ready: true };
  const { call } = await api(async () => status, 'status');
  assert.strictEqual(await call('scan_status'), status);
  assert.equal((await call('scan_status')).error, 'Recovery is required.');
});
