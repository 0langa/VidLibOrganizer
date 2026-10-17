export function desktopBridge(fixture) {
  let serial = 0, planSerial = 0, revision = '0', lastPlan = null, lastInputRevision = null;
  let recoveryRevision = null;
  const jobs = new Map();
  const counts = { changed: '1', unchanged: '0', blocked: '0', needsReview: '0' };
  const bridge = window.testBridge = {
    appliedTargets: [], calls: [], delayPreview: false, pending: [], cancelled: [], terminalReads: 0,
    pageRows: 1, summary: { selected:'1', files:'1', folders:'0', carriedDescendants:'0', companions:'0', metadataActions:'0', destinationMode:'inPlace', destinationCount:'1', destinations:['Library'], isUndo:false, createdCopyRemovals:'0', ownedDirectoryRemovals:'0' }, history: null, historyRevision: '1', historyStaleOnce: false,
    pendingConfirm: [], delayConfirm: false, finishedConfirm: 0,
    delayStartOnce: false, pendingStarts: [], releasedStarts: 0,
    delayStatusOnce: false, pendingStatuses: [], releasedStatuses: 0,
    recoveryActions: 1, recoveryPages: [], delayRecoveryPageOnce: false,
    pendingRecoveryPages: [], releasedRecoveryPages: 0,
  };
  const id = () => '10000000-0000-4000-8000-' + String(++serial).padStart(12, '0');
  const complete = (jobId, result) => jobs.set(jobId, { state: 'completed', result, error: null, progress: null });
  function recoveryPage(request) {
    if (request.recoveryRef !== fixture.recoveryPageRequest.recoveryRef || request.query.expectedRevision !== recoveryRevision || revision !== recoveryRevision) throw fixture.error;
    const start = Number(request.query.cursor || 0), end = Math.min(start + Math.min(request.query.limit, 200), bridge.recoveryActions);
    return {
      items: Array.from({ length: end - start }, (_, i) => ({
        stepId: '30000000-0000-4000-8000-' + String(start + i).padStart(12, '0'),
        metadata: bridge.recoveryMetadata || null, disposition: 'planned', beforeDisplay: 'before-' + String(start + i).padStart(3, '0'),
        afterDisplay: 'after-' + String(start + i).padStart(3, '0'),
      })),
      nextCursor: end < bridge.recoveryActions ? String(end) : null, revision: request.query.expectedRevision,
    };
  }
  window.__TAURI__ = { core: { invoke: async (command, args = {}) => {
    bridge.calls.push({ command, args });
    const r = args.request;
    if (command === 'rename_session') return { revision: '0', sessionId: fixture.planRef.sessionId };
    if (command === 'load_libraries') return [];
    if (command === 'scan_status') return { id: null, running: false, acting: false, result_ready: false };
    if (command === 'update_rename_inputs') {
      if (BigInt(r.revision) < BigInt(revision)) throw fixture.error;
      revision = r.revision; return null;
    }
    if (command === 'pending_native_selection') return bridge.pendingNative || null;
    if (command === 'accept_native_selection') { bridge.pendingNative=null; revision=String(BigInt(revision)+1n); return {revision,sourceCount:'1',sources:[{grantId:'00000000-0000-4000-8000-000000000004',display:'Dropped folder'}]}; }
    if (command === 'select_sources') { revision=String(BigInt(revision)+1n); return { revision, sourceCount: '1', sources: [{ grantId: '00000000-0000-4000-8000-000000000004', display: 'Library' }] }; }
    if (command === 'start_inventory') {
      const jobId = id(); complete(jobId, { inventoryId: '00000000-0000-4000-8000-000000000005', revision: r.revision, itemCount: '1' }); return jobId;
    }
    if (command === 'inventory_page') return { items: [{ itemId: fixture.row.itemId, displayName: 'original.mp4', sizeBytes: fixture.row.sizeBytes }], nextCursor: null, revision: r.query.expectedRevision };
    if (command === 'start_rename_preview' || command === 'start_undo') {
      if (r.revision !== revision) throw fixture.error;
      const jobId = id();
      const reference = { ...fixture.planRef, planId: '20000000-0000-4000-8000-' + String(++planSerial).padStart(12, '0'), revision: String(planSerial) };
      lastPlan = reference; lastInputRevision = r.revision; if(command==='start_undo') bridge.summary={...bridge.summary,isUndo:true,createdCopyRemovals:'1',ownedDirectoryRemovals:'1'};
      const done = () => complete(jobId, { planRef: reference, counts, summary:command==='start_undo'?{...bridge.summary,isUndo:true,createdCopyRemovals:'1',ownedDirectoryRemovals:'1'}:bridge.summary, inputRevision: r.revision });
      if (bridge.delayPreview) { jobs.set(jobId, { state: 'running', result: null, error: null, progress: null }); bridge.pending.push(done); }
      else done();
      if (bridge.delayStartOnce) {
        bridge.delayStartOnce = false;
        return new Promise(resolve => bridge.pendingStarts.push(() => { bridge.releasedStarts++; resolve(jobId); }));
      }
      return jobId;
    }
    if (command === 'job_status') {
      const value = jobs.get(r.jobId);
      if (value.state === 'completed') bridge.terminalReads++;
      if (bridge.delayStatusOnce) {
        bridge.delayStatusOnce = false;
        return new Promise(resolve => bridge.pendingStatuses.push(() => { bridge.releasedStatuses++; resolve(value); }));
      }
      return value;
    }
    if (command === 'cancel_job') { bridge.cancelled.push(r.jobId); jobs.set(r.jobId, { state: 'cancelled', result: null, error: null, progress: null }); return; }
    if (command === 'export_rename_plan') return {text: r.mode === 'exactMapping' ? 'old_path,new_path\r\noriginal.mp4,renamed.mp4\r\n' : 'status,old_name,new_name\r\nchanged,original.mp4,renamed.mp4\r\n',rowCount:'1'};
    if (command === 'rename_page') {
      const start = Number(r.query.cursor || 0), end = Math.min(start + r.query.limit, bridge.pageRows);
      return { items: Array.from({ length: end - start }, () => fixture.row), nextCursor: end < bridge.pageRows ? String(end) : null, revision: r.query.expectedRevision };
    }
    if (command === 'confirm_rename') {
      if (lastInputRevision !== revision || r.planRef.planId !== lastPlan.planId) throw fixture.error;
      const value = { nonce: 'reviewed-nonce', summary: {...counts,...bridge.summary} };
      if (bridge.delayConfirm) return new Promise(resolve => bridge.pendingConfirm.push(() => { bridge.finishedConfirm++; resolve(value); }));
      return value;
    }
    if (command === 'apply_rename') {
      if (lastInputRevision !== revision || r.planRef.planId !== lastPlan.planId || r.nonce !== 'reviewed-nonce') throw fixture.error;
      bridge.appliedTargets.push(r.planRef.planId);
      const jobId = id(); complete(jobId, { operationId: '00000000-0000-4000-8000-000000000006', state: 'completed', completedGroups: '1', completedSteps: '1', failure: null }); return jobId;
    }
    if (command === 'history_revision') return { revision: bridge.historyRevision };
    if (command === 'operation_history') {
      if (bridge.historyStaleOnce) { bridge.historyStaleOnce = false; bridge.historyRevision = String(BigInt(bridge.historyRevision) + 1n); throw fixture.error; }
      if (r.query.expectedRevision !== bridge.historyRevision) throw fixture.error;
      const value = bridge.history || { items: [], pendingOperations: [], legacyUnresolved: false };
      const cursor = r.query.cursor ? JSON.parse(r.query.cursor) : null;
      const through = cursor?.through ?? value.items.length;
      let items = value.items.slice(0, through).map((item, index) => ({ item, index }));
      items = items.filter(({ item }) => `${item.operationId} ${item.state}`.toLowerCase().includes(r.query.filter.toLowerCase()));
      if (r.query.sort === 'newest') items.reverse();
      else if (r.query.sort === 'state' || r.query.sort === 'operationId' || r.query.sort === 'name') {
        const key = r.query.sort === 'state' ? 'state' : 'operationId';
        items.sort((a, b) => a.item[key].localeCompare(b.item[key]) || a.index - b.index);
      } else if (r.query.sort !== 'oldest') throw new Error('Unsupported history sort');
      const start = cursor?.offset || 0, limit = Math.min(Math.max(r.query.limit, 1), 200), end = Math.min(start + limit, items.length);
      return { items: items.slice(start, end).map(row => row.item), nextCursor: end < items.length ? JSON.stringify({ offset: end, through }) : null, revision: bridge.historyRevision, pendingOperations: value.pendingOperations, pendingCount: String(value.pendingOperations.length), legacyUnresolved: value.legacyUnresolved };
    }
    if (command === 'recovery_page') {
      bridge.recoveryPages.push(r);
      const value = recoveryPage(r);
      if (bridge.delayRecoveryPageOnce) { bridge.delayRecoveryPageOnce = false; return new Promise(resolve => bridge.pendingRecoveryPages.push(() => { bridge.releasedRecoveryPages++; resolve(value); })); }
      return value;
    }
    if (command === 'recovery_preview') {
      recoveryRevision = revision;
      const reference = fixture.recoveryPageRequest.recoveryRef;
      return { recoveryRef: reference, nonce: '00000000-0000-4000-8000-000000000008', operationId: r.operationId, revision, choices: ['restore'], continuingRestore: true, unresolvedGroups: '0', actionCount: String(bridge.recoveryActions), page: recoveryPage({ recoveryRef: reference, query: { cursor: null, limit: 200, expectedRevision: revision } }) };
    }
    throw new Error('Unexpected command: ' + command);
  } } };
}
