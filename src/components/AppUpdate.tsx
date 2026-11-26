import { Download, LoaderCircle, RotateCcw, X } from 'lucide-react';
import type { useAppUpdate } from '../hooks/use-app-update';
import { Button } from './ui/button';
import '../app-update.css';

type UpdateState = ReturnType<typeof useAppUpdate>;

function progressText(progress: NonNullable<UpdateState['progress']>) {
  switch (progress.phase) {
    case 'downloading': {
      if (!progress.totalBytes) return '正在下载更新…';
      const percent = Math.min(100, Math.floor(progress.downloadedBytes * 100 / progress.totalBytes));
      return `下载更新 ${percent}%`;
    }
    case 'verifying': return '正在校验更新…';
    case 'installing': return '正在安装更新…';
    case 'restarting': return '正在重启…';
  }
}

export default function AppUpdate({ updater }: { updater: UpdateState }) {
  const { supported, version, available, busy, checking, progress, feedback, check, install, dismiss } = updater;
  if (!supported) return null;

  const working = checking || progress !== null;
  const waitingForIdle = available !== null && busy;
  const title = waitingForIdle ? '请等当前生成或模型准备完成后再安装更新' : `当前版本 ${version}`;
  const label = progress ? progressText(progress)
    : checking ? '正在检查更新…'
    : available ? `更新至 ${available}`
    : '检查更新';
  const Icon = working ? LoaderCircle : available ? Download : RotateCcw;

  return <div className="app-update">
    <Button type="button" variant={available ? 'default' : 'outline'} className="app-update-action"
      aria-disabled={working || waitingForIdle} aria-description={waitingForIdle ? title : undefined}
      title={title} onClick={available ? install : check}>
      <Icon size={15} className={working ? 'spin' : undefined} aria-hidden="true"/>
      {label}
    </Button>
    {feedback && <div className="app-update-feedback" data-kind={feedback.kind}>
      <div>
        <p role={feedback.kind === 'error' ? 'alert' : 'status'}>{feedback.message}</p>
        {feedback.kind === 'error' && available && <button type="button" onClick={check}>重新检查更新</button>}
      </div>
      <Button type="button" variant="outline" size="icon-sm" className="app-update-close" onClick={dismiss} aria-label="关闭更新提示">
        <X size={14} aria-hidden="true"/>
      </Button>
    </div>}
  </div>;
}
