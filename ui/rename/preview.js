import { el } from '../dom.js';
import { formatExactBytes } from '../exact.js';
/**
 * @typedef {{itemId: string, oldName: string, newName: string, oldDisplay: string, newDisplay: string, sizeBytes: ?string, kind: string, status: string, carriedDescendants: string, issues: string[], explanations: Object[], metadata: Object[]}} PreviewRow
 * @param {HTMLElement} host
 * @param {PreviewRow[]} rows Backend-generated names and exact decimal quantities.
 */
export function renderPreview(host, rows, { onOverride, onExclude, onExcludeGroup } = {}) {
  const table = el('table', 'rename-table');
  const head = el('thead'), header = el('tr');
  for (const title of ['Current name / path', 'Proposed name / path', 'Kind / exact size', 'Status / issues / actions']) header.append(el('th', '', title));
  head.append(header); const body = el('tbody');
  const fragment = document.createDocumentFragment();
  for (const row of rows.slice(0, 200)) {
    const tr = el('tr'); tr.dataset.testid = 'rename-row';
    const oldName = el('td', '', row.oldName); oldName.title = row.oldDisplay;
    const newName = el('td'); newName.title = row.newDisplay;
    const nameText=el('span');highlightDifference(nameText,row.oldName,row.newName);newName.append(nameText);
    oldName.append(el('small', 'rename-path', row.oldDisplay)); newName.append(el('small', 'rename-path', row.newDisplay));
    const size = el('td', 'mono', formatExactBytes(row.sizeBytes)); size.dataset.testid = 'exact-bytes'; size.append(el('small', 'rename-path', row.kind || ''));
    const warnings = [...row.issues, ...(row.companionWarnings || [])];
    const status = el('td', '', `${row.status}${row.carriedDescendants !== '0' ? ` · carries ${row.carriedDescendants} descendants` : ''}${warnings.length ? ` · ${warnings.join('; ')}` : ''}`);
    if (row.association?.isCompanion) status.append(el('p', '', `Linked companion of ${row.association.primaryId}`));
    if (row.explanations?.length) {
      const details = el('details'); details.append(el('summary', '', 'Rule explanations'));
      for (const value of row.explanations) details.append(el('p', '', `${value.ruleId}: ${value.before} → ${value.after}`));
      status.append(details);
    }
    if (row.metadata?.length) {
      const details = el('details'); details.append(el('summary', '', 'Metadata changes'));
      for (const value of row.metadata) {
        details.append(el('p', '', `Frozen original: created ${value.original.createdNs ?? 'unknown'} ns; modified ${value.original.modifiedNs ?? 'unknown'} ns; accessed ${value.original.accessedNs ?? 'unknown'} ns; attributes ${value.original.windowsAttributes ?? 'unknown'}`));
        for (const change of value.requested) details.append(el('p', '', requestedMetadata(change)));
      }
      details.append(el('p', '', 'Actual pre-set state is captured durably at execution and remains distinct from these frozen originals.'));
      status.append(details);
    }
    if (onOverride) {
      const input = el('input'); input.setAttribute('aria-label', 'Override name'); input.value = row.newName;
      const button = el('button', 'button', 'Use row override'); button.addEventListener('click', () => onOverride(row.itemId, input.value)); status.append(input, button);
    }
    if (onExclude) { const button = el('button', 'button', 'Exclude row'); button.addEventListener('click', () => onExclude(row.itemId)); status.append(button); }
    if(onExcludeGroup&&row.association?.groupId){const button=el('button','button','Exclude dependency group');button.addEventListener('click',()=>onExcludeGroup(row.association.groupId));status.append(button);}
    tr.append(oldName, newName, size, status); fragment.append(tr);
  }
  body.append(fragment); table.append(head, body); host.replaceChildren(table);
}
export async function reviewConfirmation({ confirmation, signal, title = 'Review rename', confirmText = 'Apply reviewed rename' }) {
  const dialog = el('dialog', 'rename-dialog'); dialog.setAttribute('aria-labelledby', 'rename-confirm-title');
  const heading = el('h2', '', title); heading.id = 'rename-confirm-title';
  const summary = el('p', '', `${confirmation.summary.changed} changed · ${confirmation.summary.unchanged} unchanged · ${confirmation.summary.blocked} blocked. The batch is recoverable; earlier completed groups can remain after cancellation or failure.`);
  const cancel = el('button', 'button', 'Cancel'); const confirm = el('button', 'button button-primary', confirmText);
  dialog.append(heading, summary); appendSummary(dialog, confirmation.summary); dialog.append(cancel, confirm); document.body.append(dialog);
  const previous = document.activeElement;
  return new Promise(resolve => {
    function close(accepted) { dialog.close(); dialog.remove(); signal?.removeEventListener('abort', abort); previous?.focus(); resolve(accepted); }
    const abort = () => close(false);
    cancel.addEventListener('click', () => close(false)); confirm.addEventListener('click', () => close(true));
    dialog.addEventListener('cancel', event => { event.preventDefault(); close(false); });
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) { close(false); return; }
    dialog.showModal(); cancel.focus();
  });
}

