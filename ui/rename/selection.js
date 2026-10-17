import { controls } from './rules.js';
export function selectionFields(host, recipe, change, signal) {
  const c = controls(host, signal), filter = structuredClone(recipe.selection_filter || {});
  const set = (key,value,refresh=false) => { filter[key] = value; change(structuredClone(filter),refresh); };
  c.button('Select video types',()=>{filter.kinds=['File'];set('extensions',['mp4','mkv','avi','mov','wmv','m4v','webm','mpg','mpeg','ts'],true);});
  c.button('Include collection assets',()=>{filter.kinds=['File'];set('extensions',[...new Set([...(filter.extensions||[]),'srt','ass','ssa','vtt','sup','idx','sub','nfo','jpg','jpeg','png','webp'])],true);});
  c.button('Include all files and folders',()=>{filter.kinds=['File','Directory'];set('extensions',[],true);});
  c.field('Recursive selection', filter.recursive ?? true, v => set('recursive',v),'checkbox');
  c.field('Maximum depth', filter.max_depth ?? '', v => set('max_depth',v === '' ? null : v),'number');
  for (const kind of ['File','Directory']) c.field(`Include ${kind}s`, !(filter.kinds?.length) || filter.kinds.includes(kind), checked => {
    const kinds = new Set(filter.kinds?.length ? filter.kinds : ['File','Directory']); checked ? kinds.add(kind) : kinds.delete(kind);
    // An empty backend kind list means unrestricted; use explicit missing kind for no matches.
    set('kinds',kinds.size ? [...kinds] : ['Other']);
  },'checkbox');
  c.field('Extensions (one per line)', (filter.extensions || []).join('\n'), v => set('extensions',v.split('\n').filter(Boolean)),'textarea');
  for (const [key,label] of [['size','size (bytes)'],['name_length','name length'],['path_length','path length'],['created_ns','created time (ns)'],['modified_ns','modified time (ns)'],['accessed_ns','accessed time (ns)']]) {
    for (const [bound,word] of [['min','Minimum'],['max','Maximum']]) c.field(`${word} ${label}`,filter[key]?.[bound] ?? '', v => { const range={min:null,max:null,...filter[key],[bound]:v || null};set(key,range.min === null && range.max === null ? null : range); },'integer');
  }
  for (const [label,bit] of [['Read only',1],['Hidden',2],['System',4],['Archive',32]]) c.select(`Filter ${label.toLowerCase()}`,filter.attributes?.required & bit ? 'required' : filter.attributes?.forbidden & bit ? 'forbidden' : 'any',['any','required','forbidden'],v=>{
    const a={required:0,forbidden:0,...filter.attributes};a.required &= ~bit;a.forbidden &= ~bit;if(v==='required')a.required|=bit;if(v==='forbidden')a.forbidden|=bit;set('attributes',a);
  });
  for (const mode of ['include','exclude']) {
    (filter[mode] || []).forEach((p,i)=>{
      const update=(key,v)=>{p[key]=v;set(mode,filter[mode]);};
      c.select(`${mode} pattern ${i+1} field`,p.field,['name','relative_path','full_path'],v=>update('field',v));
      c.select(`${mode} pattern ${i+1} syntax`,p.syntax,['wildcard','regex'],v=>update('syntax',v));
      c.field(`${mode} pattern ${i+1}`,p.value,v=>update('value',v));c.field(`${mode} pattern ${i+1} case sensitive`,p.case_sensitive,v=>update('case_sensitive',v),'checkbox');
      c.button(`Remove ${mode} pattern ${i+1}`,()=>{filter[mode].splice(i,1);set(mode,filter[mode],true);});
    });
    c.button(`Add ${mode} pattern`,()=>set(mode,[...(filter[mode] || []),{field:'name',syntax:'wildcard',value:'*',case_sensitive:false}],true));
  }
}
