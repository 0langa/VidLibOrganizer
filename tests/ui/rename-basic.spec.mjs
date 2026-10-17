import { test, expect } from '@playwright/test';
import { desktopBridge } from './fixtures/desktop-bridge.js';
import { readFile } from 'node:fs/promises';
const fixture = JSON.parse(await readFile(new URL('./fixtures/command-contracts.json', import.meta.url)));
async function open(page, desktop=true) { if(desktop) await page.addInitScript(desktopBridge, fixture); await page.goto('/'); await expect(page.getByRole('button',{name:'Rename',exact:true})).toHaveCount(1); await page.getByRole('button',{name:'Rename',exact:true}).click(); }
async function prepare(page) { await open(page); await page.getByRole('button',{name:'Choose folders',exact:true}).click(); await expect(page.getByLabel('Select original.mp4')).toBeVisible(); await page.getByLabel('Select original.mp4').check(); await page.getByLabel('Naming rule').selectOption('fixedStem'); await page.getByLabel('New stem').fill('renamed'); await page.getByTestId('rename-preview').click(); await expect(page.getByText('renamed.mp4',{exact:true})).toBeVisible(); }
test('rename_basic_preview_confirm_cancel',async({page})=>{ await prepare(page); await expect(page.getByTestId('rename-apply')).toBeEnabled(); await page.getByTestId('rename-apply').click(); await expect(page.getByRole('dialog')).toBeVisible(); await page.getByRole('button',{name:'Cancel',exact:true}).click(); expect(await page.evaluate(()=>testBridge.appliedTargets)).toEqual([]); await page.getByTestId('rename-apply').click(); await page.getByRole('button',{name:'Apply reviewed rename'}).click(); await expect(page.getByTestId('rename-status')).toContainText('Completed'); expect(await page.evaluate(()=>testBridge.appliedTargets.length)).toBe(1); });
test('preview_response_after_edit_is_ignored',async({page})=>{
  await prepare(page);await page.evaluate(()=>testBridge.delayStatusOnce=true);
  await page.getByTestId('rename-preview').click();
  await expect.poll(()=>page.evaluate(()=>testBridge.pendingStatuses.length)).toBe(1);
  await page.getByLabel('New stem').fill('later');
  await page.evaluate(()=>testBridge.pendingStatuses.shift()());
  await expect.poll(()=>page.evaluate(()=>testBridge.releasedStatuses)).toBe(1);
  await expect(page.getByTestId('rename-apply')).toBeDisabled();
  await expect(page.getByTestId('rename-status')).toContainText('Preview again');
  expect(await page.evaluate(()=>testBridge.appliedTargets)).toEqual([]);
});
test('browser_preview_cannot_apply',async({page})=>{ await open(page,false); await page.getByRole('button',{name:'Preview sample rename'}).click(); await expect(page.getByTestId('rename-apply')).toBeDisabled(); await expect(page.getByTestId('rename-status')).toContainText('read only'); });
test('exact_bytes_above_js_safe_integer',async({page})=>{ await prepare(page); await expect(page.getByTestId('exact-bytes')).toContainText('9,007,199,254,740,993'); });
test('keyboard_dialog_focus_and_escape',async({page})=>{ await prepare(page); await page.getByTestId('rename-apply').focus(); await page.keyboard.press('Enter'); await expect(page.getByRole('button',{name:'Cancel',exact:true})).toBeFocused(); await page.keyboard.press('Escape'); await expect(page.getByRole('dialog')).not.toBeVisible(); await expect(page.getByTestId('rename-apply')).toBeFocused(); expect(await page.evaluate(()=>testBridge.appliedTargets)).toEqual([]); });
test('navigation_cancels_preview_and_tears_down_polling',async({page})=>{ await prepare(page); await page.evaluate(()=>testBridge.delayPreview=true); await page.getByTestId('rename-preview').click(); await expect(page.getByTestId('rename-cancel')).toBeEnabled(); await page.getByRole('button',{name:'Duplicate finder',exact:true}).click(); await expect.poll(()=>page.evaluate(()=>testBridge.cancelled.length)).toBeGreaterThan(0); const polls=await page.evaluate(()=>testBridge.calls.filter(call=>call.command==='job_status').length); await page.waitForTimeout(400); expect(await page.evaluate(()=>testBridge.calls.filter(call=>call.command==='job_status').length)).toBe(polls); await page.evaluate(()=>testBridge.pending.forEach(done=>done())); await page.getByRole('button',{name:'Rename',exact:true}).click(); await expect(page.getByTestId('rename-apply')).toBeDisabled(); });
test('finder_sample_interaction_is_preserved',async({page})=>{ await page.goto('/'); await page.getByRole('button',{name:'Preview sample results'}).click(); await expect(page.getByText('Coastal road — final.mp4',{exact:true}).first()).toBeVisible(); await page.getByLabel('Search result filenames and paths').fill('Autumn'); await expect(page.getByText('Autumn in the mountains.mov',{exact:true}).first()).toBeVisible(); await expect(page.getByText('Coastal road — final.mp4',{exact:true})).toHaveCount(0); });
test('only_one_200_row_page_is_rendered',async({page})=>{ await prepare(page); await page.evaluate(()=>testBridge.pageRows=350); await page.getByTestId('rename-preview').click(); await expect(page.getByTestId('exact-bytes')).toHaveCount(200); await page.getByRole('button',{name:'Next page',exact:true}).click(); await expect(page.getByTestId('exact-bytes')).toHaveCount(150); });
test('confirmation_response_after_edit_is_ignored',async({page})=>{ await prepare(page); await page.evaluate(()=>testBridge.delayConfirm=true); await page.getByTestId('rename-apply').click(); await expect.poll(()=>page.evaluate(()=>testBridge.pendingConfirm.length)).toBe(1); await page.getByLabel('New stem').fill('later'); await page.evaluate(()=>testBridge.pendingConfirm.forEach(done=>done())); await expect.poll(()=>page.evaluate(()=>testBridge.finishedConfirm)).toBe(1); await expect(page.getByRole('dialog')).toHaveCount(0); await expect(page.getByTestId('rename-apply')).toBeDisabled(); expect(await page.evaluate(()=>testBridge.appliedTargets)).toEqual([]); });
test('recovery_continuation_and_unknown_legacy_are_factual',async({page})=>{ await open(page); await page.evaluate(()=>testBridge.history={items:[{operationId:'partial',state:'failed',completedSteps:'1',undoAvailable:true}],nextCursor:null,pendingOperations:['pending'],legacyUnresolved:true}); await page.getByRole('button',{name:'Refresh history',exact:true}).click(); await expect(page.getByText('Legacy interrupted operation remains unresolved. No per-item evidence is available.')).toBeVisible(); await expect(page.getByText('Failed · 1 completed steps · partial')).toBeVisible(); await expect(page.getByRole('button',{name:'Review recovery pending',exact:true})).toBeDisabled(); await page.evaluate(()=>testBridge.history.legacyUnresolved=false); await page.getByRole('button',{name:'Refresh history',exact:true}).click(); await expect(page.getByRole('button',{name:'Review recovery pending',exact:true})).toBeEnabled(); await page.getByRole('button',{name:'Review recovery pending',exact:true}).click(); await expect(page.getByText('Continue durable Restore · 0 unresolved groups · 1 actions')).toBeVisible(); await expect(page.getByRole('button',{name:'Restore interrupted group',exact:true})).toBeVisible(); await expect(page.getByRole('button',{name:'Resume compiled operation',exact:true})).toHaveCount(0); });
test('delayed_start_reserves_pending_before_reply',async({page})=>{
  await prepare(page);await page.evaluate(()=>testBridge.delayStartOnce=true);await page.getByTestId('rename-preview').click();
  await expect.poll(()=>page.evaluate(()=>testBridge.pendingStarts.length)).toBe(1);
  await expect(page.getByTestId('rename-preview')).toBeDisabled();
  const starts=await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='start_rename_preview').length);
  await page.getByTestId('rename-preview').dispatchEvent('click');
  expect(await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='start_rename_preview').length)).toBe(starts);
  await page.evaluate(()=>testBridge.pendingStarts.shift()());await expect(page.getByTestId('rename-apply')).toBeEnabled();
  await page.getByTestId('rename-apply').click();await page.getByRole('button',{name:'Apply reviewed rename'}).click();await expect(page.getByTestId('rename-status')).toContainText('Completed');
});
test('delayed_start_after_edit_cannot_replace_newer_preview',async({page})=>{
  await prepare(page);await page.evaluate(()=>testBridge.delayStartOnce=true);await page.getByTestId('rename-preview').click();await expect.poll(()=>page.evaluate(()=>testBridge.pendingStarts.length)).toBe(1);
  await page.getByLabel('New stem').fill('newer');await expect(page.getByTestId('rename-preview')).toBeEnabled();await page.getByTestId('rename-preview').click();await expect(page.getByTestId('rename-apply')).toBeEnabled();
  const newest=await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='rename_page').at(-1).args.request.planRef.planId);
  await page.evaluate(()=>testBridge.pendingStarts.shift()());await expect.poll(()=>page.evaluate(()=>testBridge.releasedStarts)).toBe(1);
  await page.getByTestId('rename-apply').click();await page.getByRole('button',{name:'Apply reviewed rename'}).click();await expect(page.getByTestId('rename-status')).toContainText('Completed');
  expect(await page.evaluate(()=>testBridge.appliedTargets.at(-1))).toBe(newest);
});
test('recovery_navigation_reaches_last_action_without_new_review',async({page})=>{
  await open(page);await page.evaluate(()=>{testBridge.recoveryActions=205;testBridge.history={items:[],nextCursor:null,pendingOperations:['pending'],legacyUnresolved:false};});await page.getByRole('button',{name:'Refresh history'}).click();await page.getByRole('button',{name:'Review recovery pending'}).click();
  await expect(page.getByText('before-000 → after-000: planned',{exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'Next recovery page',exact:true})).toHaveCount(1);
  await page.getByRole('button',{name:'Next recovery page',exact:true}).click();await expect(page.getByText('before-204 → after-204: planned',{exact:true})).toBeVisible();
  expect(await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='recovery_preview').length)).toBe(1);expect(await page.evaluate(()=>new Set(testBridge.recoveryPages.map(p=>p.recoveryRef)).size)).toBe(1);
});
test('history_query_refreshes_changed_revision_and_applies_display_filter',async({page})=>{
  await open(page);await page.evaluate(()=>{testBridge.history={items:Array.from({length:205},(_,i)=>({operationId:'40000000-0000-4000-8000-'+String(i).padStart(12,'0'),state:i===4?'failed':'completed',completedSteps:'1',undoAvailable:false})),pendingOperations:[],legacyUnresolved:false};});
  await page.getByRole('button',{name:'Refresh history'}).click();await expect(page.getByTestId('history-item')).toHaveCount(200);
  await page.evaluate(()=>testBridge.historyStaleOnce=true);await page.getByRole('button',{name:'Next history page'}).click();
  await expect.poll(()=>page.evaluate(()=>testBridge.calls.filter(c=>c.command==='operation_history').at(-1).args.request.query.expectedRevision)).toBe('2');
  expect(await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='operation_history').at(-1).args.request.query.cursor)).toBeNull();
  await page.getByLabel('History sort').selectOption('state');await page.getByLabel('Filter history').fill('failed');await page.getByRole('button',{name:'Filter history',exact:true}).click();
  await expect(page.getByTestId('history-item')).toHaveCount(1);await expect(page.getByTestId('history-item')).toContainText('Failed');
  expect(await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='operation_history').at(-1).args.request.query)).toMatchObject({limit:200,sort:'state',filter:'failed',expectedRevision:'2'});
});
test('old_recovery_page_response_is_ignored_after_edit',async({page})=>{
  await open(page);await page.evaluate(()=>{testBridge.recoveryActions=205;testBridge.history={items:[],pendingOperations:['pending'],legacyUnresolved:false};});await page.getByRole('button',{name:'Refresh history'}).click();await page.getByRole('button',{name:'Review recovery pending'}).click();await expect(page.getByTestId('recovery-action')).toHaveCount(200);
  await page.evaluate(()=>testBridge.delayRecoveryPageOnce=true);await page.getByRole('button',{name:'Next recovery page'}).click();await expect.poll(()=>page.evaluate(()=>testBridge.pendingRecoveryPages.length)).toBe(1);await page.getByLabel('Naming rule').selectOption('fixedStem');await page.evaluate(()=>testBridge.pendingRecoveryPages.shift()());await expect.poll(()=>page.evaluate(()=>testBridge.releasedRecoveryPages)).toBe(1);await expect(page.getByTestId('recovery-action')).toHaveCount(0);await expect(page.getByRole('button',{name:'Restore interrupted group',exact:true})).toHaveCount(0);
});
