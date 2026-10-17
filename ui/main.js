import { $, el, hydrateIcons, bytes, count, displayPath } from './dom.js';
import { call, isDesktop, sampleResult } from './api.js';
import { renderLibraries } from './libraries.js';
import { createResults } from './results.js';
import { createRename } from './rename/main.js';
import { createActions } from './actions.js';

hydrateIcons();
const rename = createRename();
let currentView = "finder";
const state = { paths: [], included: new Set(), ready: !isDesktop, scanning: false, busy: false, acting: false, cancelling: false, id: null, loadedResultId: null, preview: false, pollTimer: null };
const locked = () => !state.ready || state.scanning || state.busy || state.acting;

function notify(message, error = false) {
  $('#message-text').textContent = message;
  $('#message').className = `notice${error ? ' error' : ''}`;
  $('#message').setAttribute('role', error ? 'alert' : 'status');
  $('#message').hidden = false;
}

function selectionChanged(files) {
  $('#selection-bar').hidden = !files.length;
  $('#selection-count').textContent = `${count(files.length)} file${files.length === 1 ? '' : 's'} selected`;
  $('#selection-bytes').textContent = `${bytes(files.reduce((total, file) => total + file.size, 0))} across your result groups`;
  $('#move-selected').disabled = locked() || !isDesktop;
  $('#delete-selected').disabled = locked() || !isDesktop;
  $('#clear-selection').disabled = locked();
}

const results = createResults({ onSelection: selectionChanged, onReveal: async path => {
  try { await call('reveal_file', { path }); } catch (error) { notify(`Could not open the folder: ${String(error)}`, true); }
} });

function navigate(view) {
  if (!['finder', 'libraries', 'about', 'rename'].includes(view)) view = 'finder';
  if (currentView === 'rename' && view !== 'rename') rename.deactivate();
  if (view === 'rename') rename.activate();
  currentView = view;
  document.querySelectorAll('.page').forEach(page => { page.hidden = page.id !== `view-${view}`; });
  document.querySelectorAll('[data-view]').forEach(button => {
    button.classList.toggle('active', button.dataset.view === view);
    button.dataset.view === view ? button.setAttribute('aria-current', 'page') : button.removeAttribute('aria-current');
  });
  $('#breadcrumb').textContent = { finder: 'Duplicate finder', libraries: 'Library folders', about: 'About this app', rename: 'Rename' }[view];
}

function updateControls() {
  const disabled = locked() || !isDesktop;
  for (const selector of ['#folder-path', '#add-folder', '#browse-folder']) $(selector).disabled = disabled;
  document.querySelectorAll('input[name=mode]').forEach(input => { input.disabled = disabled; });
  $('#start-scan').disabled = disabled || !state.included.size;
  $('#cancel-scan').disabled = state.cancelling;
  $('#cancel-scan').textContent = state.cancelling ? 'Cancelling…' : 'Cancel scan';
  $('#export-report').hidden = !results.getResult() || state.preview;
  $('#export-report').disabled = disabled;
  $('#connection-status').replaceChildren(el('span', 'status-dot'), document.createTextNode(!isDesktop ? 'Browser preview · desktop app required' : state.scanning ? 'Scan running · your files stay local' : state.acting ? 'File action in progress…' : state.busy ? 'Working…' : state.ready ? 'Ready · local workspace' : 'Connecting to desktop…'));
  renderLibraries({ paths: state.paths, included: state.included, locked: disabled,
    onToggle: (path, included) => {
      included ? state.included.add(path) : state.included.delete(path);
      $('#start-scan').disabled = locked() || !isDesktop || !state.included.size;
    },
    onRemove: path => savePaths(state.paths.filter(saved => saved !== path)), onManage: () => navigate('libraries'),
  });
  selectionChanged(results.selectedFiles());
}

function setBusy(value) {
  state.busy = value;
  results.setBusy(locked());
  updateControls();
}
createActions({ results, getId: () => state.id, isLocked: locked, setBusy, notify });

async function savePaths(paths) {
  if (locked() || !isDesktop) return;
  setBusy(true);
  try {
    await call('save_libraries', { paths });
    const previous = new Set(state.paths);
    state.paths = paths;
    state.included = new Set(paths.filter(path => state.included.has(path) || !previous.has(path)));
    $('#folder-path').value = '';
  } catch (error) { notify(`Could not save your folders: ${String(error)}`, true); }
  finally { setBusy(false); }
}

