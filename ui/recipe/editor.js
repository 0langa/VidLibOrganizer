import { el } from '../dom.js';
import { createEditHistory } from './history.js';
import { defaultRecipe, ruleFields } from './fields.js';
export function createRecipeEditor({ recipe, onChange = () => {} }) {
  const element=el('fieldset','rename-rules'), history=createEditHistory(recipe);
  let events=new AbortController(), destroyed=false;
  const invalidDrafts=new Set();
  let undo,redo;
  function changed(renderFields=true) {
    if(renderFields) render(); else {undo.disabled=!history.canUndo;redo.disabled=!history.canRedo;}
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
    invalidDrafts.clear();events.abort();events=new AbortController();element.replaceChildren(el('legend','','Recipe rules'));
    const value=history.get();
    const toolbar=el('div','recipe-toolbar');element.append(toolbar);
    undo=button(toolbar,'Undo edit',()=>{if(history.undo())changed();},!history.canUndo);
    redo=button(toolbar,'Redo edit',()=>{if(history.redo())changed();},!history.canRedo);
    button(toolbar,'Add rule',()=>edit(value=>value.rules.push({...defaultRecipe().rules[0]})),value.rules.length>=100);
    const scope=el('select');scope.setAttribute('aria-label','Numbering scope');
    for(const [key,title] of [['files','Files'],['directories','Directories']]) {const option=el('option','',title);option.value=key;scope.append(option);}
    scope.value=value.numbering_scope || 'files';scope.addEventListener('change',()=>edit(value=>{value.numbering_scope=scope.value;},false),{signal:events.signal});
    const label=el('label','','Numbering scope');label.append(scope);element.append(label);
    value.rules.forEach((rule,index)=>{
      const row=el('fieldset','recipe-rule');row.append(el('legend','',`Rule ${index+1}`));
      const enabled=el('input');enabled.type='checkbox';enabled.checked=rule.enabled;enabled.setAttribute('aria-label',`Enable rule ${index+1}`);
      enabled.addEventListener('change',()=>edit(value=>{value.rules[index].enabled=enabled.checked;},false),{signal:events.signal});
      const label=el('label','','Enabled');label.append(enabled);row.append(label);
      ruleFields(row,rule,index,(patch,refresh=false)=>edit(value=>{value.rules[index]={...value.rules[index],...patch};},refresh),events.signal,bad=>{if(bad){invalidDrafts.add(rule.id);onChange(history.get());}else invalidDrafts.delete(rule.id);});
      const actions=el('div','recipe-toolbar');row.append(actions);
      button(actions,`Move rule ${index+1} up`,()=>edit(value=>{[value.rules[index-1],value.rules[index]]=[value.rules[index],value.rules[index-1]];}),index===0);
      button(actions,`Move rule ${index+1} down`,()=>edit(value=>{[value.rules[index+1],value.rules[index]]=[value.rules[index],value.rules[index+1]];}),index===value.rules.length-1);
      button(actions,`Repeat rule ${index+1}`,()=>edit(value=>{value.rules.splice(index+1,0,{...structuredClone(value.rules[index]),id:crypto.randomUUID()});}),value.rules.length>=100);
      button(actions,`Remove rule ${index+1}`,()=>edit(value=>value.rules.splice(index,1)));
      element.append(row);
    });
  }
  render();
  return {element,getRecipe:()=>{if(invalidDrafts.size)throw new Error("Complete the invalid rule JSON before previewing or saving.");return history.get();},setRecipe(value){if(destroyed)return;history.reset(value);render();},destroy(){if(destroyed)return;destroyed=true;events.abort();history.clear();element.remove();}};
}
