import { el } from '../dom.js';

/** Accessible data controls. No filename evaluation or native path construction. */
export function controls(host, signal) {
  const listen = (node, event, fn) => node.addEventListener(event, fn, { signal });
  function field(label, value, update, kind = 'text') {
    const wrap = el('label', '', label), input = el(kind === 'textarea' ? 'textarea' : 'input');
    if (kind !== 'textarea') input.type = kind === 'integer' ? 'text' : kind;
    input.setAttribute('aria-label', label);
    if (kind === 'checkbox') input.checked = Boolean(value); else input.value = value ?? '';
    if (kind === 'integer') input.inputMode = 'numeric';
    listen(input, kind === 'checkbox' ? 'change' : 'input', () => update(kind === 'checkbox' ? input.checked : kind === 'number' && input.value !== '' && Number.isSafeInteger(Number(input.value)) ? Number(input.value) : input.value));
    wrap.append(input); host.append(wrap); return input;
  }
  function select(label, value, options, update) {
    const wrap = el('label', '', label), input = el('select'); input.setAttribute('aria-label', label);
    const pairs = options.map(o => Array.isArray(o) ? o : [o, o.replaceAll('_', ' ')]);
    if (!pairs.some(([key]) => key === value)) pairs.push([value, `Imported: ${value}`]);
    for (const [key, title] of pairs) { const option = el('option', '', title); option.value = key; input.append(option); }
    input.value = value; listen(input, 'change', () => update(input.value)); wrap.append(input); host.append(wrap); return input;
  }
  function button(label, fn) { const b = el('button', 'button', label); b.type = 'button'; listen(b, 'click', fn); host.append(b); return b; }
  return { field, select, button };
}
export const radices = ['decimal', 'hex_lower', 'hex_upper', 'alphabetic_upper', 'alphabetic_lower'];
const dateRule = () => ({ input_format: null, output_format: '%Y-%m-%d', timezone: null, assume_timezone: null, ambiguity: 'reject', nonexistent: 'reject' });
export function parameterDefault(type) {
  const defaults = {
    keep: { type: 'keep' }, fixedStem: { type: 'fixed_stem', value: 'renamed' },
    text: { type: 'text', operation: { type: 'fixed', value: '' } },
    regex: { type: 'regex', rule: { pattern: '', replacement: '', scope: 'all', case_sensitive: true } },
    number: { type: 'number', rule: { start: '1', step: '1', padding: '0', repeat_each: '1', radix: 'decimal', reset_by_folder: false, at: { type: 'end' }, prefix: '', suffix: '' } },
    pad_numbers: { type: 'pad_numbers', padding: '2', scope: 'all' },
    date: { type: 'date', operation: { type: 'insert', source: 'now', at: { type: 'end' }, prefix: '', suffix: '', rule: dateRule() } },
    template: { type: 'template', parts: [] },
  };
  return structuredClone(defaults[type]);
}
function position(c, label, value, change, exact = false) {
  c.select(label, value.type, [['end', 'Append'], ['before', 'Before position']], type => change(type === 'end' ? { type } : { type, position: exact ? '1' : 1 }, true));
  if (value.type === 'before') c.field(`${label} grapheme`, value.position, v => change({ ...value, position: v }), exact ? 'integer' : 'number');
}
function textDefault(type) {
  return { type, ...({ fixed: { value: '' }, prefix: { text: '' }, suffix: { text: '' }, insert: { at: { type: 'end' }, text: '' }, replace: { replacements: [], scope: 'all', case_sensitive: true }, remove_characters: { characters: '' }, remove_range: { start: 1, end: -1 }, crop: { delimiter: '-', occurrence: 1, side: 'before', include_delimiter: false }, move_copy: { start: 1, end: -1, destination: { type: 'end' }, copy: false }, collapse_whitespace: { replacement: ' ' }, translate: { translations: [] }, case: { mode: 'title', exceptions: [], roman: null } }[type] || {}) };
}
export function parameterFields(host, parameters, change, signal) {
  const p = structuredClone(parameters), c = controls(host, signal);
  const set = (object, key, value, refresh = false) => { object[key] = value; change(structuredClone(p), refresh); };
  const field = (label, object, key, kind = 'text') => c.field(label, object[key], value => set(object, key, value), kind);
  const select = (label, object, key, options) => c.select(label, object[key], options, value => set(object, key, value));
  if (p.type === 'text') {
    const o = p.operation;
    c.select('Text operation', o.type, ['fixed','keep','remove','prefix','suffix','insert','replace','remove_characters','remove_range','crop','move_copy','reverse','trim','collapse_whitespace','translate','transliterate','case'], type => change({ type: 'text', operation: textDefault(type) }, true));
    if (o.type === 'fixed') field('Replacement text', o, 'value');
    if (['prefix','suffix','insert'].includes(o.type)) field('Added text', o, 'text');
    if (o.type === 'insert') position(c, 'Insert position', o.at, (v, refresh) => set(o, 'at', v, refresh));
    if (o.type === 'replace' || o.type === 'translate') {
      const key = o.type === 'replace' ? 'replacements' : 'translations';
      o[key].forEach((pair, i) => {
        field(`From ${i + 1}`, pair, 'from'); field(`To ${i + 1}`, pair, 'to');
        c.button(`Remove pair ${i + 1}`, () => { o[key].splice(i, 1); change(p, true); });
      });
      c.button('Add character pair', () => { o[key].push({ from: '', to: '' }); change(p, true); });
      if (o.type === 'replace') { select('Replacement matches', o, 'scope', ['first','all']); field('Replacement case sensitive', o, 'case_sensitive', 'checkbox'); }
    }
    if (o.type === 'remove_characters') field('Characters to remove', o, 'characters');
    if (['remove_range','move_copy'].includes(o.type)) { field('Substring start', o, 'start', 'number'); field('Substring end', o, 'end', 'number'); }
    if (o.type === 'move_copy') { position(c, 'Substring destination', o.destination, (v, refresh) => set(o, 'destination', v, refresh)); field('Copy substring', o, 'copy', 'checkbox'); }
    if (o.type === 'crop') { field('Crop delimiter', o, 'delimiter'); field('Crop occurrence', o, 'occurrence', 'number'); select('Keep side', o, 'side', ['before','after']); field('Keep crop delimiter', o, 'include_delimiter', 'checkbox'); }
    if (o.type === 'collapse_whitespace') field('Whitespace replacement', o, 'replacement');
    if (o.type === 'case') {
      select('Case mode', o, 'mode', ['keep','upper','lower','title','sentence']);
      c.field('Case exceptions', o.exceptions.join('\n'), value => set(o, 'exceptions', value.split('\n').filter(Boolean)), 'textarea');
      c.select('Roman numerals', o.roman ?? 'disabled', ['disabled','keep','upper','lower'], value => set(o, 'roman', value === 'disabled' ? null : value));
    }
  }
  if (p.type === 'regex') {
    if (!p.rule || typeof p.rule.pattern !== 'string' || typeof p.rule.replacement !== 'string' || typeof p.rule.scope !== 'string' || typeof p.rule.case_sensitive !== 'boolean') throw new Error('Regex controls require a complete rule object.');
    const details=el('details');details.append(el('summary','','Regex options'));host.append(details);let mounted=false;
    details.addEventListener('toggle',()=>{if(!details.open||mounted)return;mounted=true;
      const fields=controls(details,signal);
      fields.field('Regex pattern',p.rule.pattern,v=>set(p.rule,'pattern',v));fields.field('Regex replacement',p.rule.replacement,v=>set(p.rule,'replacement',v));
      fields.select('Regex matches',p.rule.scope,['first','all'],v=>set(p.rule,'scope',v));fields.field('Regex case sensitive',p.rule.case_sensitive,v=>set(p.rule,'case_sensitive',v),'checkbox');
      details.append(el('p','','PCRE2 captures, lookarounds and pattern backreferences. Bounded interpreted execution; no scripts.'));
    },{signal});
  }
  if (p.type === 'number') {
    for (const [label, key] of [['Counter start','start'],['Counter step','step'],['Counter padding','padding'],['Repeat each','repeat_each']]) field(label, p.rule, key, 'integer');
    select('Counter representation', p.rule, 'radix', radices); field('Reset by original folder', p.rule, 'reset_by_folder', 'checkbox');
    position(c, 'Counter position', p.rule.at, (v, refresh) => set(p.rule, 'at', v, refresh), true);
    field('Counter prefix', p.rule, 'prefix'); field('Counter suffix', p.rule, 'suffix');
  }
  if (p.type === 'pad_numbers') { field('Existing number padding', p, 'padding', 'integer'); select('Existing number matches', p, 'scope', ['first','all']); }
  if (p.type === 'date') {
    const o = p.operation;
    c.select('Date action', o.type, ['insert','reformat'], type => change({ type: 'date', operation: type === 'reformat' ? { type, rule: o.rule } : { ...parameterDefault('date').operation, rule: o.rule } }, true));
    if (o.type === 'insert') {
      select('Date source', o, 'source', ['created','modified','accessed','recorded','now']);
      position(c, 'Date position', o.at, (v, refresh) => set(o, 'at', v, refresh), true);
      field('Date prefix', o, 'prefix'); field('Date suffix', o, 'suffix');
    }
    c.field('Input date format', o.rule.input_format, v => set(o.rule, 'input_format', v || null)); field('Output date format', o.rule, 'output_format');
    for (const [key, label] of [['timezone','Output timezone'],['assume_timezone','Assumed input timezone']]) {
      const zone = o.rule[key];
      c.select(label, zone?.type ?? 'frozen', [['frozen','Use frozen environment'],['utc','UTC'],['iana','IANA timezone'],['fixed_offset','Fixed offset']], type => set(o.rule, key, type === 'frozen' ? null : { type, ...(type === 'iana' ? { name: 'Etc/UTC' } : type === 'fixed_offset' ? { seconds: '0' } : {}) }, true));
      if (zone?.type === 'iana') field(`${label} name`, zone, 'name');
      if (zone?.type === 'fixed_offset') field(`${label} seconds`, zone, 'seconds', 'integer');
    }
    select('Ambiguous local time', o.rule, 'ambiguity', ['reject','earliest','latest']); select('Nonexistent local time', o.rule, 'nonexistent', ['reject','next_valid','previous_valid']);
    host.append(el('p', '', 'Dates use an explicit format. Current time is frozen by the preview; ambiguous numeric dates are never guessed.'));
  }
  if (p.type === 'template') templateFields(host, p.parts, parts => { p.parts = parts; change(p, true); }, signal, '', () => change(p));
}
const fieldTypes = ['original_stem','current_stem','extension','parent','relative_directory','counter','date','size_bytes','media','windows_property','hash','uuid','random'];
function fieldDefault(type) {
  return { key: { type, ...({ parent: { level: 1 }, date: { source: 'now' }, media: { property: 'width' }, windows_property: { property: 'System.Title' }, hash: { algorithm: 'sha256' } }[type] || {}) }, format: type === 'date' ? { type: 'date', pattern: '%Y-%m-%d' } : { type: 'text' }, fallback: null };
}
/** Edits the portable template AST, including nested optional groups. */
export function templateFields(host, parts, structural, signal, prefix = '', changed = () => structural(parts)) {
  const c = controls(host, signal);
  parts.forEach((part, i) => {
    const number = `${prefix}${i + 1}`, group = el('fieldset', 'template-part'); group.append(el('legend', '', `Template part ${number}`)); host.append(group);
    const f = controls(group, signal);
    if (part.type === 'literal') f.field(`Literal ${number}`, part.value, value => { part.value = value; changed(); });
    if (part.type === 'optional') templateFields(group, part.value, value => { part.value = value; structural(parts); }, signal, `${number}.`, changed);
    if (part.type === 'field') {
      const value = part.value;
      f.select(`Template field ${number}`, value.key.type, fieldTypes, type => { part.value = fieldDefault(type); structural(parts); });
      const set = (object, key, v) => { object[key] = v; changed(); };
      if (value.key.type === 'parent') f.field('Parent level', value.key.level, v => set(value.key, 'level', v), 'number');
      if (value.key.type === 'date') f.select('Template date source', value.key.source, ['created','modified','accessed','recorded','now'], v => set(value.key, 'source', v));
      if (value.key.type === 'media') f.select('Media property', value.key.property, ['width','height','duration_ns','frame_rate','video_codec','audio_codec','audio_channels','bit_rate','container','title','language'], v => set(value.key, 'property', v));
      if (value.key.type === 'windows_property') f.select('Windows property', value.key.property, ['System.Author','System.Title','System.Media.DateEncoded','System.Photo.DateTaken'], v => set(value.key, 'property', v));
      if (value.key.type === 'hash') f.select('Hash algorithm', value.key.algorithm, ['crc32','md5','sha1','sha256','sha512','blake3'], v => set(value.key, 'algorithm', v));
      f.select('Field format', value.format.type, ['text','integer','decimal','date'], type => { value.format = { type, ...({ integer: { radix: 'decimal', padding: 0 }, decimal: { places: 2 }, date: { pattern: '%Y-%m-%d' } }[type] || {}) }; structural(parts); });
      if (value.format.type === 'integer') { f.select('Field representation', value.format.radix, radices, v => set(value.format, 'radix', v)); f.field('Field padding', value.format.padding, v => set(value.format, 'padding', v), 'number'); }
      if (value.format.type === 'decimal') f.field('Decimal places', value.format.places, v => set(value.format, 'places', v), 'number');
      if (value.format.type === 'date') f.field('Field date pattern', value.format.pattern, v => set(value.format, 'pattern', v));
      f.select('Missing field action', value.fallback === null ? 'block' : 'fallback', [['block','Require field (block if missing)'],['fallback','Use fallback text'],['optional','Omit optional group if missing']], action => {
        if (action === 'optional') parts[i] = { type: 'optional', value: [{ type: 'field', value: { ...value, fallback: null } }] };
        else value.fallback = action === 'block' ? null : '';
        structural(parts);
      });
      if (value.fallback !== null) f.field('Fallback text', value.fallback, v => set(value, 'fallback', v));
    }
    f.button(`Remove template part ${number}`, () => { parts.splice(i, 1); structural(parts); });
    if (i > 0) f.button(`Move template part ${number} up`, () => { [parts[i-1], parts[i]] = [parts[i], parts[i-1]]; structural(parts); });
  });
  c.button('Add literal', () => { parts.push({ type: 'literal', value: '' }); structural(parts); });
  c.button('Add template field', () => { parts.push({ type: 'field', value: fieldDefault('original_stem') }); structural(parts); });
  c.button('Add optional group', () => { parts.push({ type: 'optional', value: [] }); structural(parts); });
}