function addPath(path) {
  const value = path.trim().replace(/^"(.*)"$/, '$1');
  if (!value) { $('#folder-path').focus(); return; }
  const normalize = item => item.replace(/[\\/]+$/, '').replace(/\//g, '\\').toLocaleLowerCase();
  if (state.paths.some(saved => normalize(saved) === normalize(value))) { notify('This folder is already in your library.'); return; }
  return savePaths([...state.paths, value]);
}

$('#add-folder-form').addEventListener('submit', event => { event.preventDefault(); addPath($('#folder-path').value); });
$('#browse-folder').addEventListener('click', async () => {
  if (locked()) return;
  setBusy(true);
  let path;
  try { path = await call('pick_folder'); } catch (error) { notify(`Could not open the folder picker: ${String(error)}`, true); }
  finally { setBusy(false); }
  if (path) await addPath(path);
});
document.querySelectorAll('[data-view]').forEach(button => button.addEventListener('click', () => navigate(button.dataset.view)));
document.querySelectorAll('[data-go]').forEach(button => button.addEventListener('click', () => navigate(button.dataset.go)));
$('.brand').addEventListener('click', event => { event.preventDefault(); navigate('finder'); });
$('#dismiss-message').addEventListener('click', () => { $('#message').hidden = true; });
$('#clear-selection').addEventListener('click', () => results.clearSelection());

let lastAnnouncement = 0;
function renderProgress(progress) {
  const hashing = String(progress.phase).toLowerCase().includes('hash');
  $('#progress-phase').textContent = hashing ? 'COMPARING COMPLETE FILES' : 'SCAN IN PROGRESS';
  $('#progress-title').textContent = hashing ? 'Checking full-content MD5…' : 'Discovering your videos…';
  $('#progress-main').textContent = hashing ? `${count(progress.files_hashed)} of ${count(progress.files_to_hash)} files hashed` : `${count(progress.files_found)} videos found`;
  $('#progress-secondary').textContent = hashing ? `${bytes(progress.bytes_hashed)} / ${bytes(progress.bytes_to_hash)}` : `${bytes(progress.bytes_found)} discovered · checking subfolders`;
  const bar = $('#progress-bar');
  if (hashing && progress.bytes_to_hash > 0) { bar.max = progress.bytes_to_hash; bar.value = progress.bytes_hashed; }
  else bar.removeAttribute('value');
  $('#progress-path').textContent = displayPath(progress.current_path) || 'Preparing the scan…';
  $('#progress-path').title = displayPath(progress.current_path);
  if (Date.now() - lastAnnouncement > 5000) {
    $('#progress-live').textContent = `${$('#progress-title').textContent} ${$('#progress-main').textContent}`;
    lastAnnouncement = Date.now();
  }
}

async function acceptStatus(status) {
  const wasActing = state.acting;
  state.id = status.id;
  state.scanning = status.running;
  state.acting = Boolean(status.acting);
  if (state.acting && !wasActing) notify('A file action is in progress. Keep the drives connected; results will refresh when it finishes.');
  if (wasActing && !state.acting) state.loadedResultId = null;
  $('#scan-progress').hidden = !status.running;
  if (status.progress) renderProgress(status.progress);
  if (!status.running) state.cancelling = false;
  if (!state.acting && status.result_ready && status.id && state.loadedResultId !== status.id) {
    const result = await call('scan_result', { id: status.id });
    results.setResult(result);
    state.loadedResultId = status.id;
    state.preview = false;
    notify(wasActing ? 'File action finished. Results have been refreshed.' : result.cancelled ? 'Scan cancelled. Partial results are ready to review.' : `Scan complete. ${count(result.groups.length)} ${result.mode === 'md5' ? 'MD5 match' : 'size-match candidate'} group${result.groups.length === 1 ? '' : 's'} found.${result.issues.length ? ` ${count(result.issues.length)} issues need attention.` : ''}`);
  }
  if (status.error) notify(status.error, true);
  results.setBusy(locked());
  updateControls();
}

function schedulePoll(delay = 300) {
  clearTimeout(state.pollTimer);
  state.pollTimer = setTimeout(pollStatus, delay);
}
async function pollStatus() {
  try {
    await acceptStatus(await call('scan_status'));
    if (state.scanning || state.acting) schedulePoll();
  } catch (error) {
    notify(`Could not refresh scan status. Retrying… ${String(error)}`, true);
    schedulePoll(1500);
  }
}

$('#start-scan').addEventListener('click', async () => {
  if (locked() || !state.included.size) return;
  setBusy(true);
  try {
    state.id = await call('start_scan', { roots: [...state.included], mode: $('input[name=mode]:checked').value });
    state.loadedResultId = null;
    state.scanning = true;
    results.setResult(null);
    $('#message').hidden = true;
    $('#scan-progress').hidden = false;
    renderProgress({ phase: 'discovering', files_found: 0, bytes_found: 0 });
    schedulePoll(0);
  } catch (error) { notify(`Could not start the scan: ${String(error)}`, true); }
  finally { setBusy(false); }
});
$('#cancel-scan').addEventListener('click', async () => {
  if (!state.scanning || state.cancelling) return;
  state.cancelling = true;
  updateControls();
  try { await call('cancel_scan', { id: state.id }); }
  catch (error) { state.cancelling = false; notify(`Could not cancel the scan: ${String(error)}`, true); updateControls(); }
});
$('#export-report').addEventListener('click', async () => {
  if (locked()) return;
  setBusy(true);
  try { const path = await call('export_report', { id: state.id }); if (path) notify(`Report saved to ${displayPath(path)}`); }
  catch (error) { notify(`Could not export the report: ${String(error)}`, true); }
  finally { setBusy(false); }
});
$('#preview-toggle').addEventListener('click', () => {
  state.preview = !state.preview;
  results.setResult(state.preview ? { ...sampleResult(), preview: true } : null);
  $('#preview-toggle').textContent = state.preview ? 'Close sample preview' : 'Preview sample results';
  updateControls();
  navigate('finder');
});

async function initialize() {
  updateControls();
  if (!isDesktop) { $('#desktop-notice').hidden = false; return; }
  try {
    const paths = await call('load_libraries');
    state.paths = paths;
    state.included = new Set(paths);
    const status = await call('scan_status');
    state.ready = true;
    await acceptStatus(status);
    if (state.scanning || state.acting) schedulePoll();
  } catch (error) {
    state.ready = false;
    notify(`Could not connect to the desktop app. Retrying… ${String(error)}`, true);
    updateControls();
    setTimeout(initialize, 2000);
  }
}
initialize();