export function appendSummary(host, summary) {
  if (!summary) return;
  if (summary.selected !== undefined) host.append(el('p', '', `${summary.selected} selected across all pages · ${summary.files} files · ${summary.folders} folders · ${summary.carriedDescendants} carried descendants · ${summary.companions} companions · ${summary.metadataActions} metadata actions.`));
  if(summary.unselectedDescendants!==undefined)host.append(el('p','',`${summary.unselectedDescendants} carried descendants are outside the selected table rows. Descendants are counted once, including nested folders.`));
  if (summary.destinationMode) host.append(el('p', '', `Destination: ${summary.destinationMode} · ${summary.destinationCount} folders`));
  for (const destination of summary.destinations || []) host.append(el('p', 'mono', destination));
  if (summary.isUndo || summary.destinationMode === 'copyTo' || summary.destinationMode === 'moveTo') host.append(el('p', '', `Undo removes only created copies with matching identity and full digest (${summary.createdCopyRemovals || '0'} in this plan), and only owned empty directories (${summary.ownedDirectoryRemovals || '0'} in this plan). Cross-volume Undo may copy back and require sufficient free space.`));
}

function highlightDifference(host, before, after) {
  const a=Array.from(before),b=Array.from(after);let prefix=0,suffix=0;
  while(prefix<a.length&&prefix<b.length&&a[prefix]===b[prefix])prefix++;
  while(suffix<a.length-prefix&&suffix<b.length-prefix&&a[a.length-1-suffix]===b[b.length-1-suffix])suffix++;
  host.append(document.createTextNode(b.slice(0,prefix).join('')));
  if(prefix+suffix<b.length)host.append(el('mark','',b.slice(prefix,b.length-suffix).join('')));
  if(suffix)host.append(document.createTextNode(b.slice(b.length-suffix).join('')));
  if(before!==after&&prefix+suffix===b.length)host.append(el('mark','',' [removed text]'));
}

function requestedMetadata(change) {
  return change.type === 'windowsAttributes' ? `Requested attributes: set ${change.setBits}, clear ${change.clearBits}` : `Requested ${change.field}: ${change.valueNs ?? change.offsetNs ?? change.startNs} ns${change.stepNs !== undefined ? `, step ${change.stepNs} ns` : ''}`;
}
export function appendRecoveryMetadata(host, metadata) {
  if (!metadata) return;
  for (const [label, stamp] of [['Frozen operation original', metadata.original], ['Durable factual pre-set', metadata.factualBefore], ['Recorded setter poststate', metadata.observedAfter]]) {
    host.append(el('p', '', `${label}: ${stamp ? `modified ${stamp.modifiedNs ?? 'unknown'} ns; created ${stamp.createdNs ?? 'unknown'} ns; accessed ${stamp.accessedNs ?? 'unknown'} ns; attributes ${stamp.windowsAttributes ?? 'unknown'}` : 'not recorded'}`));
  }
  for (const change of metadata.requested || []) host.append(el('p', '', requestedMetadata(change)));
  host.append(el('p', '', 'Frozen originals supply restoration targets where required. Durable pre-set and recorded poststate are separate execution facts; absent evidence is not reconstructed.'));
}
