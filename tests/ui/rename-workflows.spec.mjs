import {test,expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {desktopBridge} from './fixtures/desktop-bridge.js';
const fixture=JSON.parse(await readFile(new URL('./fixtures/command-contracts.json',import.meta.url)));
async function start(page){await page.addInitScript(desktopBridge,fixture);await page.goto('/');await page.getByRole('button',{name:'Rename',exact:true}).click();await page.getByRole('button',{name:'Choose folders',exact:true}).click();await page.getByLabel('Select original.mp4').check();}
test('paged_preview_search_sort_highlights_and_all_selected_summary',async({page})=>{
 const errors=[];page.on('pageerror',e=>errors.push(e.message));await start(page);await page.evaluate(()=>{testBridge.pageRows=205;testBridge.summary={...testBridge.summary,selected:'205',files:'204',folders:'1',carriedDescendants:'17',companions:'2',metadataActions:'5'};});await page.getByTestId('rename-preview').click();
 await expect(page.getByTestId('rename-row')).toHaveCount(200);await expect(page.locator('.rename-summary')).toContainText('205 selected across all pages · 204 files · 1 folders · 17 carried descendants · 2 companions · 5 metadata actions');await expect(page.locator('.rename-table mark')).not.toHaveCount(0);
 await page.getByLabel('Preview search',{exact:true}).fill('film');await page.getByRole('button',{name:'Search preview',exact:true}).click();
 await page.getByLabel('Preview sort',{exact:true}).selectOption('status');
 expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='rename_page').args.request.query)).toMatchObject({limit:200,filter:'film',sort:'status'});
 const count=await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='start_rename_preview').length);expect(count).toBe(1);
 await page.getByRole('button',{name:'Next page',exact:true}).click();await expect(page.getByTestId('rename-row')).toHaveCount(5);expect(errors).toEqual([]);
});
test('cancel_revokes_even_a_terminal_completion_already_in_flight',async({page})=>{
 await start(page);await page.evaluate(()=>testBridge.delayStatusOnce=true);await page.getByTestId('rename-preview').click();await expect.poll(()=>page.evaluate(()=>testBridge.pendingStatuses.length)).toBe(1);
 await page.getByTestId('rename-cancel').click();await page.evaluate(()=>testBridge.pendingStatuses.shift()());
 await expect(page.getByTestId('rename-preview')).toBeEnabled();await expect(page.getByTestId('rename-apply')).toBeDisabled();
});
test('mapping_paste_and_manual_order_use_ids_and_explicit_preview',async({page})=>{
 await start(page);await expect(page.getByLabel('Mapping format')).toHaveCount(1);await page.getByLabel('Mapping format').selectOption('names');await page.getByLabel('Mapping text').fill('new-name.mkv');await page.getByTestId('rename-preview').click();
 expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request)).toMatchObject({mapping:{format:'names',text:'new-name.mkv'}});
 await page.getByLabel('Override name').fill('manual.mkv');await page.getByRole('button',{name:'Use row override',exact:true}).click();await expect(page.getByTestId('rename-apply')).toBeDisabled();await page.getByTestId('rename-preview').click();
 expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request.rowOverrides)).toEqual([{entryId:fixture.row.itemId,newName:'manual.mkv'}]);
});
test('hostile_names_explanations_and_metadata_render_as_text_at_narrow_width',async({page})=>{
 const hostile={...fixture,row:{...fixture.row,newName:'<img src=x onerror=alert(1)>.mkv',metadata:[{original:{modifiedNs:'<img src=x onerror=alert(1)>',createdNs:null,accessedNs:null,windowsAttributes:null},requested:[{type:'timestamp',field:'modified',valueNs:'<svg onload=alert(1)>'}]}],explanations:[{ruleId:'<script>x</script>',before:'old',after:'<svg onload=alert(1)>'}]}};
 await page.setViewportSize({width:390,height:844});await page.addInitScript(desktopBridge,hostile);await page.goto('/');await page.getByRole('button',{name:'Rename',exact:true}).click();await page.getByRole('button',{name:'Choose folders',exact:true}).click();await page.getByLabel('Select original.mp4').check();await page.getByTestId('rename-preview').click();
 await expect(page.getByTestId('rename-row')).toHaveCount(1);await expect(page.locator('.rename-rows script,.rename-rows img,.rename-rows svg')).toHaveCount(0);
 await page.getByText('Metadata changes',{exact:true}).click();await page.getByText('Rule explanations',{exact:true}).click();await expect(page.locator('.rename-rows')).toContainText('<svg onload=alert(1)>');
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
 await page.getByTestId('rename-apply').focus();await page.keyboard.press('Enter');await expect(page.getByRole('dialog')).toBeVisible();await page.keyboard.press('Escape');await expect(page.getByTestId('rename-apply')).toBeFocused();
});
test('older_job_finishes_after_newer_recipe_never_restores_apply_or_nonce',async({page})=>{
 await start(page);await page.evaluate(()=>testBridge.delayStatusOnce=true);await page.getByTestId('rename-preview').click();await expect.poll(()=>page.evaluate(()=>testBridge.pendingStatuses.length)).toBe(1);
 await page.getByLabel('Naming rule',{exact:true}).selectOption('fixedStem');await page.getByLabel('New stem',{exact:true}).fill('latest');await page.getByTestId('rename-preview').click();await expect(page.getByTestId('rename-apply')).toBeEnabled();
 const latest=await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='rename_page').args.request.planRef);
 await page.evaluate(()=>testBridge.pendingStatuses.shift()());await page.getByTestId('rename-apply').click();await page.getByRole('button',{name:'Apply reviewed rename',exact:true}).click();
 await expect.poll(()=>page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='apply_rename')?.args.request.planRef)).toEqual(latest);
});
test('selected_ownership_and_destination_remain_portable_recipe_data',async({page})=>{
 await page.addInitScript(desktopBridge,fixture);await page.addInitScript(()=>{const invoke=window.__TAURI__.core.invoke;window.__TAURI__.core.invoke=(command,args)=>command==='select_destination'?Promise.resolve({grantId:'00000000-0000-4000-8000-000000000098',display:'Native destination'}):invoke(command,args);});
 await page.goto('/');await page.getByRole('button',{name:'Rename',exact:true}).click();await page.getByRole('button',{name:'Choose folders',exact:true}).click();await page.getByLabel('Select original.mp4').check();
 await page.getByLabel('Companion policy').selectOption('explicit');await page.getByRole('button',{name:'Add primary owner',exact:true}).click();
 await page.getByLabel('Destination action').selectOption('copy_to');await page.getByRole('button',{name:'Choose destination folder',exact:true}).click();await expect(page.getByTestId('rename-status')).toContainText('Native destination');
 await page.getByTestId('rename-preview').click();const request=await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request);const recipe=JSON.parse(request.recipeJson);
 expect(recipe.companions).toEqual({type:'explicit',associations:[{primary:fixture.row.itemId,companions:[]}]});expect(recipe.selection).toContain(fixture.row.itemId);expect(recipe.destination).toEqual({type:'copy_to',grant_id:'00000000-0000-4000-8000-000000000098',layout:{type:'preserve_hierarchy'}});
});

