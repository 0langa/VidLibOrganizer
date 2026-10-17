import { el } from '../dom.js';
import { createEditHistory } from './history.js';
import { defaultRecipe, ruleFields } from './fields.js';
import { selectionFields } from '../rename/selection.js';
import { associationFields } from '../rename/associations.js';
import { destinationFields } from '../rename/destinations.js';
import { metadataFields } from '../rename/metadata.js';
import { controls } from '../rename/rules.js';
/**
 * @typedef {Object} RecipeDraft
 * @property {1} schema_version
 * @property {string} revision Exact decimal revision.
 * @property {string[]} selection Opaque entry IDs; never native paths.
 * @property {string[]} numbering_order Explicit primary order.
 * @property {Object[]} rules Portable tagged rule data, including inactive rules.
 * @property {Object} companions
 * @property {Object} destination
 * @property {Object[]} metadata_changes Portable exact-string metadata intents.
 * @param {{recipe: RecipeDraft, onChange?: (recipe: RecipeDraft) => void, chooseDestination?: () => void}} options
 * @returns {{element: HTMLFieldSetElement, getRecipe: () => RecipeDraft, setRecipe: (recipe: RecipeDraft) => void, setSelectionItems: (items: Object[]) => void, setDestination: (id: string) => void, excludeItems: (ids: string[]) => void, destroy: () => void}}
 */
