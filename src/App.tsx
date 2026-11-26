import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowLeft, Code2, Eye, House, LayoutGrid } from 'lucide-react';
import { desktop } from './bridge';
import AppUpdate from './components/AppUpdate';
import PromptInput from './components/PromptInput';
import ModelPreparation from './components/ModelPreparation';
import PromptExamples from './components/PromptExamples';
import ResultGrid from './components/ResultGrid';
import { Button } from './components/ui/button';
import StreamingPreview from './StreamingPreview';
import { useGeneration } from './hooks/use-generation';
import { useAppUpdate } from './hooks/use-app-update';
import { extractPreview } from './preview';
import GenerationStatus from './components/GenerationStatus';
import { DEFAULT_CONCURRENCY, type ConcurrencyMode } from './types';
import { seconds } from './lib/performance';

type View = { page: 'home' | 'results' } | { page: 'detail'; id: string };

export default function App() {
  const [prompt, setPrompt] = useState('');
  const [count, setCount] = useState<ConcurrencyMode>(DEFAULT_CONCURRENCY);
  const [view, setView] = useState<View>({ page: 'home' });
  const { results, loading, engine, setupStage, setupError, modelProgress, pausing, runSetup, pauseDownload, batchStats, error, generate, stop, edit, markPainted, isGenerating } = useGeneration();
  const appUpdate = useAppUpdate(isGenerating || setupStage !== 'idle');
  const hardware = engine?.hardware ?? null;
  const engineReady = engine?.ready === true;
  const effectiveCount = count === 'auto' ? hardware?.recommendedConcurrency ?? DEFAULT_CONCURRENCY : count;
  const isHome = view.page === 'home';
  const hasResults = results.length > 0;
  const detailId = view.page === 'detail' ? view.id : null;
  const detailIndex = results.findIndex(job => job.id === detailId);
  const selected = results[detailIndex];
  const code = selected?.html || extractPreview(selected?.raw || '');
  const editor = useRef<HTMLTextAreaElement>(null);
  const promptInput = useRef<HTMLTextAreaElement>(null);
  const homeHeading = useRef<HTMLHeadingElement>(null);
  const homeButton = useRef<HTMLButtonElement>(null);
  const follow = useRef(true);
  const [now, setNow] = useState(Date.now());

  const openDetail = useCallback((id: string) => {
    follow.current = true;
    setView({ page: 'detail', id });
  }, []);
  const showResults = () => setView({ page: 'results' });
  const submit = () => {
    if (!appUpdate.installing) void generate(prompt, effectiveCount, showResults);
  };

  useEffect(() => {
    if (!isGenerating) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [isGenerating]);

  useEffect(() => {
    if (view.page !== 'home') homeButton.current?.focus();
    else if (hasResults) {
      const input = promptInput.current;
      if (input && !input.disabled) input.focus();
      else homeHeading.current?.focus();
    }
  }, [view, hasResults]);

  useEffect(() => {
    if (selected?.status === 'running' && follow.current && editor.current) {
      editor.current.scrollTop = editor.current.scrollHeight;
    }
  }, [code, detailId, selected?.status]);

  return <div className="demo-app">
    <header className={isHome ? 'demo-header' : 'demo-header with-results'}>
      {isHome ? <>
        <span className="brand">RWKV-<span>Vibe-Code</span></span>
        <div className="header-space"/>
        {hasResults && <Button type="button" variant="outline" onClick={showResults}>
          <LayoutGrid size={16} aria-hidden="true"/>查看方案
        </Button>}
      </> : <nav className="view-navigation" aria-label="页面导航">
        <Button ref={homeButton} type="button" variant="outline" onClick={() => setView({ page: 'home' })}>
          <House size={16} aria-hidden="true"/>返回首页
        </Button>
        {selected && <Button type="button" variant="outline" onClick={showResults}>
          <ArrowLeft size={16} aria-hidden="true"/>全部方案
        </Button>}
      </nav>}

      {selected ? <>
        <strong className="detail-title">方案 {detailIndex + 1}</strong>
        {selected.status === 'running' && <span className="detail-live"><i/>实时更新中</span>}
        <div className="header-space"/>
      </> : !isHome && <div className="header-input">
        <PromptInput compact value={prompt} onChange={setPrompt} onSubmit={submit} disabled={isGenerating || appUpdate.installing}
          ready={engineReady} count={count} onCountChange={setCount} hardware={hardware}/>
      </div>}

      {(!isHome || isGenerating) && <GenerationStatus results={results} loading={loading} batchStats={batchStats}
        isGenerating={isGenerating} now={now} stop={() => void stop()}/>}
      <AppUpdate updater={appUpdate}/>
    </header>

    {error && <div className="inline-error" role="alert">{error}</div>}

    {selected ? <main className="detail-layout">
      <section className="source-panel" aria-label="HTML 源码">
        <div className="detail-panel-heading">
          <Code2 size={15}/>HTML 代码编辑器
          {(selected.stats?.firstTokenMs != null || selected.firstRenderMs !== undefined) && <span className="detail-metrics">
            {[
              selected.stats?.firstTokenMs != null ? `首段输出 ${seconds(selected.stats.firstTokenMs)}` : null,
              selected.firstRenderMs !== undefined ? `首屏就绪 ${seconds(selected.firstRenderMs)}` : null,
            ].filter(Boolean).join(' · ')}
          </span>}
        </div>
        <textarea ref={editor} spellCheck={false} value={code} readOnly={['running', 'queued'].includes(selected.status)}
          aria-label="HTML 代码编辑器" onScroll={event => {
            const el = event.currentTarget;
            follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
          }} onChange={event => edit(selected.id, event.target.value)}/>
      </section>
      <section className="detail-preview">
        <div className="detail-panel-heading"><Eye size={15}/>实时预览</div>
        <div className="detail-preview-frame">
          <StreamingPreview key={selected.id} html={code} streaming={selected.status === 'running'}
            queued={selected.status === 'queued'} label="实时预览" progress={selected} now={now}
            onPainted={() => markPainted(selected.id)}/>
        </div>
      </section>
    </main> : isHome ? <main className="home-page">
      <div className="home-content">
        <h1 ref={homeHeading} tabIndex={-1}>你想创建什么？</h1>
        {desktop && <ModelPreparation engine={engine} stage={setupStage} progress={modelProgress}
          error={setupError} pausing={pausing} disabled={appUpdate.installing}
          onInstall={() => void runSetup(true)} onPause={() => void pauseDownload()}/>}
        <PromptInput inputRef={promptInput} value={prompt} onChange={setPrompt} onSubmit={submit}
          disabled={isGenerating || appUpdate.installing} ready={engineReady} count={count} onCountChange={setCount} hardware={hardware}/>
        <PromptExamples onPick={value => { setPrompt(value); promptInput.current?.focus(); }}/>
      </div>
    </main> : <main className="results-viewport">
      <ResultGrid results={results} onOpen={openDetail} onPainted={markPainted} now={now}/>
    </main>}
  </div>;
}