test('filesystem_undo_is_reviewed_with_copy_deletion_disclosure',async({page})=>{
 await start(page);await page.evaluate(()=>testBridge.history={items:[{operationId:'00000000-0000-4000-8000-000000000006',state:'completed',completedSteps:'2',undoAvailable:true}],pendingOperations:[],legacyUnresolved:false});
 await page.getByRole('button',{name:'Refresh history',exact:true}).click();await page.getByRole('button',{name:'Preview Undo',exact:true}).click();await expect(page.getByTestId('rename-apply')).toBeEnabled();await page.getByTestId('rename-apply').click();
 await expect(page.getByRole('dialog')).toContainText('Review filesystem Undo');await expect(page.getByRole('dialog')).toContainText('created copies with matching identity and full digest (1 in this plan)');await expect(page.getByRole('dialog')).toContainText('owned empty directories (1 in this plan)');
 await page.getByRole('button',{name:'Apply reviewed Undo',exact:true}).click();await expect(page.getByTestId('rename-status')).toContainText('Completed');
 expect(await page.evaluate(()=>testBridge.calls.find(c=>c.command==='start_undo').args.request.operationId)).toBe('00000000-0000-4000-8000-000000000006');
});
test('explicit_dependency_group_exclusion_removes_non_displayed_rows',async({page})=>{
 await page.addInitScript(desktopBridge,fixture);await page.addInitScript(()=>{
  const invoke=window.__TAURI__.core.invoke;
  window.__TAURI__.core.invoke=(command,args)=>{
   if(command==='inventory_page'||command==='rename_page'){
    const start=Number(args.request.query.cursor||0),end=Math.min(start+200,205),items=Array.from({length:end-start},(_,i)=>{const itemId='00000000-0000-4000-8000-'+String(start+i).padStart(12,'0');return{itemId,displayName:`item${start+i}.mkv`,oldName:`item${start+i}.mkv`,newName:`new${start+i}.mkv`,oldDisplay:'source',newDisplay:'target',sizeBytes:'1',kind:'file',status:'changed',carriedDescendants:'0',issues:[],association:{groupId:'one-group',primaryId:itemId,isCompanion:false}};});
    return Promise.resolve({items,nextCursor:end<205?String(end):null,revision:args.request.query.expectedRevision});
   }
   return invoke(command,args);
  };
 });
 await page.goto('/');await page.getByRole('button',{name:'Rename',exact:true}).click();await page.getByRole('button',{name:'Choose folders',exact:true}).click();await page.getByText('Selection and numbering order',{exact:true}).click();await page.getByRole('button',{name:'Select all inventory items',exact:true}).click();await page.getByTestId('rename-preview').click();
 await expect(page.getByTestId('rename-row')).toHaveCount(200);expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request.selectedIds.length)).toBe(205);
 await page.getByRole('button',{name:'Exclude dependency group',exact:true}).first().click();await expect(page.getByTestId('rename-apply')).toBeDisabled();await expect(page.getByTestId('rename-preview')).toBeDisabled();await expect(page.getByTestId('rename-status')).toContainText('Excluded 205');
});
test('manual_order_and_explicit_adoption_are_separate_from_display_sort',async({page})=>{
 await page.addInitScript(desktopBridge,fixture);await page.addInitScript(()=>{
   const invoke=window.__TAURI__.core.invoke;
   window.__TAURI__.core.invoke=(command,args)=>{
     if(command==='inventory_page'){
       const a={itemId:'00000000-0000-4000-8000-000000000010',displayName:'movie10.mkv',sizeBytes:'10',kind:'file'},b={itemId:'00000000-0000-4000-8000-000000000002',displayName:'movie2.mkv',sizeBytes:'2',kind:'file'};
       return Promise.resolve({items:args.request.query.sort==='natural'?[b,a]:[a,b],nextCursor:null,revision:args.request.query.expectedRevision});
     }
     if(command==='rename_page')return invoke(command,args).then(result=>({...result,items:['00000000-0000-4000-8000-000000000002','00000000-0000-4000-8000-000000000010'].map((itemId,i)=>({...result.items[0],itemId,oldName:i?'movie10.mkv':'movie2.mkv',association:{primaryId:itemId,groupId:itemId,isCompanion:false}}))}));
     return invoke(command,args);
   };
 });
 await page.goto('/');await page.getByRole('button',{name:'Rename',exact:true}).click();await page.getByRole('button',{name:'Choose folders',exact:true}).click();await page.getByText('Selection and numbering order',{exact:true}).click();await page.getByRole('button',{name:'Select all inventory items',exact:true}).click();
 await page.getByTestId('rename-preview').click();const original=await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request.selectedIds);expect(original).toEqual(['00000000-0000-4000-8000-000000000010','00000000-0000-4000-8000-000000000002']);
 await page.getByLabel('Preview sort',{exact:true}).selectOption('status');await expect(page.getByTestId('rename-apply')).toBeEnabled();expect(await page.evaluate(()=>testBridge.calls.filter(c=>c.command==='start_rename_preview').length)).toBe(1);
 await page.getByRole('button',{name:'Adopt display order for numbering',exact:true}).click();await expect(page.getByTestId('rename-apply')).toBeDisabled();await page.getByTestId('rename-preview').click();expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request.selectedIds)).toEqual([...original].reverse());
 await page.getByRole('button',{name:'Move item 2 up',exact:true}).click();await page.getByTestId('rename-preview').click();expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request.selectedIds)).toEqual(original);
});

