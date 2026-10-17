import { $, el, icon, button, basename, displayPath } from './dom.js';

export function renderLibraries({ paths, included, locked, onToggle, onRemove, onManage }) {
  $('#library-count').textContent = paths.length;
  $('#saved-count').textContent = paths.length;
  const roots = $('#scan-roots');
  const library = $('#library-list');
  roots.replaceChildren();
  library.replaceChildren();
  if (!paths.length) {
    const empty = el('div', 'root-empty');
    const text = el('div');
    text.append(el('strong', '', 'Your next clear-out starts here'), el('p', '', 'Add a local folder or external drive to get started.'));
    empty.append(icon('folder'), text, button('Add folders →', 'text-button', onManage));
    roots.append(empty);
    const largeEmpty = el('div', 'empty-state');
    largeEmpty.append(el('h3', '', 'Your library starts with a folder.'), el('p', '', 'Add a path above or browse for a folder on your computer.'));
    library.append(largeEmpty);
  }
  paths.forEach(path => {
    const label = el('label', 'root-option');
    const checkbox = el('input');
    checkbox.type = 'checkbox';
    checkbox.checked = included.has(path);
    checkbox.disabled = locked;
    checkbox.setAttribute('aria-label', `Include ${displayPath(path)} in scan`);
    checkbox.addEventListener('change', () => onToggle(path, checkbox.checked));
    const description = el('span', 'root-description');
    description.append(el('span', 'root-name', basename(path)), el('span', 'root-path mono', displayPath(path)));
    label.append(checkbox, icon('drive'), description, el('span', 'root-recursive', 'SUBFOLDERS INCLUDED'));
    roots.append(label);
    const row = el('div', 'library-row');
    const details = el('div', 'file-description');
    details.append(el('strong', 'file-name', basename(path)), el('span', 'file-path', displayPath(path)));
    const remove = button('', 'icon-button', () => onRemove(path), 'close');
    remove.setAttribute('aria-label', `Remove saved folder ${displayPath(path)}`);
    remove.title = 'Remove from saved folders';
    remove.disabled = locked;
    row.append(icon('drive'), details, remove);
    library.append(row);
  });
}
