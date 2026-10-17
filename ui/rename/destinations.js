import { el } from '../dom.js';
import { controls, templateFields } from './rules.js';
export function destinationFields(host, recipe, change, signal, chooseDestination) {
  const c=controls(host,signal), destination=structuredClone(recipe.destination);
  c.select('Destination action',destination.type,[['same_directory','Rename in place'],['copy_to','Copy to chosen folder'],['move_to','Move to chosen folder']],type=>change(type==='same_directory'?{type}:{type,grant_id:destination.grant_id || null,layout:destination.type==='granted'?{type:'folder_template',parts:destination.layout}:destination.layout || {type:'preserve_hierarchy'}},true));
  if(destination.type==='same_directory')return;
  if(destination.type==='granted')host.append(el('p','','Imported legacy destination: choose explicit Copy or Move, then choose a native destination.'));
  c.button('Choose destination folder',()=>chooseDestination?.());
  host.append(el('p','',destination.grant_id?'Destination reference attached; preview validates the current native grant.':'Choose a native destination before previewing.'));
  if(destination.type==='granted')return;
  c.select('Destination layout',destination.layout.type,[['preserve_hierarchy','Preserve relative hierarchy'],['folder_template','Folder template']],type=>change({...destination,layout:type==='folder_template'?{type,parts:[]}:{type}},true));
  if(destination.layout.type==='folder_template')templateFields(host,destination.layout.parts,parts=>change({...destination,layout:{type:'folder_template',parts}},true),signal,'',()=>change(destination));
  host.append(el('p','','Copy Undo removes only the created copy with matching identity and full digest. Cross-volume Move Undo may copy back and require space. Only owned empty created directories can be removed.'));
}
