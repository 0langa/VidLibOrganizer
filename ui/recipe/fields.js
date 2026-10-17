import { el } from '../dom.js';
import { parameterDefault, parameterFields } from '../rename/rules.js';
export function defaultRecipe() {
  return {schema_version:1,revision:'1',selection:[],selection_filter:{extensions:['mp4','mkv','avi','mov','wmv','m4v','webm','mpg','mpeg','ts']},numbering_order:[],numbering_scope:'files',rules:[{id:crypto.randomUUID(),enabled:true,segment:{type:'stem'},parameters:{type:'keep'}}],companions:{type:'none'},destination:{type:'same_directory'},collision_policy:{type:'block'},metadata_changes:[]};
}
// Typed data controls only: selection and transformations are resolved in Rust.
export function ruleFields(host, rule, index, change, signal, invalid, draft) {
  const listen = (node,event,handler) => node.addEventListener(event,handler,{signal});
  function field(label, value, update, kind='text') {
    const wrap=el('label','',label), input=el('input'); input.type=kind; input.setAttribute('aria-label',label);
    if(kind==='checkbox') input.checked=value; else input.value=value;
    listen(input,kind==='checkbox'?'change':'input',()=>update(kind==='checkbox'?input.checked:kind==='number' && input.value!=='' && Number.isSafeInteger(Number(input.value))?Number(input.value):input.value));
    wrap.append(input);host.append(wrap);return input;
  }
  function select(label,value,options,update) {
    const wrap=el('label','',label), input=el('select');input.setAttribute('aria-label',label);
    for(const [key,title] of options) { const option=el('option','',title);option.value=key;input.append(option); }
    input.value=value;listen(input,'change',()=>update(input.value));wrap.append(input);host.append(wrap);
  }
  const parameters=rule.parameters;
  const types=[['keep','Keep current name'],['fixed_stem','Set filename stem'],['text','Text'],['regex','Regex replacements'],['number','Sequential numbering'],['pad_numbers','Pad existing numbers'],['date','Date'],['template','Name template']];
  if(!types.some(([key])=>key===parameters.type)) types.push([parameters.type,parameters.type]);
  // Retain the original simple selector's public option spelling.
  select(index===0?'Naming rule':`Naming rule ${index+1}`,parameters.type==='fixed_stem'?'fixedStem':parameters.type,types.map(([key,title])=>[key==='fixed_stem'?'fixedStem':key,title]),type=>{
    const next=parameterDefault(type);
    change({parameters:next,...(type==='fixedStem'?{segment:{type:'stem'}}:{})},true);
  });
  if(draft===undefined && parameters.type==='fixed_stem') field(index===0?'New stem':`New stem ${index+1}`,parameters.value,value=>change({parameters:{...parameters,value}}));
  else if(draft===undefined && parameters.type!=='keep') parameterFields(host,parameters,(value,refresh)=>change({parameters:value},refresh),signal);
  if(draft!==undefined || !['keep','fixed_stem','text'].includes(parameters.type)) {
    const label=el('label','','Rule parameters (JSON)'), input=el('textarea');input.setAttribute('aria-label',`Rule ${index+1} parameters`);input.value=draft ?? JSON.stringify(parameters,null,2);
    if(draft!==undefined)input.setCustomValidity('Enter a valid tagged rule JSON object.');
    listen(input,'input',()=>{
      try {
        const value=JSON.parse(input.value);
        if(!value || Array.isArray(value) || typeof value.type!=='string') throw new Error('A tagged rule object is required.');
        // Check renderability, not filename semantics. Rust remains the schema authority.
        const validation = new AbortController();
        try { parameterFields(document.createDocumentFragment(), value, () => {}, validation.signal); } finally { validation.abort(); }
        invalid(false);input.setCustomValidity('');change({parameters:value});
      } catch {input.setCustomValidity('Enter a valid tagged rule JSON object.');invalid(true,input.value);}
    });label.append(input);host.append(label);
  }
  if(parameters.type==='fixed_stem') return;
  host.append(el('p','','Extension edits change the name only; they do not convert media.'));
  const segment=rule.segment;
  select(`Segment ${index+1}`,segment.type,[['stem','Stem'],['extension','Extension'],['whole_name','Whole name'],['graphemes','Grapheme range'],['delimited','After delimiter'],['between','Between delimiters']],type=>{
    const defaults={graphemes:{target:'stem',start:1,end:-1},delimited:{target:'stem',delimiter:'-',occurrence:1,include_delimiter:false},between:{target:'stem',opening:'[',closing:']',occurrence:1,include_delimiters:false}};
    change({segment:{type,...defaults[type]}},true);
  });
  const edit=(key,value)=>{segment[key]=value;change({segment:{...segment}});};
  if(segment.target) select('Segment target',segment.target,[['stem','Stem'],['extension','Extension'],['whole_name','Whole name']],value=>edit('target',value));
  if(segment.type==='graphemes') {field('Start grapheme',segment.start,value=>edit('start',value),'number');field('End grapheme',segment.end,value=>edit('end',value),'number');}
  if(segment.type==='between') {
    field('Opening delimiter',segment.opening,value=>edit('opening',value));field('Closing delimiter',segment.closing,value=>edit('closing',value));
    field('Between occurrence',segment.occurrence,value=>edit('occurrence',value),'number');field('Include delimiters',segment.include_delimiters,value=>edit('include_delimiters',value),'checkbox');
  }
  if(segment.type==='delimited') {field('Delimiter',segment.delimiter,value=>edit('delimiter',value));field('Delimiter occurrence',segment.occurrence,value=>edit('occurrence',value),'number');field('Include opening delimiter',segment.include_delimiter,value=>edit('include_delimiter',value),'checkbox');}
}
