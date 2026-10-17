import { call } from '../api.js';
import { el } from '../dom.js';
function stateLabel(value) { return value[0].toUpperCase() + value.slice(1).replace(/[A-Z]/g, letter => ` ${letter.toLowerCase()}`); }
export function createHistory({ host, onUndo, onRecovery, notify }) {
  let generation = 0;
  async function load(cursor = null) {
    const token = ++generation;
    try {
      const result = await call('operation_history', { request: { cursor } });
      if (token !== generation) return;
      const fragment = document.createDocumentFragment();
      if (result.legacyUnresolved) fragment.append(el('p', 'notice warning', 'Legacy interrupted operation remains unresolved. No per-item evidence is available.'));
      for (const id of result.pendingOperations.slice(0, 200)) { const button = el('button', 'button', `Review recovery ${id}`); button.disabled = result.legacyUnresolved; button.addEventListener('click', () => onRecovery(id)); fragment.append(button); }
      if (!result.items.length) fragment.append(el('p', '', 'No completed history yet.'));
      for (const operation of result.items.slice(0, 200)) {
        const item = el('div', 'rename-history-item');
        item.append(el('p', '', `${stateLabel(operation.state)} · ${operation.completedSteps} completed steps · ${operation.operationId}`));
        if (operation.undoAvailable) { const undo = el('button', 'button', 'Preview Undo'); undo.disabled = result.legacyUnresolved || result.pendingOperations.length > 0; undo.addEventListener('click', () => onUndo(operation.operationId)); item.append(undo); }
        fragment.append(item);
      }
      if (result.nextCursor) { const next = el('button', 'button', 'Next history page'); next.addEventListener('click', () => load(result.nextCursor)); fragment.append(next); }
      host.replaceChildren(fragment);
    } catch (error) { if (token === generation) notify(error); }
  }
  return { load, destroy() { generation++; host.replaceChildren(); } };
}
