import { call } from '../api.js';
import { el } from '../dom.js';
function stateLabel(value) {
  return value[0].toUpperCase() + value.slice(1).replace(/[A-Z]/g, letter => ` ${letter.toLowerCase()}`);
}
export function createHistory({ host, onUndo, onRecovery, notify }) {
  let generation = 0, revision = null;
  const toolbar = el('div', 'rename-toolbar'), body = el('div');
  const sort = el('select'); sort.setAttribute('aria-label', 'History sort');
  for (const [value, label] of [['newest', 'Newest first'], ['oldest', 'Oldest first'], ['state', 'Operation state'], ['operationId', 'Operation ID']]) {
    const option = el('option', '', label); option.value = value; sort.append(option);
  }
  const filter = el('input'); filter.type = 'search'; filter.setAttribute('aria-label', 'Filter history');
  const search = el('button', 'button', 'Filter history');
  toolbar.append(sort, filter, search);
  sort.addEventListener('change', () => load());
  search.addEventListener('click', () => load());
  filter.addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); load(); } });
  async function load(cursor = null) {
    const token = ++generation, displaySort = sort.value, displayFilter = filter.value;
    try {
      if (cursor === null || revision === null) {
        revision = (await call('history_revision', { request: {} })).revision;
        if (token !== generation) return;
      }
      let result;
      for (let attempt = 0; attempt < 2; attempt++) {
        try {
          result = await call('operation_history', { request: { query: { cursor, limit: 200, sort: displaySort, filter: displayFilter, expectedRevision: revision } } });
          break;
        } catch (error) {
          if (error.code !== 'staleState' || attempt !== 0 || token !== generation) throw error;
          // Keep history live: refresh facts/revision and reset the keyset view.
          revision = (await call('history_revision', { request: {} })).revision;
          if (token !== generation) return;
          cursor = null;
        }
      }
      if (token !== generation) return;
      revision = result.revision;
      const fragment = document.createDocumentFragment();
      if (result.legacyUnresolved) fragment.append(el('p', 'notice warning', 'Legacy interrupted operation remains unresolved. No per-item evidence is available.'));
      for (const id of result.pendingOperations.slice(0, 200)) {
        const button = el('button', 'button', `Review recovery ${id}`);
        button.disabled = result.legacyUnresolved;
        button.addEventListener('click', () => onRecovery(id)); fragment.append(button);
      }
      if (!result.items.length) fragment.append(el('p', '', 'No completed history yet.'));
      for (const operation of result.items.slice(0, 200)) {
        const item = el('div', 'rename-history-item'); item.dataset.testid = 'history-item';
        item.append(el('p', '', `${stateLabel(operation.state)} · ${operation.completedSteps} completed steps · ${operation.operationId}`));
        if (operation.undoAvailable) {
          const undo = el('button', 'button', 'Preview Undo');
          undo.disabled = result.legacyUnresolved || result.pendingOperations.length > 0;
          undo.addEventListener('click', () => onUndo(operation.operationId)); item.append(undo);
        }
        fragment.append(item);
      }
      if (result.nextCursor) {
        const next = el('button', 'button', 'Next history page');
        next.addEventListener('click', () => load(result.nextCursor)); fragment.append(next);
      }
      body.replaceChildren(fragment); host.replaceChildren(toolbar, body);
    } catch (error) { if (token === generation) notify(error); }
  }
  return { load, destroy() { generation++; revision = null; host.replaceChildren(); } };
}
