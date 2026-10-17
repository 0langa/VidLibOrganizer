import { $, el, icon, button, bytes, count, exactBytes, displayPath } from './dom.js';

const GROUP_PAGE_SIZE = 10;
const FILE_PAGE_SIZE = 20;

export function createResults({ onSelection, onReveal }) {
  let result = null;
  let page = 0;
  let busy = false;
  const selected = new Set();
  const kept = new Set();
  const filePages = new Map();
  let groupIndex = new Map();
  const search = $('#result-search');
  const sort = $('#result-sort');
  const files = () => (result?.groups || []).flatMap(group => group.files);
  const selectedFiles = () => files().filter(file => selected.has(file.path));

  function refreshSelection() {
    const groupSelections = new Map();
    document.querySelectorAll('input[data-group-id]').forEach(input => {
      const group = groupIndex.get(input.dataset.groupId);
      if (!group) return;
      if (!groupSelections.has(group.id)) groupSelections.set(group.id, group.files.filter(file => selected.has(file.path)).length);
      const atLimit = groupSelections.get(group.id) >= group.files.length - 1;
      input.checked = selected.has(input.dataset.filePath);
      input.disabled = busy || kept.has(group.id) || (!input.checked && atLimit);
      input.title = !input.checked && atLimit ? 'Keep at least one file in this group.' : '';
    });
    onSelection(selectedFiles());
  }

  function renderStats() {
    const groups = result.groups;
    const stats = [
      [result.mode === 'md5' ? 'MD5 match groups' : 'Size-match groups', count(groups.length), `${count(groups.reduce((sum, group) => sum + group.files.length, 0))} files in these groups`],
      ['Videos scanned', count(result.files_scanned), `${bytes(result.bytes_scanned)} examined · ${(result.elapsed_ms / 1000).toFixed(1)} seconds`],
      ['Potential redundant bytes', bytes(groups.reduce((sum, group) => sum + group.redundant_bytes, 0)), 'Logical estimate · keep one file per group'],
    ];
    $('#result-stats').replaceChildren(...stats.map(([label, value, detail]) => {
      const stat = el('div', 'stat');
      stat.append(el('span', 'stat-label', label), el('strong', 'stat-value', value), el('span', 'stat-detail', detail));
      return stat;
    }));
    const banner = $('#results-banner');
    banner.hidden = !result.cancelled && !result.preview;
    banner.className = 'notice warning';
    banner.textContent = result.preview ? 'Sample preview — illustrative data only. No drives have been scanned, and file actions are unavailable.' : 'Scan cancelled. These partial results include only the files examined before cancellation.';
  }

  function renderIssues() {
    const target = $('#scan-issues');
    target.replaceChildren();
    if (!result?.issues?.length) return;
    const details = el('details', 'issues');
    details.append(el('summary', '', `${count(result.issues.length)} issues need your attention`));
    const list = el('ul', 'issue-list');
    result.issues.forEach(issue => {
      const row = el('li');
      row.append(el('strong', 'mono', displayPath(issue.path)), el('span', '', issue.message));
      list.append(row);
    });
    details.append(list);
    target.append(details);
  }

  function renderFile(file, group) {
    const row = el('li', 'file-row');
    const checkbox = el('input');
    checkbox.type = 'checkbox';
    checkbox.dataset.groupId = group.id;
    checkbox.dataset.filePath = file.path;
    checkbox.setAttribute('aria-label', `Select ${displayPath(file.path)}`);
    checkbox.addEventListener('change', () => {
      checkbox.checked ? selected.add(file.path) : selected.delete(file.path);
      refreshSelection();
    });
    const description = el('div', 'file-description');
    description.append(el('span', 'file-name', file.name), el('span', 'file-path', displayPath(file.path)), el('span', 'file-size', exactBytes(file.size)));
    const reveal = button('', 'icon-button open-folder', () => onReveal(file.path), 'folder');
    reveal.setAttribute('aria-label', `Open containing folder for ${displayPath(file.path)}`);
    reveal.title = 'Open containing folder';
    reveal.disabled = busy || result.preview;
    row.append(checkbox, icon('film'), description, reveal);
    return row;
  }

  function renderGroup(group, index) {
    const card = el('article', `group-card${kept.has(group.id) ? ' kept' : ''}`);
    const header = el('div', 'group-header');
    const heading = el('div', 'group-header-text');
    const title = el('h3', 'group-title', group.files[0]?.name || 'Matching files');
    const subtitle = el('div', 'group-subtitle');
    const match = el('span', `match-label${result.mode === 'md5' ? ' verified' : ''}`, result.mode === 'md5' ? 'MD5 match' : 'Size match · unverified');
    if (result.mode === 'md5') match.prepend(icon('check'));
    subtitle.append(match, el('span', '', `${count(group.files.length)} copies`), el('span', '', `${bytes(group.size)} each`));
    heading.append(title, subtitle);
    const space = el('div', 'group-space', bytes(group.redundant_bytes));
    space.append(el('small', '', 'potentially redundant'));
    header.append(el('span', 'group-number', String(index + 1).padStart(2, '0')), heading, space);
    const list = el('ul', 'group-files');
    const filePage = Math.min(filePages.get(group.id) || 0, Math.ceil(group.files.length / FILE_PAGE_SIZE) - 1);
    group.files.slice(filePage * FILE_PAGE_SIZE, (filePage + 1) * FILE_PAGE_SIZE).forEach(file => list.append(renderFile(file, group)));
    const footer = el('div', 'group-footer');
    footer.append(el('span', group.hash ? 'hash-text' : '', group.hash ? `MD5 ${group.hash}` : `${exactBytes(group.size)} per file · keep at least one`));
    const keepButton = button(kept.has(group.id) ? 'Undo keep all' : 'Keep all', 'text-button', () => {
      if (kept.has(group.id)) kept.delete(group.id);
      else { kept.add(group.id); group.files.forEach(file => selected.delete(file.path)); }
      render();
      document.querySelectorAll('[data-keep-group]').forEach(button => { if (button.dataset.keepGroup === String(group.id)) button.focus(); });
    }, kept.has(group.id) ? 'check' : undefined);
    keepButton.dataset.keepGroup = group.id;
    keepButton.disabled = busy;
    footer.append(keepButton);
    card.append(header, list);
    if (group.files.length > FILE_PAGE_SIZE) {
      const filePagination = el('div', 'group-footer');
      filePagination.append(el('span', '', `Files ${filePage * FILE_PAGE_SIZE + 1}–${Math.min((filePage + 1) * FILE_PAGE_SIZE, group.files.length)} of ${count(group.files.length)}`));
      const controls = el('div', 'pagination-controls');
      const previous = button('Previous files', 'text-button', () => { filePages.set(group.id, filePage - 1); render(); });
      const next = button('Next files', 'text-button', () => { filePages.set(group.id, filePage + 1); render(); });
      previous.disabled = filePage === 0 || busy;
      next.disabled = (filePage + 1) * FILE_PAGE_SIZE >= group.files.length || busy;
      controls.append(previous, next);
      filePagination.append(controls);
      card.append(filePagination);
    }
    card.append(footer);
    return card;
  }

  function render() {
    $('#results-empty').hidden = Boolean(result);
    $('#results-content').hidden = !result;
    if (!result) { refreshSelection(); return; }
    renderStats();
    const query = search.value.trim().toLocaleLowerCase();
    const groups = result.groups.filter(group => group.files.some(file => `${file.name} ${displayPath(file.path)}`.toLocaleLowerCase().includes(query)));
    const orders = { space: (a, b) => b.redundant_bytes - a.redundant_bytes, size: (a, b) => b.size - a.size, count: (a, b) => b.files.length - a.files.length, name: (a, b) => a.files[0].name.localeCompare(b.files[0].name) };
    groups.sort(orders[sort.value]);
    page = Math.min(page, Math.max(0, Math.ceil(groups.length / GROUP_PAGE_SIZE) - 1));
    $('#result-context').textContent = `${count(groups.length)} ${result.mode === 'md5' ? 'MD5 match' : 'size-match candidate'} group${groups.length === 1 ? '' : 's'}${query ? groups.length === 1 ? ' matches your search' : ' match your search' : ''}. ${result.mode === 'size' ? 'Same size does not guarantee the same content.' : 'Complete file contents were hashed.'}`;
    $('#result-groups').replaceChildren(...groups.slice(page * GROUP_PAGE_SIZE, (page + 1) * GROUP_PAGE_SIZE).map((group, index) => renderGroup(group, page * GROUP_PAGE_SIZE + index)));
    if (!groups.length) {
      const empty = el('div', 'empty-state');
      empty.append(el('h3', '', query ? 'No matches for this search.' : 'No duplicate groups to review.'), el('p', '', query ? 'Try a shorter filename or a different part of the path.' : result.issues.length ? 'Check the reported issues for files or folders that could not be examined.' : 'You can choose other folders or run a different comparison.'));
      $('#result-groups').append(empty);
    }
    const pagination = $('#result-pagination');
    pagination.replaceChildren();
    pagination.hidden = groups.length <= GROUP_PAGE_SIZE;
    if (!pagination.hidden) {
      pagination.append(el('span', '', `Page ${page + 1} of ${Math.ceil(groups.length / GROUP_PAGE_SIZE)} · ${GROUP_PAGE_SIZE} groups per page`));
      const controls = el('div', 'pagination-controls');
      const previous = button('Previous', 'button button-small', () => { page--; render(); });
      const next = button('Next', 'button button-small', () => { page++; render(); });
      previous.disabled = page === 0 || busy;
      next.disabled = (page + 1) * GROUP_PAGE_SIZE >= groups.length || busy;
      controls.append(previous, next);
      pagination.append(controls);
    }
    renderIssues();
    refreshSelection();
  }

  search.addEventListener('input', () => { page = 0; render(); });
  sort.addEventListener('change', () => { page = 0; render(); });
  return {
    setResult(value) { result = value; groupIndex = new Map((value?.groups || []).map(group => [String(group.id), group])); page = 0; selected.clear(); kept.clear(); filePages.clear(); search.value = ''; render(); },
    setBusy(value) { busy = value; search.disabled = value; sort.disabled = value; render(); },
    clearSelection() { selected.clear(); refreshSelection(); },
    selectedFiles,
    getResult: () => result,
    applyCompleted(paths, issues) {
      const completed = new Set(paths);
      result.groups = result.groups.map(group => {
        const remaining = group.files.filter(file => !completed.has(file.path));
        return { ...group, files: remaining, redundant_bytes: group.size * Math.max(0, remaining.length - 1) };
      }).filter(group => group.files.length >= 2);
      groupIndex = new Map(result.groups.map(group => [String(group.id), group]));
      result.issues.push(...issues);
      const valid = new Set(files().map(file => file.path));
      for (const path of selected) if (!valid.has(path)) selected.delete(path);
      render();
    },
  };
}
