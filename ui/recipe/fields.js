import { el } from '../dom.js';
import { parameterDefault, parameterFields } from '../rename/rules.js';
export function defaultRecipe() {
  return {schema_version:1,revision:'1',selection:[],selection_filter:{extensions:['mp4','mkv','avi','mov','wmv','m4v','webm','mpg','mpeg','ts']},numbering_order:[],numbering_scope:'files',rules:[{id:crypto.randomUUID(),enabled:true,segment:{type:'stem'},parameters:{type:'keep'}}],companions:{type:'none'},destination:{type:'same_directory'},collision_policy:{type:'block'},metadata_changes:[]};
}
// Typed data controls only: selection and transformations are resolved in Rust.
export function ruleFields(host, rule, index, change, signal, invalid, draft) {
  const typedHost = el('div', 'recipe-typed-fields');
  host.append(typedHost);
  let typedEvents = new AbortController();
  signal.addEventListener('abort', () => typedEvents.abort(), { once: true });
  const listen = (node,event,handler) => node.addEventListener(event,handler,{signal:typedEvents.signal});
  function field(label, value, update, kind='text') {
    const wrap=el('label','',label), input=el('input'); input.type=kind; input.setAttribute('aria-label',label);
    if(kind==='checkbox') input.checked=value; else input.value=value;
    listen(input,kind==='checkbox'?'change':'input',()=>update(kind==='checkbox'?input.checked:kind==='number' && input.value!=='' && Number.isSafeInteger(Number(input.value))?Number(input.value):input.value));
    wrap.append(input);typedHost.append(wrap);return input;
  }
  function select(label,value,options,update) {
    const wrap=el('label','',label), input=el('select');input.setAttribute('aria-label',label);
    for(const [key,title] of options) { const option=el('option','',title);option.value=key;input.append(option); }
    input.value=value;listen(input,'change',()=>update(input.value));wrap.append(input);typedHost.append(wrap);
  }
  let parameters = structuredClone(rule.parameters), invalidText = draft;
  let jsonInput;
  function edit(patch, refresh = false) {
    if (patch.parameters && invalidText === undefined) {
      parameters = structuredClone(patch.parameters);
      if (jsonInput) jsonInput.value = JSON.stringify(parameters, null, 2);
    }
    change(patch, refresh);
  }
  function renderTyped() {
    typedEvents.abort();
    typedEvents = new AbortController();
    typedHost.replaceChildren();
    const types=[['keep','Keep current name'],['fixed_stem','Set filename stem'],['text','Text'],['regex','Regex replacements'],['number','Sequential numbering'],['pad_numbers','Pad existing numbers'],['date','Date'],['template','Name template']];
    if(!types.some(([key])=>key===parameters.type)) types.push([parameters.type,parameters.type]);
    // Retain the original simple selector's public option spelling.
    select(index===0?'Naming rule':`Naming rule ${index+1}`,parameters.type==='fixed_stem'?'fixedStem':parameters.type,types.map(([key,title])=>[key==='fixed_stem'?'fixedStem':key,title]),type=>{
      const next=parameterDefault(type);
      edit({parameters:next,...(type==='fixedStem'?{segment:{type:'stem'}}:{})},true);
    });
    if(invalidText===undefined && parameters.type==='fixed_stem') field(index===0?'New stem':`New stem ${index+1}`,parameters.value,value=>edit({parameters:{...parameters,value}}));
    else if(invalidText===undefined && parameters.type!=='keep') parameterFields(typedHost,parameters,(value,refresh)=>edit({parameters:value},refresh),typedEvents.signal);
    if(parameters.type==='fixed_stem') return;
    typedHost.append(el('p','','Extension edits change the name only; they do not convert media.'));
    const segment=rule.segment;
    select(`Segment ${index+1}`,segment.type,[['stem','Stem'],['extension','Extension'],['whole_name','Whole name'],['graphemes','Grapheme range'],['delimited','After delimiter'],['between','Between delimiters']],type=>{
      const defaults={graphemes:{target:'stem',start:1,end:-1},delimited:{target:'stem',delimiter:'-',occurrence:1,include_delimiter:false},between:{target:'stem',opening:'[',closing:']',occurrence:1,include_delimiters:false}};
      edit({segment:{type,...defaults[type]}},true);
    });
    const editSegment=(key,value)=>{segment[key]=value;edit({segment:{...segment}});};
    if(segment.target) select('Segment target',segment.target,[['stem','Stem'],['extension','Extension'],['whole_name','Whole name']],value=>editSegment('target',value));
    if(segment.type==='graphemes') {field('Start grapheme',segment.start,value=>editSegment('start',value),'number');field('End grapheme',segment.end,value=>editSegment('end',value),'number');}
    if(segment.type==='between') {
      field('Opening delimiter',segment.opening,value=>editSegment('opening',value));field('Closing delimiter',segment.closing,value=>editSegment('closing',value));
      field('Between occurrence',segment.occurrence,value=>editSegment('occurrence',value),'number');field('Include delimiters',segment.include_delimiters,value=>editSegment('include_delimiters',value),'checkbox');
    }
    if(segment.type==='delimited') {field('Delimiter',segment.delimiter,value=>editSegment('delimiter',value));field('Delimiter occurrence',segment.occurrence,value=>editSegment('occurrence',value),'number');field('Include opening delimiter',segment.include_delimiter,value=>editSegment('include_delimiter',value),'checkbox');}
  }
  renderTyped();
  if (draft !== undefined || !['keep','fixed_stem','text'].includes(parameters.type)) {
    const label = el('label', '', 'Rule parameters (JSON)');
    jsonInput = el('textarea');
    jsonInput.setAttribute('aria-label', `Rule ${index+1} parameters`);
    jsonInput.value = draft ?? JSON.stringify(parameters, null, 2);
    if (draft !== undefined) jsonInput.setCustomValidity('Enter a valid tagged rule JSON object.');
    jsonInput.addEventListener('input', () => {
      try {
        const value = JSON.parse(jsonInput.value);
        if (!value || Array.isArray(value) || typeof value.type !== 'string') throw new Error('A tagged rule object is required.');
        // Check control structure, not filename semantics. Rust owns schema validation.
        const validation = new AbortController();
        try { parameterFields(document.createDocumentFragment(), value, () => {}, validation.signal); }
        finally { validation.abort(); }
        invalidText = undefined;
        invalid(false);
        jsonInput.setCustomValidity('');
        parameters = value;
        change({parameters:value});
        // Keep the JSON node/caret; only rebind typed controls to accepted data.
        renderTyped();
      } catch {
        invalidText = jsonInput.value;
        jsonInput.setCustomValidity('Enter a valid tagged rule JSON object.');
        invalid(true, invalidText);
        renderTyped();
      }
    }, {signal});
    label.append(jsonInput);
    host.append(label);
  }
}
