export function formatExactBytes(decimalString) {
  if (decimalString == null) return '—';
  if (typeof decimalString !== 'string' || !/^(0|[1-9][0-9]*)$/.test(decimalString)) throw new Error('Invalid exact byte count');
  return `${BigInt(decimalString).toLocaleString('en-US')} bytes`;
}
