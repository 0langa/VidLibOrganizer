import { call, isDesktop } from '../api.js';
import { $, el } from '../dom.js';
import { formatExactBytes } from '../exact.js';
import { watchJob } from '../jobs.js';
import { createRecipeEditor } from '../recipe/editor.js';
import { renderPreview, reviewConfirmation } from './preview.js';
import { createHistory } from './history.js';
export function formatOperationState(value) { return value ? value[0].toUpperCase() + value.slice(1).replace(/[A-Z]/g, letter => ` ${letter.toLowerCase()}`) : 'Unknown'; }
export function createRename() {
  const host = $('#view-rename');
  let initialized = !isDesktop;
  let revision = 0n, generation = 0, ready = Promise.resolve(), planRef = null, counts = null, inventoryReady = false;
  let controller = new AbortController(), activeJob = null, selected = new Set(), cursor = null, pageQuery = { sort: 'name', filter: '' };
  const status = host.querySelector('[data-testid=rename-status]');
  const apply = host.querySelector('[data-testid=rename-apply]');
  const preview = host.querySelector('[data-testid=rename-preview]');
  const cancel = host.querySelector('[data-testid=rename-cancel]');
  const rows = host.querySelector('.rename-rows');
  const paging = host.querySelector('.rename-paging');
  const sourceList = host.querySelector('.rename-sources');
  const recipe = createRecipeEditor({ recipe: { rule: 'keep', fixedStem: '' }, onChange: () => edited() });
  host.querySelector('.rename-editor').append(recipe.element);
  function say(message) { status.textContent = message; }
  function failed(error) { if (error.name !== 'AbortError') say(`${error.message || String(error)}${error.phase ? ` (${error.phase})` : ''}`); }
  function controls() { host.querySelector('[data-action=selection]').disabled = !isDesktop || !inventoryReady || Boolean(activeJob); recipe.element.disabled = !initialized; for (const action of ['folders','files']) host.querySelector(`[data-action=${action}]`).disabled = !isDesktop || !initialized || Boolean(activeJob); apply.disabled = !isDesktop || !planRef || !counts || counts.changed === '0' || counts.blocked !== '0' || counts.needsReview !== '0' || Boolean(activeJob); preview.disabled = !isDesktop || !inventoryReady || !selected.size || Boolean(activeJob); cancel.disabled = !activeJob; }
  function edited() {
    generation++; revision++; planRef = null; counts = null;
    if (controller.signal.aborted) controller = new AbortController();
    const input = revision.toString();
    if (isDesktop && initialized) { ready = ready.catch(() => {}).then(() => call('update_rename_inputs', { request: { revision: input } })); ready.catch(failed); }
    say('Inputs changed. Preview again.'); controls();
  }
  async function run(command, request) {
    const token = generation, signal = controller.signal;
    await ready;
    if (token !== generation || signal.aborted) throw new DOMException('Superseded', 'AbortError');
    const id = await call(command, { request });
    if (token !== generation || signal.aborted) { await call('cancel_job', { request: { jobId: id } }); throw new DOMException('Superseded', 'AbortError'); }
    activeJob = id; controls();
    try { return await watchJob(id, { signal, onProgress: value => { if (token === generation) say(`${value.phase} · ${value.done}`); } }); }
    catch(error) { if(error.name === 'AbortError' && !signal.aborted && token === generation) say('Job cancelled. Completed work remains in history.'); throw error; }
    finally { if (activeJob === id) activeJob = null; controls(); }
  }
  function query(next) { return { ...pageQuery, cursor: next, limit: 200, expectedRevision: revision.toString() }; }
  async function inventoryPage(next = null) {
    const token = generation;
    const result = await call('inventory_page', { request: { query: query(next) } });
    if (token !== generation) return;
    const table = el('table', 'rename-table'), body = el('tbody');
    for (const item of result.items.slice(0, 200)) {
      const tr = el('tr'), selectCell = el('td'), label = el('label'), check = el('input'); check.type = 'checkbox'; check.checked = selected.has(item.itemId); check.setAttribute('aria-label', `Select ${item.displayName}`);
      check.addEventListener('change', () => { check.checked ? selected.add(item.itemId) : selected.delete(item.itemId); edited(); });
      label.append(check, document.createTextNode(item.displayName)); selectCell.append(label); tr.append(selectCell, el('td', '', formatExactBytes(item.sizeBytes))); body.append(tr);
    }
    table.append(body); rows.replaceChildren(table); renderPaging(result, inventoryPage);
  }
  function renderPaging(page, load) {
    paging.replaceChildren();
    if (cursor) { const first = el('button', 'button', 'First page'); first.addEventListener('click', () => { cursor = null; load().catch(failed); }); paging.append(first); }
    if (page.nextCursor) { const next = el('button', 'button', 'Next page'); next.addEventListener('click', () => { cursor = page.nextCursor; load(cursor).catch(failed); }); paging.append(next); }
  }
  async function planPage(next = null) {
    const token = generation, reference = planRef;
    const result = await call('rename_page', { request: { planRef: reference, query: query(next) } });
    if (token !== generation || planRef !== reference) return;
    renderPreview(rows, result.items); renderPaging(result, planPage);
  }
  async function acceptPlan(result, token) {
    if (token !== generation || controller.signal.aborted) return;
    planRef = result.planRef; counts = result.counts;
    await planPage();
    if (token !== generation || !planRef || controller.signal.aborted) return;
    say(`${counts.changed} changed · ${counts.unchanged} unchanged · ${counts.blocked} blocked · ${counts.needsReview} need review. Review every selected item before applying.`); controls();
  }
  async function choose(kind) {
    try {
      edited(); const token = generation; await ready;
      const selection = await call('select_sources', { request: { kind, revision: revision.toString() } });
      if (token !== generation || !selection.sources.length) return;
      inventoryReady = false; selected.clear(); rows.replaceChildren();
      sourceList.replaceChildren(el('p', '', `${selection.sourceCount} native sources selected${BigInt(selection.sourceCount) > 200n ? ' · showing the first 200' : ''}`), ...selection.sources.map(source => el('p', 'mono', source.display)));
      say('Scanning selected sources…');
      await run('start_inventory', { revision: revision.toString() });
      if (token !== generation) return;
      inventoryReady = true; await inventoryPage(); say('Select items, choose a naming rule, then preview.'); controls();
    } catch (error) { failed(error); }
  }
  host.querySelector('[data-action=folders]').addEventListener('click', () => choose('folders'));
  host.querySelector('[data-action=files]').addEventListener('click', () => choose('files'));
  preview.addEventListener('click', async () => {
    try { planRef = null; counts = null; controls(); const token = generation; const result = await run('start_rename_preview', { revision: revision.toString(), selectedIds: [...selected], ...recipe.getRecipe() }); await acceptPlan(result, token); }
    catch (error) { failed(error); }
  });
  apply.addEventListener('click', async () => {
    const token = generation, reference = planRef;
    try {
      const confirmation = await call('confirm_rename', { request: { planRef: reference } });
      if (token !== generation || !planRef) return;
      if (!await reviewConfirmation({ confirmation, signal: controller.signal })) return;
      if (token !== generation || planRef !== reference) return;
      planRef = null; inventoryReady = false; selected.clear(); controls();
      const result = await run('apply_rename', { planRef: reference, nonce: confirmation.nonce });
      say(`${formatOperationState(result.state)} · ${result.completedGroups} completed groups · ${result.completedSteps} recorded steps.${result.failure ? ` ${result.failure.message}` : ''} Review history for retained work and Undo.`);
      history.load();
    } catch (error) { failed(error); }
  });
  cancel.addEventListener('click', async () => { if (activeJob) { try { await call('cancel_job', { request: { jobId: activeJob } }); say('Cancellation requested. Completed work is retained in history.'); } catch (error) { failed(error); } } });
  host.querySelector('[data-action=sample]').addEventListener('click', () => { planRef = null; counts = null; renderPreview(rows, [{ itemId: 'sample', oldName: 'Sample video.mp4', newName: 'My video.mp4', oldDisplay: 'Sample / Sample video.mp4', newDisplay: 'Sample / My video.mp4', sizeBytes: '9007199254740993', status: 'changed', carriedDescendants: '0', issues: [] }]); say('Browser sample · read only. Choose sources in the desktop app to rename.'); controls(); });
  host.querySelector('[data-action=selection]').addEventListener('click', async () => { edited(); try { await ready; cursor = null; await inventoryPage(); } catch(error) { failed(error); } });
  host.querySelector('[data-action=history]').addEventListener('click', () => history.load());
  const history = createHistory({ host: host.querySelector('.rename-history'), notify: failed, onUndo: async operationId => {
    try { edited(); inventoryReady = false; selected.clear(); controls(); const token = generation; const result = await run('start_undo', { operationId, revision: revision.toString() }); await acceptPlan(result, token); }
    catch (error) { failed(error); }
  }, onRecovery: async operationId => {
    try {
      const token = generation;
      const review = await call('recovery_preview', { request: { operationId } });
      if (token !== generation) return;
      const target = host.querySelector('.rename-recovery'); target.replaceChildren(el('p', '', `${review.continuingRestore ? 'Continue durable Restore' : 'Review recovery'} · ${review.unresolvedGroups} unresolved groups · ${review.actionCount} actions`));
      for (const action of review.actions) target.append(el('p', '', `${action.beforeDisplay || action.stepId} → ${action.afterDisplay || "—"}: ${action.disposition}`));
      for (const choice of review.choices) {
        const button = el('button', 'button', choice === 'restore' ? 'Restore interrupted group' : 'Resume compiled operation');
        button.addEventListener('click', async () => {
          try {
            if (!await reviewConfirmation({ confirmation: { summary: { changed: review.actionCount, unchanged: '0', blocked: review.unresolvedGroups } }, signal: controller.signal, title: 'Review recovery', confirmText: choice === 'restore' ? 'Confirm Restore' : 'Confirm Resume' })) return;
            if (token !== generation) return;
            target.replaceChildren(); const result = await run('start_recovery', { recoveryRef: review.recoveryRef, choice, nonce: review.nonce }); say(`${formatOperationState(result.state)} · ${result.completedSteps} recorded recovery steps.`); history.load();
          } catch (error) { failed(error); }
        }); target.append(button);
      }
    } catch (error) { failed(error); }
  } });
  host.querySelector('[data-action=folders]').disabled = !isDesktop; host.querySelector('[data-action=files]').disabled = !isDesktop;
  host.querySelector('[data-action=history]').disabled = !isDesktop; host.querySelector('[data-action=selection]').disabled = !isDesktop;
  host.querySelector('[data-action=sample]').hidden = isDesktop;
  if (isDesktop) ready = call('rename_session', { request: {} }).then(result => { revision = BigInt(result.revision); initialized = true; controls(); }).catch(failed);
  controls(); say(isDesktop ? 'Choose native sources to begin.' : 'Browser preview · read only. Desktop app required.');
  return { activate() { if (controller.signal.aborted) controller = new AbortController(); }, deactivate() { controller.abort(); if (activeJob) call('cancel_job', { request: { jobId: activeJob } }).catch(() => {}); activeJob = null; if(initialized) edited(); history.destroy(); host.querySelector('.rename-recovery').replaceChildren(); controls(); }, destroy() { this.deactivate(); recipe.destroy(); } };
}
