import { $, el, bytes, exactBytes, displayPath } from './dom.js';
import { call } from './api.js';

export function createActions({ results, getId, isLocked, setBusy, notify }) {
  const dialog = $('#action-dialog');
  let pending = null;
  let returnFocus = null;

  async function review(action) {
    if (isLocked()) return;
    const files = results.selectedFiles();
    if (!files.length) return;
    const result = results.getResult();
    if (result.preview) { notify('File actions are unavailable in the sample preview.'); return; }
    let destination = null;
    returnFocus = document.activeElement;
    if (action === 'move') {
      setBusy(true);
      try { destination = await call('pick_folder'); }
      catch (error) { notify(String(error), true); return; }
      finally { setBusy(false); }
      if (!destination) return;
    }
    pending = { id: getId(), paths: files.map(file => file.path), action, destination };
    const total = files.reduce((sum, file) => sum + file.size, 0);
    $('#action-title').textContent = action === 'delete' ? 'Permanently delete these files?' : 'Move these files?';
    $('#action-description').textContent = `${files.length} selected file${files.length === 1 ? '' : 's'} · ${bytes(total)} (${exactBytes(total)}). ${action === 'delete' ? 'This cannot be undone. Files will not go to the Recycle Bin.' : 'Selected files will be moved to the destination folder. At least one copy will remain in each group.'}`;
    const warning = $('#action-warning');
    warning.hidden = result.mode !== 'size';
    warning.textContent = 'These are unverified size matches. Different videos can have the same byte length. Check the files or run a full-content MD5 scan before taking action.';
    $('#action-destination').hidden = !destination;
    $('#action-destination').textContent = destination ? `Destination: ${displayPath(destination)}` : '';
    $('#action-files').replaceChildren(...files.map(file => {
      const item = el('li', 'mono', displayPath(file.path));
      item.append(el('small', '', exactBytes(file.size)));
      return item;
    }));
    $('#action-confirm').textContent = action === 'delete' ? `Delete ${files.length} file${files.length === 1 ? '' : 's'} permanently` : `Move ${files.length} file${files.length === 1 ? '' : 's'}`;
    $('#action-confirm').className = `button ${action === 'delete' ? 'button-danger' : 'button-primary'}`;
    dialog.returnValue = '';
    dialog.showModal();
    $('#action-cancel').focus();
  }

  dialog.addEventListener('close', async () => {
    const operation = pending;
    pending = null;
    if (dialog.returnValue !== 'confirm' || !operation) { returnFocus?.focus(); return; }
    setBusy(true);
    notify(operation.action === 'move' ? 'Moving selected files… Keep the drives connected.' : 'Deleting selected files…');
    try {
      const outcome = await call('apply_actions', operation);
      results.applyCompleted(outcome.completed, outcome.issues);
      const verb = operation.action === 'move' ? 'Moved' : 'Permanently deleted';
      notify(`${verb} ${outcome.completed.length} file${outcome.completed.length === 1 ? '' : 's'}.${outcome.issues.length ? ` ${outcome.issues.length} could not be processed. See issues below; those files remain selected where possible.` : ' At least one file remains in each group.'}`, outcome.issues.length > 0);
    } catch (error) { notify(`The action could not be completed: ${String(error)}`, true); }
    finally { setBusy(false); returnFocus?.focus(); }
  });
  $('#move-selected').addEventListener('click', () => review('move'));
  $('#delete-selected').addEventListener('click', () => review('delete'));
}
