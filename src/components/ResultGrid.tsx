import { memo, useEffect, useMemo, useRef, useState } from 'react';
import type { PageJob } from '../types';
import StreamingPreview from '../StreamingPreview';
import { extractPreview } from '../preview';

// Adapted from the reference ResultGrid/useGridMetrics: render at 1280 px,
// then scale the whole document to the card instead of forcing mobile breakpoints.
const PREVIEW_FRAME_WIDTH = 1280;
function ResultCard({ job, scale, viewportHeight, onOpen, onPainted }: { job: PageJob; scale: number; viewportHeight: number; onOpen: (id: string) => void; onPainted: (id: string) => void }) {
  const html = useMemo(() => job.html || extractPreview(job.raw), [job.html, job.raw]);
  return <article className="result-card" role="button" tabIndex={0} aria-label={`查看${job.title}`} onClick={() => onOpen(job.id)} onKeyDown={event => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onOpen(job.id); } }} style={{ height: viewportHeight * scale }}>
    <StreamingPreview html={html} streaming={job.status === 'running'} label={`${job.title}实时预览`}
      logicalWidth={PREVIEW_FRAME_WIDTH} logicalHeight={viewportHeight} scale={scale} interactive={false}
      onPainted={() => onPainted(job.id)}/>
  </article>;
}
const Card = memo(ResultCard);
export default function ResultGrid({ results, onOpen, onPainted }: { results: PageJob[]; onOpen: (id: string) => void; onPainted: (id: string) => void }) {
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
  return <div className="result-grid" ref={gridRef} style={{ gridTemplateColumns: `repeat(${metrics.columns}, minmax(0,1fr))` }}>{results.map(job => <Card key={job.id} job={job} scale={metrics.scale} viewportHeight={metrics.viewportHeight} onOpen={onOpen} onPainted={onPainted}/>)}</div>;
}
