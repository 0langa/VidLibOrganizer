import { el } from '../dom.js';
import { formatExactBytes } from '../exact.js';
export function renderPreview(host, rows) {
  const table = el('table', 'rename-table');
  const head = el('thead'), header = el('tr');
  for (const title of ['Current name', 'Proposed name', 'Exact size', 'Status / issues']) header.append(el('th', '', title));
  head.append(header); const body = el('tbody');
  const fragment = document.createDocumentFragment();
  for (const row of rows.slice(0, 200)) {
    const tr = el('tr');
    const oldName = el('td', '', row.oldName); oldName.title = row.oldDisplay;
    const newName = el('td', '', row.newName); newName.title = row.newDisplay;
    const size = el('td', 'mono', formatExactBytes(row.sizeBytes)); size.dataset.testid = 'exact-bytes';
    const status = el('td', '', `${row.status}${row.carriedDescendants !== '0' ? ` · carries ${row.carriedDescendants} descendants` : ''}${row.issues.length ? ` · ${row.issues.join('; ')}` : ''}`);
    tr.append(oldName, newName, size, status); fragment.append(tr);
  }
  body.append(fragment); table.append(head, body); host.replaceChildren(table);
}
export async function reviewConfirmation({ confirmation, signal, title = 'Review rename', confirmText = 'Apply reviewed rename' }) {
  const dialog = el('dialog', 'rename-dialog'); dialog.setAttribute('aria-labelledby', 'rename-confirm-title');
  const heading = el('h2', '', title); heading.id = 'rename-confirm-title';
  const summary = el('p', '', `${confirmation.summary.changed} changed · ${confirmation.summary.unchanged} unchanged · ${confirmation.summary.blocked} blocked. The batch is recoverable; earlier completed groups can remain after cancellation or failure.`);
  const cancel = el('button', 'button', 'Cancel'); const confirm = el('button', 'button button-primary', confirmText);
  dialog.append(heading, summary, cancel, confirm); document.body.append(dialog);
  const previous = document.activeElement;
  return new Promise(resolve => {
    function close(accepted) { dialog.close(); dialog.remove(); signal?.removeEventListener('abort', abort); previous?.focus(); resolve(accepted); }
    const abort = () => close(false);
    cancel.addEventListener('click', () => close(false)); confirm.addEventListener('click', () => close(true));
    dialog.addEventListener('cancel', event => { event.preventDefault(); close(false); });
    signal?.addEventListener('abort', abort, { once: true });
    dialog.showModal(); cancel.focus();
  });
}
