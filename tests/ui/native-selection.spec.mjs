import { test, expect } from '@playwright/test';
import { desktopBridge } from './fixtures/desktop-bridge.js';
import { readFile } from 'node:fs/promises';
const fixture = JSON.parse(await readFile(new URL('./fixtures/command-contracts.json', import.meta.url)));
async function open(page) {
  await page.addInitScript(desktopBridge, fixture);
  await page.addInitScript(() => {
    const invoke = window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke = async (command,args) => {
      if (command==='accept_native_selection' && window.nativeAcceptBusy) throw Object.assign(new Error('Native acceptance is busy'),{code:'busy'});
      if (command==='pending_native_selection' && window.nativeBusy) throw Object.assign(new Error('Another job active'),{code:'busy'});
      if (command==='discard_native_selection') { testBridge.calls.push({command,args});window.nativeBusy=false;testBridge.pendingNative=null;return; }
      if (command==='import_rename_recipe') return {recipeJson:args.request.recipeJson};
      if (command==='select_sources' && window.cancelPicker) { testBridge.calls.push({command,args}); return null; }
      return invoke(command,args);
    };
  });
  await page.goto('/'); await page.getByRole('button',{name:'Rename',exact:true}).click();
}
test('cancelled_picker_changes_nothing', async ({page}) => {
  await open(page);
  await page.getByRole('button',{name:'Choose files',exact:true}).click();
  await page.getByLabel('Select original.mp4').check();
  await page.getByTestId('rename-preview').click();
  await expect(page.getByTestId('rename-apply')).toBeEnabled();
  const before=await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='update_rename_inputs').length);
  await page.evaluate(()=>window.cancelPicker=true);
  await page.getByRole('button',{name:'Choose folders',exact:true}).click();
  await expect(page.getByTestId('rename-apply')).toBeEnabled();
  expect(await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='update_rename_inputs').length)).toBe(before);
});
test('library_roots_picker_has_keyboard_access', async ({page}) => {
  await open(page);
  const button=page.getByRole('button',{name:'Use library roots',exact:true});
  await expect(button).toBeVisible(); await button.focus(); await page.keyboard.press('Enter');
  await expect(page.getByLabel('Select original.mp4')).toBeVisible();
  expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='select_sources').args.request.kind)).toBe('libraryRoots');
  expect(await page.evaluate(()=>testBridge.appliedTargets)).toEqual([]);
});
test('native_drop_replaces_selection_through_current_ticket_without_path_IPC',async({page})=>{
  await open(page);await page.getByRole('button',{name:'Choose files',exact:true}).click();await page.getByLabel('Select original.mp4').check();
  await page.evaluate(()=>testBridge.delayStatusOnce=true);await page.getByTestId('rename-preview').click();
  await expect.poll(()=>page.evaluate(()=>testBridge.pendingStatuses.length)).toBe(1);
  await page.evaluate(()=>testBridge.pendingNative={selectionId:'00000000-0000-4000-8000-000000000099'});
  await expect(page.getByText('Dropped folder',{exact:true})).toBeVisible();
  await page.evaluate(()=>testBridge.pendingStatuses.shift()());
  await expect(page.getByTestId('rename-apply')).toBeDisabled();
  expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='accept_native_selection').args)).toEqual({request:{selectionId:'00000000-0000-4000-8000-000000000099',revision:'3'}});
  expect(await page.evaluate(()=>testBridge.appliedTargets)).toEqual([]);
  await page.getByLabel('Select original.mp4').check();await page.getByTestId('rename-preview').click();await expect(page.getByTestId('rename-apply')).toBeEnabled();
});
test('native_startup_selection_opens_review_without_apply',async({page})=>{
  await page.addInitScript(desktopBridge,fixture);
  await page.addInitScript(()=>{testBridge.pendingNative={selectionId:'00000000-0000-4000-8000-000000000099'};});
  await page.goto('/');await expect(page.getByText('Dropped folder',{exact:true})).toBeVisible();
  await expect(page.getByTestId('rename-apply')).toBeDisabled();
  expect(await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='start_rename_preview'||c.command==='apply_rename'))).toEqual([]);
});

test('waiting_native_selection_can_be_discarded_without_mutation',async({page})=>{
  await open(page);await page.evaluate(()=>window.nativeBusy=true);
  await expect(page.getByText(/Native selection is waiting for review/)).toBeVisible();
  await page.getByRole('button',{name:'Discard waiting native selection',exact:true}).click();
  expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='discard_native_selection').args)).toEqual({request:{}});
  expect(await page.evaluate(()=>testBridge.appliedTargets)).toEqual([]);
});

test('new_native_selection_clears_imported_entry_bindings_and_mapping_history',async({page})=>{
  await open(page);
  await page.getByRole('button',{name:'Choose files',exact:true}).click();
  const recipe=await page.evaluate(async()=>{const {defaultRecipe}=await import('/recipe/fields.js');const value=defaultRecipe();const id='00000000-0000-4000-8000-000000000088';value.selection=[id];value.numbering_order=[id];value.selection_filter.excluded_ids=[id];value.companions={type:'explicit',associations:[{primary:id,companions:[]}]};return value;});
  await page.getByLabel('Import recipe file').setInputFiles({name:'recipe.json',mimeType:'application/json',buffer:Buffer.from(JSON.stringify(recipe))});
  await expect(page.getByText('Imported recipe. Save as a new preset to keep it.',{exact:true})).toBeVisible();
  await page.getByLabel('Mapping text').fill('stale.mp4,stale-target.mp4');
  await page.getByRole('button',{name:'Choose folders',exact:true}).click();
  await expect(page.getByLabel('Mapping text')).toHaveValue('');
  await expect(page.getByRole('button',{name:'Undo edit',exact:true})).toBeDisabled();
  await page.getByLabel('Select original.mp4').check();await page.getByTestId('rename-preview').click();
  const request=await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request);const sent=JSON.parse(request.recipeJson);
  expect(request.mapping).toBeNull();expect(sent.selection_filter.excluded_ids).toEqual([]);expect(sent.companions.associations).toEqual([]);
  expect(sent.selection).toEqual([fixture.row.itemId]);expect(sent.numbering_order).not.toContain('00000000-0000-4000-8000-000000000088');
});


test('selection_waiting_at_acceptance_can_be_discarded',async({page})=>{
  await open(page);await page.evaluate(()=>{window.nativeAcceptBusy=true;testBridge.pendingNative={selectionId:'00000000-0000-4000-8000-000000000099'};});
  await expect(page.getByTestId('rename-status')).toContainText('Native acceptance is busy');
  await page.getByRole('button',{name:'Discard waiting native selection',exact:true}).click();
  expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='discard_native_selection').args)).toEqual({request:{}});
  expect(await page.evaluate(()=>testBridge.appliedTargets)).toEqual([]);
});
