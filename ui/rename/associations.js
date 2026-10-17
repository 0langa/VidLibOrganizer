import { el } from '../dom.js';
import { controls } from './rules.js';
export function associationFields(host, recipe, change, signal, items=[]) {
  const c=controls(host,signal), policy=structuredClone(recipe.companions);
  c.select('Companion policy',policy.type,[['none','Independent selected items'],['explicit','Manual owners only'],['match_exact','Match exact stem and language/role, with manual owners']],type=>change({type,...(type==='none'?{}:{associations:policy.associations || []})},true));
  host.append(el('p','','Only selected companions can change. Ambiguous matches need an explicit owner. Unselected related files stay untouched.'));
  if(policy.type==='none')return;
  const options=items.map(v=>[v.itemId,v.displayName]);
  const bindings=policy.associations || [];
  bindings.forEach((binding,i)=>{
    c.select(`Primary owner ${i+1}`,binding.primary,options,v=>{binding.primary=v;change({...policy,associations:bindings});});
    binding.companions.forEach((id,j)=>{
      c.select(`Companion ${i+1}.${j+1}`,id,options,v=>{binding.companions[j]=v;change({...policy,associations:bindings});});
      c.button(`Remove companion ${i+1}.${j+1}`,()=>{binding.companions.splice(j,1);change({...policy,associations:bindings},true);});
    });
    c.button(`Add companion to owner ${i+1}`,()=>{if(items.length){binding.companions.push(items.find(v=>v.itemId!==binding.primary)?.itemId || items[0].itemId);change({...policy,associations:bindings},true);}});
    c.button(`Remove owner ${i+1}`,()=>{bindings.splice(i,1);change({...policy,associations:bindings},true);});
  });
  c.button('Add primary owner',()=>{if(items.length){bindings.push({primary:items[0].itemId,companions:[]});change({...policy,associations:bindings},true);}});
}
