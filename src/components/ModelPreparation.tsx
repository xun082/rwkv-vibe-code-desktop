import { LoaderCircle, Pause, RotateCcw } from 'lucide-react';
import type { ModelProgress } from '../types';

const formatBytes = (bytes: number) => bytes >= 1e9 ? `${(bytes / 1e9).toFixed(2)} GB` : `${(bytes / 1e6).toFixed(1)} MB`;
export default function ModelPreparation({ progress, onPause, onRetry, compact = false }: { progress: ModelProgress | null; onPause: () => void; onRetry: () => void; compact?: boolean }) {
  if (!progress || progress.phase === 'ready') return null;
  const waiting = progress.phase === 'paused' || progress.phase === 'error';
  const downloading = progress.phase === 'downloading';
  const percent = progress.totalBytes ? Math.min(100, progress.downloadedBytes * 100 / progress.totalBytes) : 0;
  return <section className={`model-preparation ${compact ? 'compact' : ''}`} aria-label="本地模型准备">
    <div className="model-preparation-heading"><span>{!waiting && <LoaderCircle className="spin" size={14}/>}<strong>{progress.message || '正在准备本地模型'}</strong></span>
      {waiting ? <button type="button" onClick={onRetry}><RotateCcw size={13}/>{progress.phase === 'paused' ? '继续下载' : '重试'}</button> : downloading ? <button type="button" onClick={onPause}><Pause size={13}/>暂停</button> : null}
    </div>
    {(downloading || waiting) && <><progress aria-label="模型下载进度" max={progress.totalBytes || 1} value={progress.downloadedBytes}/><div className="model-preparation-detail"><span>{formatBytes(progress.downloadedBytes)} / {formatBytes(progress.totalBytes)} · {percent.toFixed(1)}%</span><span>{progress.speedBps > 0 ? `${formatBytes(progress.speedBps)}/s · ` : ''}{progress.source}</span></div></>}
    <p>首次使用需准备约 1.34 GB 模型，完成后可离线推理。下载中断会保留进度。</p>
  </section>;
}
