import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowLeft, Code2, Eye, House, LayoutGrid } from 'lucide-react';
import PromptInput from './components/PromptInput';
import ModelPreparation from './components/ModelPreparation';
import PromptExamples from './components/PromptExamples';
import ResultGrid from './components/ResultGrid';
import { Button } from './components/ui/button';
import StreamingPreview from './StreamingPreview';
import { useGeneration } from './hooks/use-generation';
import { extractPreview } from './preview';
import GenerationStatus from './components/GenerationStatus';
import { DEFAULT_CONCURRENCY, MAX_CONCURRENCY, MIN_CONCURRENCY, type ConcurrencyMode } from './types';
import { seconds, formatTps } from './lib/performance';

type View = { page: 'home' | 'results' } | { page: 'detail'; id: string };

export default function App() {
  const [prompt, setPrompt] = useState('');
  const [count, setCount] = useState<ConcurrencyMode>(DEFAULT_CONCURRENCY);
  const [view, setView] = useState<View>({ page: 'home' });
  const { results, loading, preparing, modelProgress, pauseModel, retryModel, error, generate, stop, edit, markPainted, isGenerating, hardware } = useGeneration();
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
  const submit = () => void generate(prompt, effectiveCount, showResults);

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
        <PromptInput compact value={prompt} onChange={setPrompt} onSubmit={submit} disabled={isGenerating}
          count={count} onCountChange={setCount} hardware={hardware}/>
      </div>}

      {(!isHome || isGenerating) && <GenerationStatus results={results} loading={loading}
        isGenerating={isGenerating} now={now} stop={() => void stop()}/>}
    </header>

    {!isHome && <ModelPreparation progress={modelProgress} onPause={pauseModel}
      onRetry={() => void retryModel(effectiveCount)} compact/>}
    {error && <div className="inline-error" role="alert">{error}</div>}

    {selected ? <main className="detail-layout">
      <section className="source-panel" aria-label="HTML 源码">
        <div className="detail-panel-heading">
          <Code2 size={15}/>HTML 代码编辑器
          {selected.stats && <span className="detail-metrics">
            {formatTps(selected.stats.outputTokens > 1 ? selected.stats.tps : null)} TPS · {selected.stats.outputTokens.toLocaleString()} tokens
            {selected.stats.firstTokenMs != null ? ` · 首 token ${seconds(selected.stats.firstTokenMs)}` : ''}
            {selected.firstRenderMs !== undefined ? ` · 首屏就绪 ${seconds(selected.firstRenderMs)}` : ''}
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
        <PromptInput inputRef={promptInput} value={prompt} onChange={setPrompt} onSubmit={submit}
          disabled={isGenerating} count={count} onCountChange={setCount} hardware={hardware}/>
        <p className="batch-input-hint">多条需求用单独一行 <code>---</code> 分隔，每批最多 {MAX_CONCURRENCY} 条，且不超过所选并发数。单条需求可选 {MIN_CONCURRENCY}–{MAX_CONCURRENCY} 路的偶数档位。</p>
        <ModelPreparation progress={modelProgress} onPause={pauseModel} onRetry={() => void retryModel(effectiveCount)}/>
        <PromptExamples onPick={value => { setPrompt(value); promptInput.current?.focus(); }}/>
        <p className="model-caption">RWKV7 · 1.5B · {preparing ? '正在预热本地模型…' : '本地 HTML 流式生成'}</p>
      </div>
    </main> : <main className="results-viewport">
      <ResultGrid results={results} onOpen={openDetail} onPainted={markPainted} now={now}/>
    </main>}
  </div>;
}
