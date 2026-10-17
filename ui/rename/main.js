import { call, isDesktop } from '../api.js';
import { $, el } from '../dom.js';
import { formatExactBytes } from '../exact.js';
import { watchJob } from '../jobs.js';
import { createRecipeEditor } from '../recipe/editor.js';
import { renderPreview, reviewConfirmation } from './preview.js';
import { createHistory } from './history.js';

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
  const selected = new Set();
  const recipe = createRecipeEditor({ recipe: { rule: 'keep', fixedStem: '' }, onChange: edited });
  host.querySelector('.rename-editor').append(recipe.element);

  function say(message) { status.textContent = message; }
  function failed(error) {
    if (error.name !== 'AbortError') say(`${error.message || String(error)}${error.phase ? ` (${error.phase})` : ''}`);
  }
  function controls() {
    const pending = active !== null;
    recipe.element.disabled = !initialized || Boolean(active?.mutation);
    for (const action of ['folders', 'files']) host.querySelector(`[data-action=${action}]`).disabled = !isDesktop || !initialized || pending;
    host.querySelector('[data-action=selection]').disabled = !isDesktop || !inventoryReady || pending;
    preview.disabled = !isDesktop || !inventoryReady || !selected.size || pending;
    apply.disabled = !isDesktop || !planRef || !counts || counts.changed === '0' || counts.blocked !== '0' || counts.needsReview !== '0' || pending;
    cancel.disabled = !pending;
  }
  function current(task) {
    return task.generation === generation && task.sequence === sequence && !task.abort.signal.aborted;
  }
  function invalidate() {
    sequence++;
    if (active) {
      active.abort.abort();
      if (active.jobId) call('cancel_job', { request: { jobId: active.jobId } }).catch(() => {});
      active = null;
    }
  }
  function edited() {
    invalidate(); generation++; revision++;
    planRef = null; counts = null; recoveryView = null;
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
    const task = { sequence: ++sequence, generation, revision: revision.toString(), abort: new AbortController(), jobId: null, cancelRequested: false, mutation: false };
    active = task; controls();
    try {
      await ready;
      if (!current(task)) return;
      await work(task);
    } catch (error) {
      if (current(task)) failed(error);
    } finally {
      if (active === task) active = null;
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
    return { cursor, limit: 200, sort: 'name', filter: '', expectedRevision: task.revision };
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
    const page = await call('inventory_page', { request: { query: query(task, cursor) } });
    if (!current(task)) return;
    const table = el('table', 'rename-table'), body = el('tbody');
    for (const item of page.items.slice(0, 200)) {
      const tr = el('tr'), cell = el('td'), label = el('label'), check = el('input');
      check.type = 'checkbox'; check.checked = selected.has(item.itemId);
      check.setAttribute('aria-label', `Select ${item.displayName}`);
      check.addEventListener('change', () => {
        check.checked ? selected.add(item.itemId) : selected.delete(item.itemId);
        edited();
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
      renderPreview(rows, page.items);
      renderPaging({ ...page, hasPrevious: cursor !== null }, next => perform(nextTask => planPage(nextTask, reference, next)));
      return true;
    } catch (error) {
      if (current(task)) { planRef = null; counts = null; }
      throw error;
    }
  }
  async function acceptPlan(task, result) {
    if (!current(task)) return;
    if (!await planPage(task, result.planRef)) return;
    if (!current(task)) return;
    planRef = result.planRef; counts = result.counts;
    say(`${counts.changed} changed · ${counts.unchanged} unchanged · ${counts.blocked} blocked · ${counts.needsReview} need review. Review every selected item before applying.`);
  }
  function choose(kind) {
    return perform(async task => {
      const selection = await call('select_sources', { request: { kind, revision: task.revision } });
      if (!current(task) || !selection.sources.length) return;
      inventoryReady = false; selected.clear(); rows.replaceChildren();
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
      const result = await run(task, 'start_rename_preview', { revision: task.revision, selectedIds: [...selected], ...recipe.getRecipe() });
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
      if (!await reviewConfirmation({ confirmation, signal: task.abort.signal })) { restoreFocus = true; return; }
      if (!current(task) || planRef !== reference) return;
      task.mutation = true; planRef = null; counts = null; inventoryReady = false; selected.clear(); controls();
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
    try {
      if (task.jobId) await call('cancel_job', { request: { jobId: task.jobId } });
      if (current(task)) say('Cancellation requested. Completed work is retained in history.');
    } catch (error) { if (current(task)) failed(error); }
  });
  host.querySelector('[data-action=sample]').addEventListener('click', () => {
    planRef = null; counts = null;
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
      row.dataset.testid = 'recovery-action'; fragment.append(row);
    }
    const loadPage = next => {
      if (!current(task) || active || recoveryView !== view) return;
      perform(async nextTask => {
        const result = await call('recovery_page', { request: { recoveryRef: view.recoveryRef, query: query(nextTask, next) } });
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
    revision = BigInt(result.revision); initialized = true; controls();
  }).catch(failed);
  controls(); say(isDesktop ? 'Choose native sources to begin.' : 'Browser preview · read only. Desktop app required.');
  return {
    activate() {},
    deactivate() {
      if (initialized) edited(); else invalidate();
      history.destroy(); recoveryHost.replaceChildren(); controls();
    },
    destroy() { this.deactivate(); recipe.destroy(); },
  };
}
