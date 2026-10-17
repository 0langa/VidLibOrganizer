import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatExactBytes } from '../../ui/exact.js';
test('exact byte formatting preserves values above JavaScript safe integer and rejects rounded numbers', () => {
  assert.equal(formatExactBytes('9007199254740993'), '9,007,199,254,740,993 bytes');
  assert.throws(() => formatExactBytes(9007199254740992), /Invalid exact byte count/);
  assert.throws(() => formatExactBytes('01'), /Invalid exact byte count/);
  assert.equal(formatExactBytes(null), '—');
});