export function createRecipeEditor({ recipe, onChange = () => {}, chooseDestination }) {
  const element=el('fieldset','rename-rules'), history=createEditHistory(recipe);
  let events=new AbortController(), destroyed=false;
  const invalidDrafts=new Map(), openPanels=new Set();
  let selectionItems=[];
  let undo,redo;
  function changed(renderFields=true) {
    if(renderFields) render(); else {undo.disabled=invalidDrafts.size>0 || !history.canUndo;redo.disabled=invalidDrafts.size>0 || !history.canRedo;}
    onChange(history.get());
  }
  function edit(mutator,renderFields=true) {
    if(destroyed) return;
    const value=history.get();mutator(value);if(history.change(value)) changed(renderFields);
  }
  function button(host,label,fn,disabled=false) {
    const button=el('button','button',label);button.type='button';button.disabled=disabled;
    button.addEventListener('click',fn,{signal:events.signal});host.append(button);return button;
  }
  function render() {
    const focused=element.contains(document.activeElement)?document.activeElement:null;
    const focusLabel=focused?.getAttribute('aria-label') || focused?.textContent;
    const focusRule=focused?.closest('.recipe-rule')?.dataset.ruleId;
    events.abort();events=new AbortController();element.replaceChildren(el('legend','','Recipe rules'));
    const value=history.get();
    const toolbar=el('div','recipe-toolbar');element.append(toolbar);
    undo=button(toolbar,'Undo edit',()=>{if(!invalidDrafts.size && history.undo())changed();},invalidDrafts.size>0 || !history.canUndo);
    redo=button(toolbar,'Redo edit',()=>{if(!invalidDrafts.size && history.redo())changed();},invalidDrafts.size>0 || !history.canRedo);
    button(toolbar,'Add rule',()=>edit(value=>value.rules.push({...defaultRecipe().rules[0]})),value.rules.length>=100);
    const scope=el('select');scope.setAttribute('aria-label','Numbering scope');
    for(const [key,title] of [['files','Files'],['directories','Directories']]) {const option=el('option','',title);option.value=key;scope.append(option);}
    scope.value=value.numbering_scope || 'files';scope.addEventListener('change',()=>edit(value=>{value.numbering_scope=scope.value;},false),{signal:events.signal});
    const label=el('label','','Numbering scope');label.append(scope);element.append(label);
    const panel=(title,fields)=>{
      const details=el('details'),summary=el('summary','',title);details.append(summary);element.append(details);
      let mounted=false;const mount=()=>{if(!mounted){mounted=true;fields(details);}};
      details.open=openPanels.has(title);if(details.open)mount();
      details.addEventListener('toggle',()=>{if(details.open){openPanels.add(title);mount();}else openPanels.delete(title);},{signal:events.signal});
    };
    panel('Selection filters',host=>selectionFields(host,value,(filter,refresh)=>edit(v=>{v.selection_filter=filter;},refresh),events.signal));
    panel('Filesystem metadata',host=>metadataFields(host,value,(changes,refresh)=>edit(v=>{v.metadata_changes=changes;},refresh),events.signal));
    associationFields(element,value,(policy,refresh)=>edit(v=>{v.companions=policy;v.selection=selectionItems.map(item=>item.itemId);const owned=new Set((policy.associations||[]).flatMap(a=>a.companions));v.numbering_order=v.selection.filter(id=>!owned.has(id));},refresh),events.signal,selectionItems);
    destinationFields(element,value,(destination,refresh)=>edit(v=>{v.destination=destination;},refresh),events.signal,chooseDestination);
    const policyControls=controls(element,events.signal),policy=structuredClone(value.collision_policy);
    policyControls.select('Collision handling',policy.type,[['block','Block conflicts'],['suffix','Try explicit suffix']],type=>edit(v=>{v.collision_policy=type==='block'?{type}:{type,separator:'_',max_attempts:100};}));
    if(policy.type==='suffix'){
      policyControls.field('Collision separator',policy.separator,value=>{policy.separator=value;edit(v=>{v.collision_policy={...policy};},false);});
      policyControls.field('Maximum collision attempts',policy.max_attempts,value=>{policy.max_attempts=value;edit(v=>{v.collision_policy={...policy};},false);},'number');
    }
    value.rules.forEach((rule,index)=>{
      const row=el('fieldset','recipe-rule');row.dataset.ruleId=rule.id;row.append(el('legend','',`Rule ${index+1}`));
      const enabled=el('input');enabled.type='checkbox';enabled.checked=rule.enabled;enabled.setAttribute('aria-label',`Enable rule ${index+1}`);
      enabled.addEventListener('change',()=>edit(value=>{value.rules[index].enabled=enabled.checked;},false),{signal:events.signal});
      const label=el('label','','Enabled');label.append(enabled);row.append(label);
      ruleFields(row,rule,index,(patch,refresh=false)=>{
        // Replacing this rule's type must not hide its unresolved draft.
        if(refresh && patch.parameters && invalidDrafts.has(rule.id)){render();return;}
        edit(value=>{value.rules[index]={...value.rules[index],...patch};},refresh);
      },events.signal,(bad,text)=>{
        if(bad){invalidDrafts.set(rule.id,text);onChange(history.get());}
        else invalidDrafts.delete(rule.id);
        undo.disabled=invalidDrafts.size>0 || !history.canUndo;
        redo.disabled=invalidDrafts.size>0 || !history.canRedo;
      },invalidDrafts.get(rule.id));
      const actions=el('div','recipe-toolbar');row.append(actions);
      button(actions,`Move rule ${index+1} up`,()=>edit(value=>{[value.rules[index-1],value.rules[index]]=[value.rules[index],value.rules[index-1]];}),index===0);
      button(actions,`Move rule ${index+1} down`,()=>edit(value=>{[value.rules[index+1],value.rules[index]]=[value.rules[index],value.rules[index+1]];}),index===value.rules.length-1);
      button(actions,`Repeat rule ${index+1}`,()=>edit(value=>{value.rules.splice(index+1,0,{...structuredClone(value.rules[index]),id:crypto.randomUUID()});}),value.rules.length>=100);
      button(actions,`Remove rule ${index+1}`,()=>{invalidDrafts.delete(rule.id);edit(value=>value.rules.splice(index,1));});
      element.append(row);
    });
    if(focused){const scope=focusRule?[...element.querySelectorAll('.recipe-rule')].find(row=>row.dataset.ruleId===focusRule):element;const candidates=[...(scope||element).querySelectorAll('input,select,textarea,button,summary')];const target=candidates.find(node=>(node.getAttribute('aria-label')||node.textContent)===focusLabel)||scope?.querySelector('select')||element.querySelector('button');target?.focus();}
  }
  render();
  return {element,excludeItems(ids){const excluded=new Set(ids);edit(v=>{v.selection=v.selection.filter(id=>!excluded.has(id));v.numbering_order=v.numbering_order.filter(id=>!excluded.has(id));if(v.companions.associations)v.companions.associations=v.companions.associations.filter(a=>!excluded.has(a.primary)).map(a=>({...a,companions:a.companions.filter(id=>!excluded.has(id))}));});},setSelectionItems(items){selectionItems=structuredClone(items);render();},setDestination(grantId){edit(v=>{v.destination={...v.destination,grant_id:grantId};});},getRecipe:()=>{if(invalidDrafts.size)throw new Error("Complete the invalid rule JSON before previewing or saving.");return history.get();},setRecipe(value){if(destroyed)return;invalidDrafts.clear();history.reset(value);render();},destroy(){if(destroyed)return;destroyed=true;events.abort();invalidDrafts.clear();history.clear();element.remove();}};
}
