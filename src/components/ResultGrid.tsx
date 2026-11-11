import { memo, useEffect, useMemo, useRef, useState } from 'react';
import type { PageJob } from '../types';
import StreamingPreview from '../StreamingPreview';
import { extractPreview } from '../preview';
import { formatTps, seconds, waitingStage } from '../lib/performance';

// Adapted from the reference ResultGrid/useGridMetrics: render at 1280 px,
// then scale the whole document to the card instead of forcing mobile breakpoints.
const PREVIEW_FRAME_WIDTH = 1280;
function ResultCard({ job, index, scale, viewportHeight, onOpen, onPainted, now }: { job: PageJob; index: number; scale: number; viewportHeight: number; onOpen: (id: string) => void; onPainted: (id: string) => void; now: number }) {
  const running = job.status === 'running';
  const html = useMemo(() => job.html || extractPreview(job.raw), [job.html, job.raw]);
  return <article className="result-card" role="button" tabIndex={0} aria-label={`查看方案 ${index + 1}`} onClick={() => onOpen(job.id)} onKeyDown={event => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onOpen(job.id); } }} style={{ height: viewportHeight * scale }}>
    <StreamingPreview html={html} streaming={running} queued={job.status === 'queued'} label={`方案 ${index + 1}实时预览`} logicalWidth={PREVIEW_FRAME_WIDTH} logicalHeight={viewportHeight} scale={scale} interactive={false} progress={job} now={now} onPainted={() => onPainted(job.id)}/>
    <span className="result-number">{index + 1}</span>
    <span className={`result-status metrics ${running ? 'running' : ''}`} title={running ? waitingStage(job.raw, job.stats?.outputTokens) : '该路生成指标由 llama-server 实际计时返回'}>{running && <i/>}{job.status === 'cancelled' && <span>已停止 ·</span>}<strong>{formatTps((job.stats?.outputTokens || 0) > 1 ? job.stats?.tps : null)} TPS</strong><span>{(job.stats?.outputTokens || 0).toLocaleString()} tokens</span></span>
    <span className="result-timing" title="首 token：推理请求到首次输出；首屏就绪：点击生成到非空预览 DOM 准备好（含模型加载）">{job.stats?.firstTokenMs != null ? `首 token ${seconds(job.stats.firstTokenMs)}` : '等待首 token'}{job.firstRenderMs !== undefined ? ` · 首屏就绪 ${seconds(job.firstRenderMs)}` : running ? ' · 首屏等待中' : ''}</span>
    {job.error && <span className="result-notice error">{job.error}</span>}
    {job.incomplete && <span className="result-notice">内容未完成</span>}
    {!running && job.status !== 'queued' && <span className="result-open">方案 {index + 1}</span>}
  </article>;
}
const Card = memo(ResultCard);
export default function ResultGrid({ results, onOpen, onPainted, now }: { results: PageJob[]; onOpen: (id: string) => void; onPainted: (id: string) => void; now: number }) {
  const gridRef = useRef<HTMLDivElement>(null);
  const [metrics, setMetrics] = useState({ columns: 2, scale: .5, viewportHeight: 1000 });
  useEffect(() => {
    const grid = gridRef.current, viewport = grid?.parentElement;
    if (!grid || !viewport) return;
    const update = () => {
      const width = grid.clientWidth;
      if (!width) return;
      const columns = width < 640 || results.length === 1 ? 1 : 2;
      const scale = (width - 16 * (columns - 1)) / columns / PREVIEW_FRAME_WIDTH;
      const rows = Math.ceil(results.length / columns);
      const rowHeight = Math.max((viewport.clientHeight - 32 - 16 * (rows - 1)) / rows, 240);
      setMetrics({ columns, scale, viewportHeight: Math.max(rows > 1 ? 320 : 700, Math.min(Math.round(rowHeight / scale), 1600)) });
    };
    update(); const observer = new ResizeObserver(update); observer.observe(grid); observer.observe(viewport);
    return () => observer.disconnect();
  }, [results.length]);
  return <div className="result-grid" ref={gridRef} style={{ gridTemplateColumns: `repeat(${metrics.columns}, minmax(0,1fr))` }}>{results.map((job, index) => <Card key={job.id} job={job} index={index} scale={metrics.scale} viewportHeight={metrics.viewportHeight} onOpen={onOpen} onPainted={onPainted} now={now}/>)}</div>;
}
