import { call } from './api.js';
export async function watchJob(jobId, { signal, onProgress = () => {} } = {}) {
  while (!signal?.aborted) {
    const status = await call('job_status', { request: { jobId } });
    if (signal?.aborted) break;
    if (status.progress) onProgress(status.progress);
    if (status.state === 'completed') return status.result;
    if (status.state === 'cancelled') throw new DOMException('Job cancelled', 'AbortError');
    if (status.state === 'failed') throw Object.assign(new Error(status.error?.message || 'Job failed'), status.error);
    await new Promise((resolve, reject) => {
      const abort = () => { clearTimeout(timer); signal?.removeEventListener('abort', abort); reject(new DOMException('Job cancelled', 'AbortError')); };
      const timer = setTimeout(() => { signal?.removeEventListener('abort', abort); resolve(); }, 150);
      signal?.addEventListener('abort', abort, { once: true });
      if (signal?.aborted) abort();
    });
  }
  throw new DOMException('Job cancelled', 'AbortError');
}
