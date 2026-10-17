import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
const golden=JSON.parse(await readFile(new URL('./fixtures/command-contracts.json',import.meta.url)));
const original=JSON.parse(golden.recipeDocumentRequest.recipeJson);
async function mount(page) {
  await page.goto('/');
  await page.evaluate(async recipe => {
    const {createRecipeEditor}=await import('/recipe/editor.js');
    document.body.replaceChildren(); window.changes=[];
    window.editor=createRecipeEditor({recipe,onChange:value=>changes.push(value)});
    document.body.append(editor.element);
  },original);
}
test('reorder_repeat_disable_and_restore_exact_serialized_recipe',async({page})=>{
  await mount(page);
  expect(await page.evaluate(()=>editor.getRecipe())).toEqual(original);
  await page.getByRole('button',{name:'Move rule 2 up',exact:true}).click();
  await page.getByRole('button',{name:'Repeat rule 1',exact:true}).click();
  await page.getByLabel('Enable rule 3').uncheck();
  const result=await page.evaluate(()=>editor.getRecipe());
  expect(result.rules.map(r=>r.parameters)).toEqual([original.rules[1].parameters,original.rules[1].parameters,original.rules[0].parameters]);
  expect(new Set(result.rules.map(r=>r.id)).size).toBe(3);
  expect(result.rules.map(r=>r.enabled)).toEqual([false,false,false]);
  await page.evaluate(value=>editor.setRecipe(value),original);
  expect(await page.evaluate(()=>editor.getRecipe())).toEqual(original);
  await expect(page.getByRole('button',{name:'Undo edit',exact:true})).toBeDisabled();
});
test('edit_undo_redo_is_not_filesystem_undo',async({page})=>{
  await mount(page);
  await expect(page.getByRole('button',{name:'Undo edit',exact:true})).toHaveCount(1);
  await page.getByLabel('New stem').fill('changed');
  await page.getByRole('button',{name:'Undo edit',exact:true}).click();
  expect(await page.evaluate(()=>editor.getRecipe())).toEqual(original);
  await page.getByRole('button',{name:'Redo edit',exact:true}).click();
  expect(await page.evaluate(()=>editor.getRecipe().rules[0].parameters.value)).toBe('changed');
  await page.getByRole('button',{name:'Undo edit',exact:true}).click();
  await page.getByLabel('New stem').fill('branch');
  await expect(page.getByRole('button',{name:'Redo edit',exact:true})).toBeDisabled();
  expect(await page.evaluate(()=>changes.length)).toBe(5);
  const before=await page.evaluate(()=>changes.length);
  await page.evaluate(()=>{const input=document.querySelector('input[aria-label="New stem"]');editor.destroy();input.value='detached';input.dispatchEvent(new Event('input'));});
  expect(await page.evaluate(()=>changes.length)).toBe(before);
});
test('returned_snapshots_are_detached_and_between_controls_keep_typed_semantics',async({page})=>{
  await mount(page);
  expect(await page.evaluate(()=>{const value=editor.getRecipe();value.rules[0].parameters.value='outside';return editor.getRecipe();})).toEqual(original);
  await page.getByLabel('Between occurrence').fill('3');
  await page.getByLabel('Include delimiters').check();
  expect(await page.evaluate(()=>editor.getRecipe().rules[1].segment)).toEqual({...original.rules[1].segment,occurrence:3,include_delimiters:true});
});

test('rename_sends_every_ordered_rule_and_explicit_selection_binding',async({page})=>{
  const {desktopBridge}=await import('./fixtures/desktop-bridge.js');
  const {readFile}=await import('node:fs/promises');
  const fixture=JSON.parse(await readFile(new URL('./fixtures/command-contracts.json',import.meta.url)));
  await page.addInitScript(desktopBridge,fixture);await page.goto('/');
  await page.getByRole('button',{name:'Rename',exact:true}).click();
  await page.getByRole('button',{name:'Choose folders',exact:true}).click();await page.getByLabel('Select original.mp4').check();
  await page.getByLabel('Naming rule',{exact:true}).selectOption('fixedStem');await page.getByLabel('New stem',{exact:true}).fill('one');
  await page.getByRole('button',{name:'Repeat rule 1',exact:true}).click();await page.getByLabel('New stem 2',{exact:true}).fill('two');
  await page.getByLabel('Enable rule 1').uncheck();await page.getByTestId('rename-preview').click();
  const request=await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request);
  expect(request.rebindSelection).toBe(true);
  expect(JSON.parse(request.recipeJson).rules.map(r=>[r.enabled,r.parameters.value])).toEqual([[false,'one'],[true,'two']]);
  await page.getByRole('button',{name:'Undo edit',exact:true}).click();
  await expect(page.getByTestId('rename-apply')).toBeDisabled();
  expect(await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='start_undo'))).toEqual([]);
});