test('recovery_distinguishes_frozen_original_factual_prestate_and_recorded_poststate',async({page})=>{
 await start(page);await page.evaluate(()=>{testBridge.history={items:[],pendingOperations:['pending'],legacyUnresolved:false};testBridge.recoveryMetadata={original:{modifiedNs:'100',createdNs:null,accessedNs:'10',windowsAttributes:null},factualBefore:{modifiedNs:'200',createdNs:null,accessedNs:'20',windowsAttributes:null},observedAfter:{modifiedNs:'300',createdNs:null,accessedNs:'20',windowsAttributes:null},requested:[{type:'timestamp',field:'modified',valueNs:'300'}]};});
 await page.getByRole('button',{name:'Refresh history',exact:true}).click();await page.getByRole('button',{name:'Review recovery pending',exact:true}).click();
 await expect(page.locator('.rename-recovery')).toContainText('Frozen operation original: modified 100');await expect(page.locator('.rename-recovery')).toContainText('Durable factual pre-set: modified 200');await expect(page.locator('.rename-recovery')).toContainText('Recorded setter poststate: modified 300');
});
test('excluding_one_linked_companion_removes_only_its_explicit_binding',async({page})=>{
 await page.addInitScript(desktopBridge,fixture);await page.addInitScript(fixture=>{
  const invoke=window.__TAURI__.core.invoke,companion='00000000-0000-4000-8000-000000000099';
  window.__TAURI__.core.invoke=(command,args)=>{
   if(command==='inventory_page')return Promise.resolve({items:[{itemId:fixture.row.itemId,displayName:'original.mp4',kind:'file',sizeBytes:'1'},{itemId:companion,displayName:'original.srt',kind:'file',sizeBytes:'1'}],nextCursor:null,revision:args.request.query.expectedRevision});
   if(command==='rename_page')return invoke(command,args).then(value=>({...value,items:[fixture.row,{...fixture.row,itemId:companion,oldName:'original.srt',newName:'renamed.srt'}]}));
   return invoke(command,args);
  };
 },fixture);
 await page.goto('/');await page.getByRole('button',{name:'Rename',exact:true}).click();await page.getByRole('button',{name:'Choose folders',exact:true}).click();await page.getByLabel('Select original.mp4').check();await page.getByLabel('Select original.srt').check();
 await page.getByLabel('Companion policy').selectOption('explicit');await page.getByRole('button',{name:'Add primary owner',exact:true}).click();await page.getByRole('button',{name:'Add companion to owner 1',exact:true}).click();await page.getByTestId('rename-preview').click();await page.getByRole('button',{name:'Exclude row',exact:true}).nth(1).click();await page.getByTestId('rename-preview').click();
 const request=await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='start_rename_preview').args.request);expect(request.selectedIds).toEqual([fixture.row.itemId]);expect(JSON.parse(request.recipeJson).companions).toEqual({type:'explicit',associations:[{primary:fixture.row.itemId,companions:[]}]});
});
test('mapping_file_and_both_exports_use_reviewed_native_reference',async({page})=>{
 await start(page);await page.getByLabel('Import mapping file',{exact:true}).setInputFiles({name:'mapping.csv',mimeType:'text/csv',buffer:Buffer.from('old_path,new_path\noriginal.mp4,renamed.mp4\n')});await expect(page.getByLabel('Mapping text')).toHaveValue('old_path,new_path\noriginal.mp4,renamed.mp4\n');await page.getByTestId('rename-preview').click();
 const reference=await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='rename_page').args.request.planRef);
 for(const [name,mode,filename] of [['Export exact mapping','exactMapping','rename-mapping.csv'],['Export reviewed report','spreadsheetSafe','rename-report.csv']]){
  const downloaded=page.waitForEvent('download');await page.getByRole('button',{name,exact:true}).click();const download=await downloaded;expect(download.suggestedFilename()).toBe(filename);expect(await readFile(await download.path(),'utf8')).toContain('original.mp4,renamed.mp4');
  expect(await page.evaluate(()=>testBridge.calls.findLast(c=>c.command==='export_rename_plan').args.request)).toEqual({planRef:reference,mode});
 }
});
