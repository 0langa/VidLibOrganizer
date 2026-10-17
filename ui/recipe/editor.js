import { el } from '../dom.js';
export function createRecipeEditor({ recipe, onChange }) {
  const element = el('fieldset', 'rename-rules');
  const legend = el('legend', '', 'Naming rule');
  const label = el('label', '', 'Naming rule');
  const select = el('select'); select.setAttribute('aria-label', 'Naming rule');
  for (const [value, title] of [['keep', 'Keep current name'], ['fixedStem', 'Set filename stem']]) { const option = el('option', '', title); option.value = value; select.append(option); }
  label.append(select);
  const stemLabel = el('label', '', 'New stem');
  const stem = el('input'); stem.setAttribute('aria-label', 'New stem'); stem.maxLength = 16384; stemLabel.append(stem);
  element.append(legend, label, stemLabel);
  function getRecipe() { return { rule: select.value, fixedStem: stem.value }; }
  const changed = () => { stemLabel.hidden = select.value !== 'fixedStem'; onChange(getRecipe()); };
  select.addEventListener('change', changed); stem.addEventListener('input', changed);
  function setRecipe(value) { select.value = value.rule; stem.value = value.fixedStem; stemLabel.hidden = value.rule !== 'fixedStem'; }
  setRecipe(recipe);
  return { element, getRecipe, setRecipe, destroy() { select.removeEventListener('change', changed); stem.removeEventListener('input', changed); } };
}
