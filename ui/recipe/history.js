// Recipe edit history only. This module never invokes filesystem operations.
export function createEditHistory(initial) {
  let present = structuredClone(initial), past = [], future = [];
  const copy = () => structuredClone(present);
  return {
    get: copy,
    get canUndo() { return past.length > 0; },
    get canRedo() { return future.length > 0; },
    change(value) {
      if (JSON.stringify(value) === JSON.stringify(present)) return false;
      past.push(present); if (past.length > 100) past.shift();
      present = structuredClone(value); future = []; return true;
    },
    undo() { if (!past.length) return false; future.push(present); present = past.pop(); return true; },
    redo() { if (!future.length) return false; past.push(present); present = future.pop(); return true; },
    reset(value) { present = structuredClone(value); past = []; future = []; },
    clear() { past = []; future = []; },
  };
}
