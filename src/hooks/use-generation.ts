import { useCallback, useEffect, useRef, useState } from 'react';
import * as bridge from '../bridge';
import { buildPromptBatch } from '../lib/batch-prompts';
import { applyGenerationEvent } from '../lib/generation-events';
import type { Concurrency, GenerationEvent, ModelProgress, PageJob } from '../types';

interface ActiveBatch {
  ids: string[];
  submission?: Promise<void>;
  cancelled: boolean;
}

const isActive = (job: PageJob) => job.status === 'queued' || job.status === 'running';
const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error);

export function useGeneration() {
  const [results, setResults] = useState<PageJob[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [preparing, setPreparing] = useState(bridge.desktop);
  const [modelProgress, setModelProgress] = useState<ModelProgress | null>(null);
  const preparation = useRef<Promise<void> | null>(null);
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

  const prepareEngine = useCallback(() => {
    if (preparation.current) return preparation.current;

    setPreparing(true);
    let modelReportedFailure = false;
    const pending = (async () => {
      const status = await bridge.engineStatus();
      if (status.ready) return;

      await bridge.prepareModel(progress => {
        modelReportedFailure = progress.phase === 'error' || progress.phase === 'paused';
        if (mounted.current) setModelProgress(progress);
      });
      await bridge.startEngine(4);
    })().catch(error => {
      if (mounted.current && !modelReportedFailure) setError(errorMessage(error));
      throw error;
    }).finally(() => {
      preparation.current = null;
      if (mounted.current) setPreparing(false);
    });
    preparation.current = pending;
    return pending;
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (bridge.desktop) void prepareEngine().catch(() => {});
    return () => {
      mounted.current = false;
      if (timer.current) clearTimeout(timer.current);
    };
  }, [prepareEngine]);

  const receive = useCallback((event: GenerationEvent) => {
    const job = buffered.current.get(event.id);
    if (!job || !isActive(job)) return;

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

  const generate = async (prompt: string, count: Concurrency) => {
    if (activeBatch.current) return;

    let prompts: string[];
    try {
      prompts = buildPromptBatch(prompt, count);
    } catch (error) {
      setError(errorMessage(error));
      return;
    }
    if (!bridge.desktop) {
      setError('本地模型需要在桌面应用中运行：pnpm desktop:dev');
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
    setError('');
    setLoading(true);

    try {
      await prepareEngine();
      if (batch.cancelled) return;

      setLoading(false);
      batch.submission = bridge.generatePages(jobs.map(({ id, prompt }) => ({ id, prompt })), receive);
      await batch.submission;
    } catch (error) {
      if (activeBatch.current !== batch) return;

      setLoading(false);
      const message = errorMessage(error);
      if (batch.submission && !batch.cancelled) setError(message);
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

  const pauseModel = async () => {
    if (!activeBatch.current?.submission) void stop();
    try {
      await bridge.cancelModelPreparation();
    } catch (error) {
      setError(errorMessage(error));
    }
  };

  const retryModel = async () => {
    // The paused/error event may arrive before the original IPC call settles.
    await preparation.current?.catch(() => {});
    setError('');
    await prepareEngine().catch(() => {});
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
    results, loading, preparing, modelProgress, pauseModel, retryModel, error,
    generate, stop, edit, markPainted,
    isGenerating: results.some(isActive),
  };
}
