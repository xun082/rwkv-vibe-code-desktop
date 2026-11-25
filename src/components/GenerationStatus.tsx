import { Check, CircleAlert, LoaderCircle, Square } from 'lucide-react';
import type { InferenceMetrics, PageJob } from '../types';
import { formatTokens, formatTps, seconds, summarizePerformance } from '../lib/performance';
export default function GenerationStatus({ results, loading, isGenerating, now, stop, batchStats }: { results: PageJob[]; loading: boolean; isGenerating: boolean; now: number; stop: () => void; batchStats: InferenceMetrics | null }) {
  if (!results.length) return null;
  const complete = results.filter(job => !['queued', 'running'].includes(job.status)).length;
  const metrics = summarizePerformance(results, now);
  const hasError = results.some(job => job.status === 'error');
  return <div className="generation-status">
    <div className="generation-badge">
      {isGenerating ? <LoaderCircle size={14} className="spin"/> : hasError ? <CircleAlert size={14}/> : <Check size={14}/>}
      <span className="completion-count">{complete}/{results.length}</span>
      <span className="status-divider"/>
      <strong className="aggregate-tps" title="Albatross 上报的批量解码速度">{formatTps(batchStats?.tps ?? metrics.tps)} <small>TPS</small></strong>
      <span className="output-token-count" title="服务端上报的批量输出 token 总数">{formatTokens(batchStats?.outputTokens ?? metrics.outputTokens)} tokens</span>
      <span className="secondary-metric" title="从点击生成开始计时">{seconds(metrics.elapsedMs)}</span>
      {loading && <span className="loading-label">准备推理…</span>}
    </div>
    {isGenerating && <button className="stop-button" onClick={stop} title="终止生成" aria-label="终止生成"><Square size={13} fill="currentColor"/></button>}
  </div>;
}
