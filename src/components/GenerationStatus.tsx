import { Check, CircleAlert, LoaderCircle, Square } from 'lucide-react';
import type { InferenceMetrics, PageJob } from '../types';
import { formatTokens, formatTps, seconds, summarizePerformance } from '../lib/performance';
export default function GenerationStatus({ results, loading, isGenerating, now, stop, batchStats }: { results: PageJob[]; loading: boolean; isGenerating: boolean; now: number; stop: () => void; batchStats: InferenceMetrics | null }) {
  if (!results.length) return null;
  const complete = results.filter(job => !['queued', 'running'].includes(job.status)).length;
  const metrics = summarizePerformance(results, now);
  const hasError = results.some(job => job.status === 'error');
  const tps = formatTps(batchStats?.tps);
  const tokens = formatTokens(batchStats?.outputTokens);
  const streamRate = batchStats?.rateKind === 'stream';
  const ratePrefix = streamRate ? (isGenerating ? '实时约' : '约') : '解码';
  const rateTitle = streamRate
    ? '按 Albatross 的流式输出 token 计数变化估算'
    : tps !== '—'
      ? 'Albatross 上报的本批平均解码速度'
      : '等待 Albatross 的吞吐量统计';

  return <div className="generation-status">
    <div className="generation-badge">
      {isGenerating ? <LoaderCircle size={14} className="spin"/> : hasError ? <CircleAlert size={14}/> : <Check size={14}/>}
      <span className="completion-count">{complete}/{results.length}</span>
      <span className="status-divider"/>
      <strong className="aggregate-tps" title={rateTitle}>
        {tps !== '—' && <small>{ratePrefix} </small>}{tps} <small>TPS</small>
      </strong>
      {tokens !== '—' && <span className="output-token-count" title={streamRate ? '本批已输出的估算 token 数' : 'Albatross 上报的本批输出 token 总数'}>
        {streamRate && '约 '}{tokens} tokens
      </span>}
      <span className="secondary-metric" title="从点击生成开始计时">{seconds(metrics.elapsedMs)}</span>
      {loading && <span className="loading-label">准备推理…</span>}
    </div>
    {isGenerating && <button className="stop-button" onClick={stop} title="终止生成" aria-label="终止生成"><Square size={13} fill="currentColor"/></button>}
  </div>;
}
