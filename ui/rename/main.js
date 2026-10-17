import { call, isDesktop } from '../api.js';
import { $, el } from '../dom.js';
import { formatExactBytes } from '../exact.js';
import { watchJob } from '../jobs.js';
import { createRecipeEditor } from '../recipe/editor.js';
import { defaultRecipe } from '../recipe/fields.js';
import { renderPreview, reviewConfirmation, appendSummary, appendRecoveryMetadata } from './preview.js';
import { createHistory } from './history.js';

/**
 * @typedef {{sessionId: string, planId: string, revision: string}} PlanReference
 * @typedef {{sequence: number, generation: number, revision: string, abort: AbortController, jobId: ?string, cancelRequested: boolean, mutation: boolean}} RenameTask
 * @typedef {{planRef: PlanReference, counts: {changed: string, unchanged: string, blocked: string, needsReview: string}, summary: Object}} PreviewResult
 * All native references are opaque. Display paths are never reused as permissions.
 */
export function formatOperationState(value) {
  return value ? value[0].toUpperCase() + value.slice(1).replace(/[A-Z]/g, letter => ` ${letter.toLowerCase()}`) : 'Unknown';
}
export function createRename() {
  const host = $('#view-rename');
  const status = host.querySelector('[data-testid=rename-status]');
  const apply = host.querySelector('[data-testid=rename-apply]');
  const preview = host.querySelector('[data-testid=rename-preview]');
  const cancel = host.querySelector('[data-testid=rename-cancel]');
  const rows = host.querySelector('.rename-rows');
  const paging = host.querySelector('.rename-paging');
  const sourceList = host.querySelector('.rename-sources');
  const recoveryHost = host.querySelector('.rename-recovery');
  let initialized = !isDesktop, revision = 0n, generation = 0, sequence = 0;
  let ready = Promise.resolve(), active = null, planRef = null, counts = null;
  let inventoryReady = false, recoveryView = null;
  const selected = new Set(), inventoryItems = new Map(), overrides = new Map();
  let summary = null, mapping = null, orderPage = 0;
  const reviewSummary = el('div', 'rename-summary'); rows.before(reviewSummary);
  const viewTools = el('fieldset', 'rename-toolbar'); viewTools.append(el('legend', '', 'Preview display')); rows.before(viewTools);
  const displaySearch = el('input'); displaySearch.type='search'; displaySearch.setAttribute('aria-label','Preview search');
  const displaySort = el('select'); displaySort.setAttribute('aria-label','Preview sort');
  for(const [value,title] of [['name','Name'],['natural','Natural name'],['path','Path'],['status','Status'],['size','Size']]) {const o=el('option','',title);o.value=value;displaySort.append(o);}
  const statusFilter = el('select'); statusFilter.setAttribute('aria-label','Preview status');
  for(const [value,title] of [['','All'],['changed','Changed'],['unchanged','Unchanged'],['needsReview','Needs review'],['blocked','Blocked']]) {const o=el('option','',title);o.value=value;statusFilter.append(o);}
  const searchButton = el('button','button','Search preview'); viewTools.append(displaySearch,displaySort,statusFilter,searchButton);
  const orderHost=el('details');orderHost.append(el('summary','','Selection and numbering order'));host.querySelector('.rename-editor').before(orderHost);
  const orderRows=el('div');orderHost.append(orderRows);
  const selectedCount=el('p');orderHost.before(selectedCount);
  const orderButton=(label,action)=>{const b=el('button','button',label);b.type='button';b.addEventListener('click',action);orderHost.append(b);return b;};
  const selectAll=orderButton('Select all inventory items',()=>collectInventory(false));
  const adoptOrder=orderButton('Adopt display order for numbering',()=>collectInventory(true));
  orderButton('Clear selection',()=>{if(active?.mutation)return;const ids=[...selected];selected.clear();recipe.excludeItems(ids);syncSelection();edited();});
  const recipe = createRecipeEditor({ recipe: defaultRecipe(), onChange: edited, chooseDestination });
  const editorHost = host.querySelector('.rename-editor');
  const presets = el('fieldset', 'recipe-presets');
  presets.append(el('legend', '', 'Recipe presets'));
  const presetChoice = el('select'); presetChoice.setAttribute('aria-label', 'Saved preset');
  const blank = el('option', '', 'Choose a saved preset'); blank.value = ''; presetChoice.append(blank);
  const presetStatus = el('p'); presetStatus.dataset.testid = 'recipe-preset-status';
  let attachedPreset = null, presetBusy = false, presetCursor = null;
  function presetButton(title, action) {
    const button = el('button', 'button', title); button.type = 'button';
    button.addEventListener('click', action); presets.append(button); return button;
  }
  function rememberPreset(value) {
    let option = [...presetChoice.options].find(option => option.value === value.presetId);
    if (!option) { option = el('option'); option.value = value.presetId; presetChoice.append(option); }
    option.textContent = `${value.presetId.slice(0, 8)} · revision ${value.revision}`;
    presetChoice.value = value.presetId;
  }
  async function presetAction(work) {
    if (presetBusy || !initialized || !isDesktop || active?.mutation) return;
    presetBusy = true; controls(); const ticket = generation;
    try { await work(() => ticket === generation); } catch (error) { if (ticket === generation) failed(error); }
    finally { presetBusy = false; controls(); }
  }
  const presetRevision=el('input');presetRevision.setAttribute('aria-label','Preset revision (blank for current)');presetRevision.inputMode='numeric';
  presets.append(presetChoice,presetRevision);
  presetButton('Refresh presets', () => presetAction(async current => {
    const result = await call('list_rename_presets', { request: { cursor: null } });
    if (!current()) return;
    presetChoice.replaceChildren(blank);
    result.items.forEach(rememberPreset); presetCursor = result.nextCursor;
  }));
  const nextPresets = presetButton('Next presets', () => presetAction(async current => {
    const result = await call('list_rename_presets', { request: { cursor: presetCursor } });
    if (!current()) return;
    presetChoice.replaceChildren(blank); result.items.forEach(rememberPreset); presetCursor = result.nextCursor;
  }));
  const loadPreset = presetButton('Load preset', () => presetAction(async current => {
    const result = await call('load_rename_preset', { request: { presetId: presetChoice.value, revision: presetRevision.value || null } });
    if (!current()) return;
    recipe.setRecipe(JSON.parse(result.recipeJson)); attachedPreset = result;
    presetStatus.textContent = `Attached preset ${result.presetId.slice(0, 8)} · revision ${result.revision}`;
    edited();
  }));
  function savePreset(fresh) {
    return presetAction(async current => {
      const result = await call('save_rename_preset', { request: { presetId: fresh ? null : attachedPreset.presetId, expectedRevision: fresh ? null : attachedPreset.revision, recipeJson: JSON.stringify(recipe.getRecipe()) } });
      if (!current()) return;
      attachedPreset = result; rememberPreset(result);
      presetStatus.textContent = `Saved preset ${result.presetId.slice(0, 8)} · revision ${result.revision}`;
    });
  }
  presetButton('Save new preset', () => savePreset(true));
  const saveRevision = presetButton('Save preset revision', () => savePreset(false));
  presetButton('Export recipe', () => presetAction(async current => {
    const result = await call('export_rename_recipe', { request: { recipeJson: JSON.stringify(recipe.getRecipe()) } });
    if (!current()) return;
    const url = URL.createObjectURL(new Blob([result.recipeJson], { type: 'application/json' }));
    const link = el('a'); link.href = url; link.download = 'rename-recipe.json'; link.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }));
  const importLabel = el('label', '', 'Import recipe file'), importFile = el('input');
  importFile.type = 'file'; importFile.accept = '.json,application/json'; importFile.setAttribute('aria-label', 'Import recipe file');
  importFile.addEventListener('change', () => {
    const file = importFile.files[0]; importFile.value = ''; if (!file) return;
    presetAction(async current => {
      if (file.size > 1024 * 1024) throw new Error('Recipe exceeds 1 MiB.');
      const result = await call('import_rename_recipe', { request: { recipeJson: await file.text() } });
      if (!current()) return;
      recipe.setRecipe(JSON.parse(result.recipeJson)); attachedPreset = null;
      presetStatus.textContent = 'Imported recipe. Save as a new preset to keep it.'; edited();
    });
  });
  importLabel.append(importFile); presets.append(importLabel, presetStatus);
  presetChoice.addEventListener('change', controls);
  editorHost.append(presets, el('p', '', 'Preview binds this recipe to the currently checked items in selection order. Saved exclusions remain and must match the current inventory.'), recipe.element);

  const mappingHost=el('fieldset','rename-mappings');mappingHost.append(el('legend','','Mappings and pasted names'));
  const mappingFormat=el('select');mappingFormat.setAttribute('aria-label','Mapping format');
  for(const [value,title] of [['csv','CSV old_path,new_path'],['tab','Tab delimited old_path,new_path'],['names','Pasted names in frozen selection order']]){const o=el('option','',title);o.value=value;mappingFormat.append(o);}
  const mappingText=el('textarea');mappingText.setAttribute('aria-label','Mapping text');
  const mappingFile=el('input');mappingFile.type='file';mappingFile.setAttribute('aria-label','Import mapping file');
  const mappingChanged=()=>{mapping=mappingText.value?{format:mappingFormat.value,text:mappingText.value}:null;edited();};
  mappingFormat.addEventListener('change',mappingChanged);mappingText.addEventListener('input',mappingChanged);
  mappingFile.addEventListener('change',async()=>{const file=mappingFile.files[0];if(!file)return;const ticket=generation;try{if(file.size>64*1024*1024)throw new Error('Mapping exceeds 64 MiB.');const text=await file.text();if(ticket!==generation)return;mappingText.value=text;mappingChanged();}catch(error){failed(error);}});
  mappingHost.append(mappingFormat,mappingText,mappingFile);editorHost.append(mappingHost);
  for(const [mode,label] of [['spreadsheetSafe','Export reviewed report'],['exactMapping','Export exact mapping']]) {
    const button=el('button','button',label);viewTools.append(button);
    button.addEventListener('click',()=>{if(!planRef || active || !isDesktop)return;const reference=planRef;perform(async task=>{const value=await call('export_rename_plan',{request:{planRef:reference,mode}});if(!current(task)||planRef!==reference)return;const url=URL.createObjectURL(new Blob([value.text],{type:'text/csv'}));const a=el('a');a.href=url;a.download=mode==='exactMapping'?'rename-mapping.csv':'rename-report.csv';a.click();setTimeout(()=>URL.revokeObjectURL(url),0);});});
  }
  function syncSelection() {
    selectedCount.textContent=`${selected.size} checked items. Display sorting never changes numbering until explicitly adopted.`;
    recipe.setSelectionItems([...selected].map(itemId=>inventoryItems.get(itemId)||{itemId,displayName:itemId}));
    orderRows.replaceChildren();
    const ids=[...selected];
    orderPage=Math.min(orderPage,Math.max(0,Math.ceil(ids.length/200)-1));
    ids.slice(orderPage*200,(orderPage+1)*200).forEach((id,offset)=>{const index=orderPage*200+offset;const row=el('p','',`${index+1}. ${inventoryItems.get(id)?.displayName || id} `);
      for(const [delta,label] of [[-1,'up'],[1,'down']]){const b=el('button','button',`Move item ${index+1} ${label}`);b.disabled=index+delta<0||index+delta>=ids.length;b.addEventListener('click',()=>{if(active?.mutation)return;[ids[index],ids[index+delta]]=[ids[index+delta],ids[index]];selected.clear();ids.forEach(id=>selected.add(id));syncSelection();edited();});row.append(b);}orderRows.append(row);});
    for(const [delta,label] of [[-1,'Previous order page'],[1,'Next order page']]) {const b=el('button','button',label);b.disabled=orderPage+delta<0||(orderPage+delta)*200>=ids.length;b.addEventListener('click',()=>{orderPage+=delta;syncSelection();});orderRows.append(b);}
  }
  function collectInventory(adopt) {
    if(!inventoryReady || active || !isDesktop)return;
    const reference=adopt?planRef:null;
    if(adopt && displaySort.value==='status' && !reference){say('Preview before adopting status order.');return;}
    perform(async task=>{
      const ids=[],seen=new Set();let cursor=null;
      do {
        const queryValue={...query(task,cursor),filter:'',status:null,sort:adopt?displaySort.value:'name'};
        const page=await call(reference?'rename_page':'inventory_page',{request:reference?{planRef:reference,query:queryValue}:{query:queryValue}});
        if(!current(task) || (reference && planRef!==reference))return;
        for(const item of page.items){
          if(!reference)inventoryItems.set(item.itemId,item);
          if((!adopt || selected.has(item.itemId)) && !seen.has(item.itemId)){ids.push(item.itemId);seen.add(item.itemId);}
        }
        cursor=page.nextCursor;
      } while(cursor);
      // Filtered-out checked items retain their relative order and selection.
      if(adopt)for(const id of selected)if(!seen.has(id))ids.push(id);
      selected.clear();ids.forEach(id=>selected.add(id));syncSelection();edited();
    });
  }
  function boundRecipe() {
    const value=recipe.getRecipe();value.selection=[...selected];
    const owned=new Set((value.companions.associations || []).flatMap(a=>a.companions));
    value.numbering_order=value.selection.filter(id=>!owned.has(id));return value;
  }
  function chooseDestination() {
    if(!isDesktop||active)return;
    perform(async task=>{const grant=await call('select_destination',{request:{revision:task.revision}});if(!current(task)||!grant)return;recipe.setDestination(grant.grantId);say(`Destination selected: ${grant.display}. Preview again.`);},{advance:true});
  }
  function refreshDisplay(){if(active||!planRef)return;const reference=planRef;perform(task=>planPage(task,reference));}
  searchButton.addEventListener('click',refreshDisplay);displaySort.addEventListener('change',refreshDisplay);statusFilter.addEventListener('change',refreshDisplay);
  displaySearch.addEventListener('keydown',event=>{if(event.key==='Enter'){event.preventDefault();refreshDisplay();}});
  function say(message) { status.textContent = message; }
  function failed(error) {
    if (error.name !== 'AbortError') say(`${error.message || String(error)}${error.phase ? ` (${error.phase})` : ''}`);
  }
  function controls() {
    const pending = active !== null;
    viewTools.disabled=pending;
    selectAll.disabled=!isDesktop||!inventoryReady||pending;adoptOrder.disabled=selectAll.disabled;
    mappingHost.disabled=!isDesktop||Boolean(active?.mutation);
    recipe.element.disabled = !initialized || Boolean(active?.mutation);
    presets.disabled = !isDesktop || !initialized || presetBusy || Boolean(active?.mutation);
    saveRevision.disabled = !attachedPreset;
    loadPreset.disabled = !presetChoice.value;
    nextPresets.disabled = !presetCursor;
    for (const action of ['folders', 'files']) host.querySelector(`[data-action=${action}]`).disabled = !isDesktop || !initialized || pending;
    host.querySelector('[data-action=selection]').disabled = !isDesktop || !inventoryReady || pending;
    preview.disabled = !isDesktop || !inventoryReady || !selected.size || pending;
    apply.disabled = !isDesktop || !planRef || !counts || counts.changed === '0' || counts.blocked !== '0' || counts.needsReview !== '0' || pending;
    cancel.disabled = !pending;
  }
  function current(task) {
    return task.generation === generation && task.sequence === sequence && !task.abort.signal.aborted;
  }
  /** Single reducer for revisions, current-job ownership and accepted plans. */
  function transition(type, payload) {
    switch(type) {
      case 'invalidate': {
        sequence++;
        const previous=active; active=null;
        if(previous){previous.abort.abort();if(previous.jobId)call('cancel_job',{request:{jobId:previous.jobId}}).catch(()=>{});}
        break;
      }
      case 'edit': transition('invalidate'); generation++; revision++; transition('clearPlan'); break;
      case 'clearPlan': planRef=null;counts=null;summary=null;break;
      case 'start': active={sequence:++sequence,generation,revision:revision.toString(),abort:new AbortController(),jobId:null,cancelRequested:false,mutation:false};return active;
      case 'finish': if(active===payload)active=null;break;
      case 'accept': if(!current(payload.task))return false;planRef=payload.result.planRef;counts=payload.result.counts;summary=payload.result.summary;return true;
      case 'session': revision=BigInt(payload);break;
      default: throw new Error('Unknown Rename transition');
    }
  }
  function invalidate(){transition('invalidate');}
  function edited() {
    transition('edit'); reviewSummary.replaceChildren(); recoveryView = null;
    recoveryHost.replaceChildren();
    const value = revision.toString();
    if (isDesktop && initialized) {
      ready = ready.catch(() => {}).then(() => call('update_rename_inputs', { request: { revision: value } }));
      ready.catch(failed);
    }
    say('Inputs changed. Preview again.'); controls();
  }
  // The ticket is reserved synchronously, before even awaiting the input queue.
  // It orders dispatch replies, terminal results, pages and confirmation dialogs.
  async function perform(work, { advance = false } = {}) {
    if (active) return;
    if (advance) edited();
    const task = transition('start'); controls();
    try {
      await ready;
      if (!current(task)) return;
      await work(task);
    } catch (error) {
      if (current(task)) failed(error);
    } finally {
      transition('finish',task);
      controls();
    }
  }
  async function run(task, command, request) {
    const id = await call(command, { request });
    if (!current(task)) {
      await call('cancel_job', { request: { jobId: id } });
      throw new DOMException('Superseded', 'AbortError');
    }
    task.jobId = id;
    if (task.cancelRequested) await call('cancel_job', { request: { jobId: id } });
    try {
      return await watchJob(id, {
        signal: task.abort.signal,
        onProgress: value => { if (current(task)) say(`${value.phase} · ${value.done}`); },
      });
    } catch (error) {
      if (current(task) && error.name === 'AbortError') say('Job cancelled. Completed work remains in history.');
      throw error;
    }
  }
  function query(task, cursor = null) {
    return { cursor, limit: 200, sort: displaySort.value, filter: displaySearch.value, status: statusFilter.value || null, expectedRevision: task.revision };
  }
  function renderPaging(page, load) {
    paging.replaceChildren();
    const first = el('button', 'button', 'First page');
    first.addEventListener('click', () => { if (!active) load(null); });
    if (page.hasPrevious) paging.append(first);
    if (page.nextCursor) {
      const next = el('button', 'button', 'Next page');
      next.addEventListener('click', () => { if (!active) load(page.nextCursor); });
      paging.append(next);
    }
  }
  async function inventoryPage(task, cursor = null) {
    const page = await call('inventory_page', { request: { query: { ...query(task, cursor), status:null, filter:'' } } });
    if (!current(task)) return;
    const table = el('table', 'rename-table'), body = el('tbody');
    for (const item of page.items.slice(0, 200)) {
      inventoryItems.set(item.itemId,item);
      const tr = el('tr'), cell = el('td'), label = el('label'), check = el('input');
      check.type = 'checkbox'; check.checked = selected.has(item.itemId);
      check.setAttribute('aria-label', `Select ${item.displayName}`);
      check.addEventListener('change', () => {
        if(check.checked) selected.add(item.itemId); else { selected.delete(item.itemId); overrides.delete(item.itemId); recipe.excludeItems([item.itemId]); }
        syncSelection(); edited();
      });
      label.append(check, document.createTextNode(item.displayName)); cell.append(label);
      tr.append(cell, el('td', '', formatExactBytes(item.sizeBytes))); body.append(tr);
    }
    table.append(body); rows.replaceChildren(table);
    renderPaging({ ...page, hasPrevious: cursor !== null }, next => perform(nextTask => inventoryPage(nextTask, next)));
  }
  async function planPage(task, reference, cursor = null) {
    try {
      const page = await call('rename_page', { request: { planRef: reference, query: query(task, cursor) } });
      if (!current(task) || (planRef && planRef !== reference)) return false;
      renderPreview(rows, page.items, {onOverride:(id,name)=>{if(active?.mutation)return;overrides.set(id,name);edited();},onExclude:id=>{if(active?.mutation)return;selected.delete(id);overrides.delete(id);recipe.excludeItems([id]);syncSelection();edited();},onExcludeGroup:excludeGroup});
      renderPaging({ ...page, hasPrevious: cursor !== null }, next => perform(nextTask => planPage(nextTask, reference, next)));
      return true;
    } catch (error) {
      if (current(task)) transition('clearPlan');
      throw error;
    }
  }
  function excludeGroup(groupId) {
    if(active || !planRef)return;
    const reference=planRef;
    perform(async task=>{
      const ids=[];let cursor=null;
      do {
        const page=await call('rename_page',{request:{planRef:reference,query:{...query(task,cursor),sort:'name',filter:'',status:null}}});
        if(!current(task)||planRef!==reference)return;
        for(const row of page.items)if(row.association?.groupId===groupId)ids.push(row.itemId);
        cursor=page.nextCursor;
      } while(cursor);
      ids.forEach(id=>{selected.delete(id);overrides.delete(id);});recipe.excludeItems(ids);syncSelection();edited();
      say(`Excluded ${ids.length} dependency-group rows across all pages. Preview again.`);
    });
  }
  async function acceptPlan(task, result) {
    if (!current(task)) return;
    if (!await planPage(task, result.planRef)) return;
    if (!current(task)) return;
    if(!transition('accept',{task,result}))return;reviewSummary.replaceChildren();appendSummary(reviewSummary,summary);
    say(`${counts.changed} changed · ${counts.unchanged} unchanged · ${counts.blocked} blocked · ${counts.needsReview} need review. Review every selected item before applying.`);
  }
  function choose(kind) {
    return perform(async task => {
      const selection = await call('select_sources', { request: { kind, revision: task.revision } });
      if (!current(task) || !selection.sources.length) return;
      inventoryReady = false; selected.clear(); inventoryItems.clear(); overrides.clear(); syncSelection(); rows.replaceChildren();
      sourceList.replaceChildren(
        el('p', '', `${selection.sourceCount} native sources selected${BigInt(selection.sourceCount) > 200n ? ' · showing the first 200' : ''}`),
        ...selection.sources.map(source => el('p', 'mono', source.display)),
      );
      say('Scanning selected sources…');
      await run(task, 'start_inventory', { revision: task.revision });
      if (!current(task)) return;
      inventoryReady = true;
      await inventoryPage(task);
      if (current(task)) say('Select items, choose a naming rule, then preview.');
    }, { advance: true });
  }
  host.querySelector('[data-action=folders]').addEventListener('click', () => choose('folders'));
  host.querySelector('[data-action=files]').addEventListener('click', () => choose('files'));
  preview.addEventListener('click', () => {
    if (active || !inventoryReady || !selected.size) return;
    perform(async task => {
      const result = await run(task, 'start_rename_preview', { revision: task.revision, selectedIds: [...selected], recipeJson: JSON.stringify(boundRecipe()), rebindSelection: true, mapping, rowOverrides:[...overrides].map(([entryId,newName])=>({entryId,newName})) });
      await acceptPlan(task, result);
    }, { advance: true });
  });
  apply.addEventListener('click', () => {
    if (active || !planRef) return;
    const reference = planRef;
    let restoreFocus = false;
    perform(async task => {
      const confirmation = await call('confirm_rename', { request: { planRef: reference } });
      if (!current(task) || planRef !== reference) return;
      if (!await reviewConfirmation({ confirmation, signal: task.abort.signal, title:confirmation.summary.isUndo?'Review filesystem Undo':'Review rename', confirmText:confirmation.summary.isUndo?'Apply reviewed Undo':'Apply reviewed rename' })) { restoreFocus = true; return; }
      if (!current(task) || planRef !== reference) return;
      task.mutation = true; transition('clearPlan'); inventoryReady = false; selected.clear(); controls();
      const result = await run(task, 'apply_rename', { planRef: reference, nonce: confirmation.nonce });
      if (!current(task)) return;
      say(`${formatOperationState(result.state)} · ${result.completedGroups} completed groups · ${result.completedSteps} recorded steps.${result.failure ? ` ${result.failure.message}` : ''} Review history for retained work and Undo.`);
      history.load();
    }).finally(() => { if (restoreFocus && !active && planRef === reference) apply.focus(); });
  });
  cancel.addEventListener('click', async () => {
    const task = active;
    if (!task) return;
    task.cancelRequested = true;
    if (!task.mutation) { edited(); say('Job cancelled. Preview again.'); return; }
    try {
      if (task.jobId) await call('cancel_job', { request: { jobId: task.jobId } });
      if (current(task)) say('Cancellation requested. Completed work is retained in history.');
    } catch (error) { if (current(task)) failed(error); }
  });
  host.querySelector('[data-action=sample]').addEventListener('click', () => {
    transition('clearPlan');
    renderPreview(rows, [{ itemId: 'sample', oldName: 'Sample video.mp4', newName: 'My video.mp4', oldDisplay: 'Sample / Sample video.mp4', newDisplay: 'Sample / My video.mp4', sizeBytes: '9007199254740993', status: 'changed', carriedDescendants: '0', issues: [] }]);
    say('Browser sample · read only. Choose sources in the desktop app to rename.'); controls();
  });
  host.querySelector('[data-action=selection]').addEventListener('click', () => perform(task => inventoryPage(task), { advance: true }));
  host.querySelector('[data-action=history]').addEventListener('click', () => history.load());

  function renderRecovery(task, view, page, cursor = null) {
    if (!current(task) || recoveryView !== view) return;
    const fragment = document.createDocumentFragment();
    fragment.append(el('p', '', `${view.continuingRestore ? 'Continue durable Restore' : 'Review recovery'} · ${view.unresolvedGroups} unresolved groups · ${view.actionCount} actions`));
    for (const action of page.items.slice(0, 200)) {
      const row = el('p', '', `${action.beforeDisplay || action.stepId} → ${action.afterDisplay || '—'}: ${action.disposition}`);
      row.dataset.testid = 'recovery-action'; fragment.append(row); appendRecoveryMetadata(fragment, action.metadata);
    }
    const loadPage = next => {
      if (!current(task) || active || recoveryView !== view) return;
      perform(async nextTask => {
        const result = await call('recovery_page', { request: { recoveryRef: view.recoveryRef, query: {...query(nextTask,next),sort:'name',filter:'',status:null} } });
        renderRecovery(nextTask, view, result, next);
      });
    };
    if (cursor) {
      const first = el('button', 'button', 'First recovery page');
      first.addEventListener('click', () => loadPage(null)); fragment.append(first);
    }
    if (page.nextCursor) {
      const next = el('button', 'button', 'Next recovery page');
      next.addEventListener('click', () => loadPage(page.nextCursor)); fragment.append(next);
    }
    for (const choice of view.choices) {
      const button = el('button', 'button', choice === 'restore' ? 'Restore interrupted group' : 'Resume compiled operation');
      button.addEventListener('click', () => {
        if (!current(task) || active || recoveryView !== view) return;
        perform(async confirmationTask => {
          const accepted = await reviewConfirmation({
            confirmation: { summary: { changed: view.actionCount, unchanged: '0', blocked: view.unresolvedGroups } },
            signal: confirmationTask.abort.signal, title: 'Review recovery',
            confirmText: choice === 'restore' ? 'Confirm Restore' : 'Confirm Resume',
          });
          if (!current(confirmationTask) || recoveryView !== view) return;
          if (!accepted) { renderRecovery(confirmationTask, view, page, cursor); return; }
          confirmationTask.mutation = true; recoveryView = null; recoveryHost.replaceChildren(); controls();
          const result = await run(confirmationTask, 'start_recovery', { recoveryRef: view.recoveryRef, choice, nonce: view.nonce });
          if (current(confirmationTask)) { say(`${formatOperationState(result.state)} · ${result.completedSteps} recorded recovery steps.`); history.load(); }
        });
      });
      fragment.append(button);
    }
    recoveryHost.replaceChildren(fragment);
  }
  const history = createHistory({
    host: host.querySelector('.rename-history'), notify: failed,
    onUndo: operationId => perform(async task => {
      inventoryReady = false; selected.clear();
      const result = await run(task, 'start_undo', { operationId, revision: task.revision });
      await acceptPlan(task, result);
    }, { advance: true }),
    onRecovery: operationId => perform(async task => {
      const view = await call('recovery_preview', { request: { operationId } });
      if (!current(task)) return;
      recoveryView = view; renderRecovery(task, view, view.page);
    }, { advance: true }),
  });
  host.querySelector('[data-action=history]').disabled = !isDesktop;
  host.querySelector('[data-action=sample]').hidden = isDesktop;
  if (isDesktop) ready = call('rename_session', { request: {} }).then(result => {
    transition('session',result.revision); initialized = true; controls();
  }).catch(failed);
  syncSelection(); controls(); say(isDesktop ? 'Choose native sources to begin.' : 'Browser preview · read only. Desktop app required.');
  return {
    activate() {},
    deactivate() {
      if (initialized) edited(); else invalidate();
      history.destroy(); recoveryHost.replaceChildren(); controls();
    },
    destroy() { this.deactivate(); recipe.destroy(); },
  };
}
