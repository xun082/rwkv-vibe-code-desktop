import { Check, LoaderCircle, Square } from 'lucide-react';
import type { PageJob } from '../types';
import { formatTps, seconds, summarizePerformance } from '../lib/performance';
export default function GenerationStatus({ results, loading, isGenerating, now, stop }: { results: PageJob[]; loading: boolean; isGenerating: boolean; now: number; stop: () => void }) {
  if (!results.length) return null;
  const complete = results.filter(job => !['queued', 'running'].includes(job.status)).length;
  const metrics = summarizePerformance(results, now);
  return <div className="generation-status">
    <div className="generation-badge">
      {isGenerating ? <LoaderCircle size={14} className="spin"/> : <Check size={14}/>}
      <span className="completion-count">{complete}/{results.length}</span>
      <span className="status-divider"/>
      <strong className="aggregate-tps" title="合计吞吐：引擎上报的输出 token 总数 ÷ 推理经过时间（含提示词处理，不含加载模型）">{formatTps(metrics.tps)} <small>TPS</small></strong>
      <span className="output-token-count" title="llama-server 上报的实际输出 token 数，非字符数">{metrics.outputTokens.toLocaleString()} tokens</span>
      <span className="secondary-metric" title="从点击生成开始计时，包含模型加载">{seconds(metrics.elapsedMs)}</span>
      {loading && <span className="loading-label">加载模型…</span>}
    </div>
    {isGenerating && <button className="stop-button" onClick={stop} title="终止生成" aria-label="终止生成"><Square size={13} fill="currentColor"/></button>}
  </div>;
}
