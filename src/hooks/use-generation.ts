import { useCallback, useEffect, useRef, useState } from 'react';
import * as bridge from '../bridge';
import { buildPromptBatch } from '../lib/batch-prompts';
import { applyGenerationEvent } from '../lib/generation-events';
import { MIN_CONCURRENCY, type Concurrency, type EngineStatus, type GenerationEvent, type InferenceMetrics, type ModelProgress, type ModelSetupStage, type PageJob } from '../types';

interface ActiveBatch {
  ids: string[];
  submission?: Promise<void>;
  cancelled: boolean;
}

interface SetupAttempt {
  pauseRequested: boolean;
  paused: boolean;
}

const isActive = (job: PageJob) => job.status === 'queued' || job.status === 'running';
const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error);

export function useGeneration() {
  const [results, setResults] = useState<PageJob[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [setupStage, setSetupStage] = useState<ModelSetupStage>(bridge.desktop ? 'checking' : 'idle');
  const [setupError, setSetupError] = useState('');
  const [modelProgress, setModelProgress] = useState<ModelProgress | null>(null);
  const [pausing, setPausing] = useState(false);
  const [batchStats, setBatchStats] = useState<InferenceMetrics | null>(null);
  const setupAttempt = useRef<SetupAttempt | null>(null);
  const buffered = useRef(new Map<string, PageJob>());
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeBatch = useRef<ActiveBatch | null>(null);
  const mounted = useRef(true);

  const flush = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    const jobs = [...buffered.current.values()];
    if (!jobs.some(isActive)) activeBatch.current = null;
    if (mounted.current) setResults(jobs);
  }, []);

  const runSetup = useCallback(async (install: boolean) => {
    if (!bridge.desktop || setupAttempt.current || activeBatch.current) return;
    const attempt: SetupAttempt = { pauseRequested: false, paused: false };
    setupAttempt.current = attempt;
    setSetupStage('checking');
    setSetupError('');
    setModelProgress(null);
    try {
      const status = await bridge.engineStatus();
      if (mounted.current) setEngine(status);
      if (status.ready || !status.canPrepare) return;

      if (status.localSupported) {
        if (!install && !(status.modelInstalled && status.runtimeInstalled)) return;
        if (install) {
          if (mounted.current) setSetupStage('installing');
          await bridge.prepareModel(progress => {
            if (setupAttempt.current !== attempt || !mounted.current) return;
            if (progress.phase === 'paused') attempt.paused = true;
            setModelProgress(progress);
          });
          if (attempt.paused) return;
          attempt.pauseRequested = false;
        }
      }

      if (mounted.current) setSetupStage('starting');
      const ready = await bridge.startEngine(MIN_CONCURRENCY);
      if (!ready.ready) throw new Error(ready.error || '本地模型启动失败');
      if (mounted.current) {
        setEngine(ready);
        setError('');
      }
    } catch (error) {
      const latest = await bridge.engineStatus().catch(() => null);
      if (mounted.current) {
        if (latest) setEngine(latest);
        if (attempt.paused || attempt.pauseRequested) {
          setModelProgress(progress => ({
            downloadedBytes: progress?.downloadedBytes ?? 0,
            totalBytes: progress?.totalBytes ?? 0,
            speedBps: 0,
            source: progress?.source ?? '',
            message: '下载已暂停',
            phase: 'paused',
          }));
        } else {
          setSetupError(errorMessage(error));
        }
      }
    } finally {
      setupAttempt.current = null;
      if (mounted.current) {
        setSetupStage('idle');
        setPausing(false);
      }
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void runSetup(false);
    return () => {
      mounted.current = false;
      if (timer.current) {
        clearTimeout(timer.current);
        timer.current = null;
      }
    };
  }, [runSetup]);

  const pauseDownload = async () => {
    const attempt = setupAttempt.current;
    if (!attempt || attempt.pauseRequested || modelProgress?.phase !== 'downloading') return;
    attempt.pauseRequested = true;
    setPausing(true);
    try {
      await bridge.cancelModelPreparation();
    } catch (error) {
      if (setupAttempt.current === attempt && mounted.current) {
        attempt.pauseRequested = false;
        setPausing(false);
        setSetupError(errorMessage(error));
      }
    }
  };

  const receive = useCallback((event: GenerationEvent) => {
    const job = buffered.current.get(event.id);
    if (!job || !mounted.current) return;
    // A batch total may be delivered after its last page has finished.
    if (event.type === 'batchMetrics') {
      setBatchStats(event.stats);
      return;
    }
    if (!isActive(job)) return;

    const updated = applyGenerationEvent(job, event, Date.now());
    buffered.current.set(job.id, updated);
    if (!isActive(updated)) {
      flush();
    } else if (!timer.current) {
      timer.current = setTimeout(flush, 80);
    }
  }, [flush]);

  function finishBatch(batch: ActiveBatch, error?: string) {
    for (const id of batch.ids) {
      receive(error === undefined ? { type: 'cancelled', id } : { type: 'error', id, message: error });
    }
  }

  const generate = async (prompt: string, count: Concurrency, onQueued: () => void) => {
    if (activeBatch.current || setupAttempt.current) return;

    let prompts: string[];
    try {
      prompts = buildPromptBatch(prompt, count);
    } catch (error) {
      setError(errorMessage(error));
      return;
    }
    if (!bridge.desktop) {
      setError('请在桌面应用中生成网页');
      return;
    }
    if (!engine?.ready) {
      setError('请先安装并启动本地模型');
      return;
    }

    const createdAt = Date.now();
    const jobs: PageJob[] = prompts.map((prompt, index) => ({
      id: crypto.randomUUID(),
      title: `方案 ${index + 1}`,
      prompt,
      raw: '',
      html: '',
      status: 'queued',
      createdAt,
    }));
    const batch: ActiveBatch = { ids: jobs.map(job => job.id), cancelled: false };
    activeBatch.current = batch;
    buffered.current = new Map(jobs.map(job => [job.id, job]));
    setResults(jobs);
    setBatchStats(null);
    setError('');
    setLoading(true);
    onQueued();

    try {
      // An odd number of distinct requests uses one fewer slot than the even capacity.
      const lanes = prompts.length + prompts.length % 2;
      const ready = await bridge.startEngine(lanes);
      if (!ready.ready) throw new Error(ready.error || '本地模型尚未就绪');
      if (mounted.current) setEngine(ready);
      if (batch.cancelled) return;

      setLoading(false);
      batch.submission = bridge.generatePages(jobs.map(({ id, prompt }) => ({ id, prompt })), receive);
      await batch.submission;
    } catch (error) {
      if (activeBatch.current !== batch) return;

      if (!batch.submission && !batch.cancelled) {
        const latest = await bridge.engineStatus().catch(() => null);
        if (activeBatch.current !== batch) return;
        if (latest && mounted.current) setEngine(latest);
      }
      setLoading(false);
      const message = errorMessage(error);
      if (!batch.cancelled) setError(message);
      finishBatch(batch, batch.cancelled ? undefined : message);
    }
  };

  const stop = async () => {
    const batch = activeBatch.current;
    if (!batch || batch.cancelled) return;

    batch.cancelled = true;
    if (!batch.submission) {
      setLoading(false);
      finishBatch(batch);
      return;
    }

    // Wait for Rust to register the batch before sending its cancellation.
    // A lane may already have finished, so send every ID from this batch.
    try {
      await batch.submission;
      if (activeBatch.current === batch) await bridge.cancelBatch(batch.ids);
    } catch (error) {
      if (activeBatch.current === batch) {
        batch.cancelled = false;
        setError(errorMessage(error));
      }
    }
  };

  const markPainted = useCallback((id: string) => {
    const job = buffered.current.get(id);
    if (!job || job.firstRenderMs !== undefined || job.startedAt === undefined) return;

    buffered.current.set(id, { ...job, firstRenderMs: Math.max(0, Date.now() - job.createdAt) });
    flush();
  }, [flush]);

  const edit = (id: string, html: string) => {
    const job = buffered.current.get(id);
    if (!job || isActive(job)) return;

    buffered.current.set(id, { ...job, raw: html, html, incomplete: !/<\/html>/i.test(html) });
    flush();
  };

  return {
    results, loading, engine, setupStage, setupError, modelProgress, pausing, runSetup, pauseDownload, batchStats, error,
    generate, stop, edit, markPainted,
    isGenerating: results.some(isActive),
  };
}
