import { Download, LoaderCircle, Pause, RotateCcw } from 'lucide-react';
import type { EngineStatus, ModelProgress, ModelSetupStage } from '../types';
import { Button } from './ui/button';

const formatBytes = (bytes: number) => bytes >= 1e9 ? `${(bytes / 1e9).toFixed(2)} GB` : `${(bytes / 1e6).toFixed(1)} MB`;

interface Props {
  engine: EngineStatus | null;
  stage: ModelSetupStage;
  progress: ModelProgress | null;
  error: string;
  pausing: boolean;
  disabled: boolean;
  onInstall: () => void;
  onPause: () => void;
}

export default function ModelPreparation({ engine, stage, progress, error, pausing, disabled, onInstall, onPause }: Props) {
  if (engine?.ready) return null;

  if (stage === 'idle' && engine && !engine.canPrepare) {
    return <p className="model-support" role="status">
      {engine.error || '本地生成支持配有 NVIDIA 显卡的 Windows 和 Linux 设备。'}
    </p>;
  }

  const installing = stage === 'installing';
  const active = stage !== 'idle';
  const downloading = installing && progress?.phase === 'downloading';
  const paused = progress?.phase === 'paused';
  const hasAssets = engine?.modelInstalled && engine.runtimeInstalled;
  const failure = error || (progress?.phase === 'error' ? progress.message : '');
  const asset = progress?.source === 'Albatross 运行时' ? 'Albatross 运行时' : '模型';
  const message = stage === 'checking' ? '正在检查本地模型'
    : stage === 'starting' ? '正在启动本地模型'
    : downloading ? `正在下载${asset}`
    : installing && progress?.phase === 'verifying' ? `正在校验${asset}`
    : installing && progress?.phase === 'importing' ? (asset === '模型' ? '正在导入模型' : '正在解压 Albatross 运行时')
    : installing ? '正在准备本地模型'
    : paused ? '下载已暂停'
    : failure ? '本地模型未就绪'
    : hasAssets ? '本地模型已安装'
    : '需要安装本地模型';
  const percent = progress?.totalBytes ? Math.min(100, progress.downloadedBytes * 100 / progress.totalBytes) : null;

  return <section className="model-preparation" aria-label="本地模型准备" aria-busy={active}>
    <div className="model-preparation-heading">
      <p role="status">
        {active ? <LoaderCircle size={16} className="spin" aria-hidden="true"/> : <Download size={16} aria-hidden="true"/>}
        <strong>{message}</strong>
      </p>
      {downloading && <Button type="button" variant="outline" onClick={onPause} disabled={pausing}>
        <Pause size={14} aria-hidden="true"/>{pausing ? '暂停中…' : '暂停'}
      </Button>}
      {!active && <Button type="button" onClick={onInstall} disabled={disabled}>
        {paused || failure ? <RotateCcw size={14} aria-hidden="true"/> : <Download size={14} aria-hidden="true"/>}
        {paused ? '继续下载' : failure ? '重试' : hasAssets ? '启动模型' : '安装模型'}
      </Button>}
    </div>
    {(downloading || paused) && progress && <div className="model-preparation-progress">
      <progress aria-label={`${asset}下载进度`} max={progress.totalBytes || 1}
        value={progress.totalBytes ? Math.min(progress.downloadedBytes, progress.totalBytes) : undefined}/>
      <div>
        <span>{formatBytes(progress.downloadedBytes)}{percent !== null && ` / ${formatBytes(progress.totalBytes)} · ${percent.toFixed(1)}%`}</span>
        {progress.speedBps > 0 && <span>{formatBytes(progress.speedBps)}/s</span>}
      </div>
    </div>}
    {failure && <p className="model-preparation-error" role="alert">{failure}</p>}
    {!active && !paused && !failure && !hasAssets &&
      <p className="model-preparation-hint">首次会下载 Albatross 运行时和约 3.06 GB 的 RWKV-7 模型，安装后可离线生成。</p>}
  </section>;
}
