const invoke = window.__TAURI__?.core?.invoke;
export const isDesktop = typeof invoke === 'function';
export async function call(command, args = {}) {
  if (!isDesktop) throw new Error('Open the desktop app to access your files.');
  try {
    return await invoke(command, args);
  } catch (error) {
    if (error && typeof error === 'object' && typeof error.message === 'string') {
      throw Object.assign(new Error(error.message), error);
    }
    throw error;
  }
}
// Sample data is available only via the explicit browser preview button.
export function sampleResult() {
  const groups = [
    { size: 2487392256, paths: ['E:\\Video archive\\Travel\\Coastal road — final.mp4', 'F:\\Backup\\Travel\\Coastal road — final.mp4', 'E:\\Video archive\\Exports\\Coastal road — copy.mp4'] },
    { size: 1295845376, paths: ['E:\\Video archive\\Outdoors\\Autumn in the mountains.mov', 'F:\\Backup\\2025\\Autumn in the mountains.mov'] },
    { size: 687153152, paths: ['E:\\Video archive\\Personal\\City lights at dusk.mkv', 'F:\\Backup\\Personal\\City lights at dusk.mkv'] },
  ].map((group, index) => ({ id: `sample-${index}`, size: group.size, hash: null, redundant_bytes: group.size * (group.paths.length - 1), files: group.paths.map(path => ({ path, name: path.split('\\').pop(), size: group.size, modified_ms: null })) }));
  return { mode: 'size', groups, issues: [], files_scanned: 1824, bytes_scanned: 1630422892544, elapsed_ms: 3200, cancelled: false };
}
