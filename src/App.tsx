import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowLeft, Code2, Eye } from 'lucide-react';
import PromptInput from './components/PromptInput';
import ModelPreparation from './components/ModelPreparation';
import PromptExamples from './components/PromptExamples';
import ResultGrid from './components/ResultGrid';
import StreamingPreview from './StreamingPreview';
import { useGeneration } from './hooks/use-generation';
import { extractPreview } from './preview';
import GenerationStatus from './components/GenerationStatus';
import type { Concurrency } from './types';
import { seconds, formatTps } from './lib/performance';

// The reference's HomePage -> ChatPage -> DetailPage, kept in one Tauri WebView.
// No account, sidebar, settings, history manager or decorative product controls.
export default function App() {
  const [prompt, setPrompt] = useState('');
  const [count, setCount] = useState<Concurrency>(4);
  const [detailId, setDetailId] = useState<string | null>(null);
  const { results, loading, preparing, modelProgress, pauseModel, retryModel, error, generate, stop, edit, markPainted, isGenerating } = useGeneration();
  const selected = results.find(job => job.id === detailId);
  const openDetail = useCallback((id: string) => setDetailId(id), []);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { if (!isGenerating) return; setNow(Date.now()); const timer = setInterval(() => setNow(Date.now()), 250); return () => clearInterval(timer); }, [isGenerating]);
  const code = selected?.html || extractPreview(selected?.raw || '');
  const editor = useRef<HTMLTextAreaElement>(null);
  const promptInput = useRef<HTMLTextAreaElement>(null);
  const follow = useRef(true);
  useEffect(() => {
    if (selected?.status === 'running' && follow.current && editor.current) editor.current.scrollTop = editor.current.scrollHeight;
  }, [code, selected?.status]);
  const submit = () => { follow.current = true; setDetailId(null); void generate(prompt, count); };
  const status = <GenerationStatus results={results} loading={loading} isGenerating={isGenerating} now={now} stop={() => void stop()}/>;
  return <div className="demo-app">
    <header className={`demo-header ${results.length ? 'with-results' : ''}`}>
      {selected ? <><button className="back-button" onClick={() => setDetailId(null)} aria-label="返回全部方案"><ArrowLeft size={18}/></button><strong className="detail-title">方案 {results.indexOf(selected) + 1}</strong>{selected.status === 'running' && <span className="detail-live"><i/>实时更新中</span>}<div className="header-space"/>{status}</> : <><span className="brand">RWKV <span>Vibe Code</span></span>{results.length > 0 && <div className="header-input"><PromptInput compact value={prompt} onChange={setPrompt} onSubmit={submit} disabled={isGenerating} count={count} onCountChange={setCount}/></div>}{status}</>}
    </header>
    {results.length > 0 && <ModelPreparation progress={modelProgress} onPause={pauseModel} onRetry={retryModel} compact/>}
    {error && <div className="inline-error" role="alert">{error}</div>}
    {selected ? <main className="detail-layout"><section className="source-panel" aria-label="HTML 源码"><div className="detail-panel-heading"><Code2 size={15}/>HTML 代码编辑器{selected.stats && <span className="detail-metrics">{formatTps(selected.stats.outputTokens > 1 ? selected.stats.tps : null)} TPS · {selected.stats.outputTokens.toLocaleString()} tokens{selected.stats.firstTokenMs != null ? ` · 首 token ${seconds(selected.stats.firstTokenMs)}` : ''}{selected.firstRenderMs !== undefined ? ` · 首屏就绪 ${seconds(selected.firstRenderMs)}` : ''}</span>}</div><textarea ref={editor} spellCheck={false} value={code} readOnly={['running', 'queued'].includes(selected.status)} aria-label="HTML 代码编辑器" onScroll={() => { const el = editor.current; if (el) follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60; }} onChange={event => edit(selected.id, event.target.value)}/></section><section className="detail-preview"><div className="detail-panel-heading"><Eye size={15}/>实时预览</div><div className="detail-preview-frame"><StreamingPreview key={selected.id} html={selected.html || extractPreview(selected.raw)} streaming={selected.status === 'running'} queued={selected.status === 'queued'} label="实时预览" progress={selected} now={now} onPainted={() => markPainted(selected.id)}/></div></section></main> : results.length ? <main className="results-viewport"><ResultGrid results={results} onOpen={openDetail} onPainted={markPainted} now={now}/></main> : <main className="home-page"><div className="home-content"><h1>你想创建什么？</h1><PromptInput inputRef={promptInput} value={prompt} onChange={setPrompt} onSubmit={submit} disabled={isGenerating} count={count} onCountChange={setCount}/><p className="batch-input-hint">多条需求用单独一行 <code>---</code> 分隔，最多 4 条；单条需求可选 1–4 路生成不同方案。</p><ModelPreparation progress={modelProgress} onPause={pauseModel} onRetry={retryModel}/><PromptExamples onPick={value => { setPrompt(value); promptInput.current?.focus(); }}/><p className="model-caption">RWKV7 · 1.5B · {preparing ? '正在预热本地模型…' : '本地 HTML 流式生成'}</p></div></main>}
  </div>;
}