test('preset_save_load_and_import_keep_snapshots_and_edit_history_separate',async({page})=>{
  const {desktopBridge}=await import('./fixtures/desktop-bridge.js');
  const {readFile}=await import('node:fs/promises');
  const fixture=JSON.parse(await readFile(new URL('./fixtures/command-contracts.json',import.meta.url)));
  await page.addInitScript(desktopBridge,fixture);
  await page.addInitScript(()=>{
    const invoke=window.__TAURI__.core.invoke;let saved=null;
    window.__TAURI__.core.invoke=async(command,args)=>{
      if(command==='save_rename_preset') {saved={presetId:'00000000-0000-4000-8000-000000000099',revision:saved?String(BigInt(saved.revision)+1n):'1',recipeJson:args.request.recipeJson};return structuredClone(saved);}
      if(command==='load_rename_preset')return structuredClone(saved);
      if(command==='import_rename_recipe')return {recipeJson:args.request.recipeJson};
      return invoke(command,args);
    };
  });
  await page.goto('/');await page.getByRole('button',{name:'Rename',exact:true}).click();
  await expect(page.getByRole('button',{name:'Save new preset',exact:true})).toHaveCount(1);
  await page.getByLabel('Naming rule',{exact:true}).selectOption('fixedStem');await page.getByLabel('New stem',{exact:true}).fill('saved');
  await page.getByRole('button',{name:'Save new preset',exact:true}).click();await expect(page.getByTestId('recipe-preset-status')).toContainText('revision 1');
  await page.getByLabel('New stem',{exact:true}).fill('draft');
  await page.getByRole('button',{name:'Load preset',exact:true}).click();await expect(page.getByLabel('New stem',{exact:true})).toHaveValue('saved');
  await expect(page.getByRole('button',{name:'Undo edit',exact:true})).toBeDisabled();
  await page.getByLabel('New stem',{exact:true}).fill('new revision');await page.getByRole('button',{name:'Save preset revision',exact:true}).click();await expect(page.getByTestId('recipe-preset-status')).toContainText('revision 2');
  await page.getByLabel('Import recipe file').setInputFiles({name:'recipe.json',mimeType:'application/json',buffer:Buffer.from(JSON.stringify(original))});
  await expect(page.getByLabel('Numbering scope')).toHaveValue('directories');
  await expect(page.getByLabel('Between occurrence')).toHaveValue('2');await expect(page.getByLabel('Enable rule 2')).not.toBeChecked();
  await expect(page.getByRole('button',{name:'Save preset revision',exact:true})).toBeDisabled();
});

test('unfinished_advanced_rule_edit_cannot_preview_previous_valid_recipe',async({page})=>{
  await mount(page);
  await page.evaluate(value=>{value.rules=[{id:'template',enabled:true,segment:{type:'stem'},parameters:{type:'template',parts:[{type:'literal',value:'safe'}]}}];editor.setRecipe(value);},original);
  await page.getByLabel('Rule 1 parameters').fill('{broken');
  expect(await page.evaluate(()=>{try{editor.getRecipe();return false;}catch{return true;}})).toBe(true);
  expect(await page.evaluate(()=>changes.length)).toBeGreaterThan(0);
  await page.getByLabel('Rule 1 parameters').fill(JSON.stringify({type:'template',parts:[{type:'literal',value:'fixed'}]}));
  expect(await page.evaluate(()=>editor.getRecipe().rules[0].parameters.parts[0].value)).toBe('fixed');
});
