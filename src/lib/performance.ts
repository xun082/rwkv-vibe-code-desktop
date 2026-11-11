import type { PageJob } from '../types';
export const isActive = (job: PageJob) => job.status === 'queued' || job.status === 'running';
export function summarizePerformance(jobs: PageJob[], now: number) {
  const outputTokens = jobs.reduce((sum, job) => sum + (job.stats?.outputTokens || 0), 0);
  const started = jobs.filter(job => job.startedAt !== undefined);
  const start = started.length ? Math.min(...started.map(job => job.startedAt!)) : now;
  const requestEnd = (job: PageJob) => job.endedAt ?? (job.startedAt !== undefined ? job.startedAt + (job.stats?.elapsedMs || (job.seconds || 0) * 1000) : job.createdAt);
  const end = started.length ? Math.max(...started.map(job => isActive(job) ? Math.max(now, requestEnd(job)) : requestEnd(job))) : now;
  const inferenceMs = Math.max(0, end - start);
  return { outputTokens, inferenceMs, tps: outputTokens && inferenceMs > 0 ? outputTokens * 1000 / inferenceMs : null, elapsedMs: jobs.length ? Math.max(0, (jobs.some(isActive) ? Math.max(now, end) : Math.max(...jobs.map(requestEnd))) - Math.min(...jobs.map(job => job.createdAt))) : 0 };
}
export function waitingStage(raw: string, tokens = 0) {
  if (!tokens && !raw) return '正在处理输入，等待首个 token';
  const body = /<body(?:\s|>)/i.exec(raw);
  if (!body && /<style(?:\s|>)/i.test(raw)) return '正在生成样式，正文尚未输出';
  if (!body) return '正在生成 HTML 结构';
  return '正在生成正文，等待首次绘制';
}
export const seconds = (ms: number) => `${(Math.max(0, ms) / 1000).toFixed(1)}s`;
export const formatTps = (tps: number | null | undefined) => tps != null && Number.isFinite(tps) ? tps.toFixed(1) : '—';
