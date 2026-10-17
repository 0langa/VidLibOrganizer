import { el } from '../dom.js';
import { controls } from './rules.js';
/** The portable recipe is the authoritative metadata state. Opening is read only. */
export function metadataFields(host, recipe, change, signal) {
  const c = controls(host, signal), changes = structuredClone(recipe.metadata_changes);
  host.append(el('p', '', 'Keep leaves fields unchanged. Times are exact epoch nanoseconds; offsets are nanoseconds. Increment uses all selected primary items in frozen order, independently of file/directory numbering; linked companions inherit their owner’s ordinal.'));
  for (const [key, bit] of [['Hidden',2],['Read only',1],['Archive',32],['System',4]]) {
    const current = changes.find(v => v.type === 'windows_attributes');
    c.select(`${key} attribute`, current?.set & bit ? 'set' : current?.clear & bit ? 'clear' : 'keep', ['keep','set','clear'], action => {
      let attrs = changes.find(v => v.type === 'windows_attributes');
      if (!attrs) { attrs = { type: 'windows_attributes', set: 0, clear: 0 }; changes.push(attrs); }
      attrs.set &= ~bit; attrs.clear &= ~bit;
      if (action === 'set') attrs.set |= bit; if (action === 'clear') attrs.clear |= bit;
      change(changes.filter(v => v.type !== 'windows_attributes' || v.set || v.clear));
    });
  }
  for (const field of ['created','modified','accessed']) {
    const title = field[0].toUpperCase() + field.slice(1), v = changes.find(v => v.field === field);
    const type = !v ? 'keep' : v.type === 'timestamp' ? 'set' : v.type.slice('timestamp_'.length);
    c.select(`${title} timestamp action`, type, ['keep','set','offset','increment'], action => {
      const next = changes.filter(v => v.field !== field);
      if (action !== 'keep') next.push({ type: action === 'set' ? 'timestamp' : `timestamp_${action}`, field, ...(action === 'set' ? { value_ns: '0' } : action === 'offset' ? { offset_ns: '0' } : { start_ns: '0', step_ns: '1' }) });
      change(next, true);
    });
    if (v) for (const [key,label] of [['value_ns','value'],['offset_ns','offset'],['start_ns','increment start'],['step_ns','increment step']]) if (key in v) c.field(`${title} ${label} (ns)`, v[key], value => { v[key] = value; change(changes); }, 'integer');
  }
}
