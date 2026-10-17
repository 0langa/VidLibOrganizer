export function desktopBridge(fixture) {
  let serial = 0, planSerial = 0, revision = '0', lastPlan = null, lastInputRevision = null;
  const jobs = new Map();
  const bridge = window.testBridge = { appliedTargets: [], calls: [], delayPreview: false, pending: [], cancelled: [], terminalReads: 0, pageRows: 1, history: null, pendingConfirm: [], delayConfirm: false, finishedConfirm: 0 };
  window.__TAURI__ = { core: { invoke: async (command, args = {}) => {
    bridge.calls.push({ command, args });
    const r = args.request;
    if(command==='rename_session') return {revision:'0',sessionId:fixture.planRef.sessionId};
    if (['load_libraries'].includes(command)) return [];
    if (command === 'scan_status') return { id:null, running:false, acting:false, result_ready:false };
    if (command === 'update_rename_inputs') { revision = r.revision; return null; }
    if (command === 'select_sources') return { revision, sourceCount:'1', sources:[{grantId:'00000000-0000-4000-8000-000000000004', display:'Library'}] };
    if (command === 'start_inventory') { const id = '10000000-0000-4000-8000-'+String(++serial).padStart(12,'0'); jobs.set(id, { state:'completed', result:{ inventoryId:'00000000-0000-4000-8000-000000000005', revision:r.revision, itemCount:'1' } }); return id; }
    if (command === 'inventory_page') return { items:[{itemId:fixture.row.itemId, displayName:'original.mp4', sizeBytes:fixture.row.sizeBytes}], nextCursor:null, revision:r.query.expectedRevision };
    if (command === 'start_rename_preview') { const id = '10000000-0000-4000-8000-'+String(++serial).padStart(12,'0'); const ref = {...fixture.planRef, planId:'20000000-0000-4000-8000-'+String(++planSerial).padStart(12,'0'), revision:String(planSerial)}; lastPlan=ref;lastInputRevision=r.revision; const done = () => jobs.set(id, { state:'completed', result:{ planRef:ref, counts:{changed:'1', unchanged:'0', blocked:'0', needsReview:'0'}, inputRevision:r.revision } }); if (bridge.delayPreview) { jobs.set(id,{state:'running'}); bridge.pending.push(done); } else done(); return id; }
    if (command === 'job_status') { const value = jobs.get(r.jobId); if (value.state === 'completed') bridge.terminalReads++; return value; }
    if (command === 'cancel_job') { bridge.cancelled.push(r.jobId); jobs.set(r.jobId,{state:'cancelled'}); return; }
    if (command === 'rename_page') { const start=Number(r.query.cursor || 0); const end=Math.min(start+r.query.limit,bridge.pageRows); return {items:Array.from({length:end-start},()=>fixture.row),nextCursor:end<bridge.pageRows?String(end):null, revision:r.query.expectedRevision}; }
    if (command === 'confirm_rename') { const value={ nonce:'reviewed-nonce', summary:{changed:'1', unchanged:'0', blocked:'0', needsReview:'0'} }; if(bridge.delayConfirm) return new Promise(resolve=>bridge.pendingConfirm.push(()=>{bridge.finishedConfirm++;resolve(value);})); return value; }
    if (command === 'apply_rename') { if (lastInputRevision !== revision || r.planRef.planId !== lastPlan.planId || r.nonce !== 'reviewed-nonce') throw fixture.error; bridge.appliedTargets.push(r.planRef.planId); const id='10000000-0000-4000-8000-'+String(++serial).padStart(12,'0'); jobs.set(id,{state:'completed',result:{operationId:'00000000-0000-4000-8000-000000000006',state:'completed',completedGroups:'1',completedSteps:'1',failure:null}}); return id; }
    if (command === 'operation_history') return bridge.history || {items:[],nextCursor:null,pendingOperations:[],legacyUnresolved:false};
    if(command==='recovery_preview') return {recoveryRef:'00000000-0000-4000-8000-000000000007',nonce:'00000000-0000-4000-8000-000000000008',operationId:r.operationId,choices:['restore'],continuingRestore:true,unresolvedGroups:'0',actions:[],actionCount:'1'};
    throw new Error('Unexpected command: '+command);
  } } };
}
