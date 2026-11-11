export type Concurrency = 1 | 2 | 3 | 4;
export type JobStatus = 'queued' | 'running' | 'done' | 'cancelled' | 'error';
export interface InferenceMetrics { outputTokens: number; promptTokens: number; generationMs: number; tps: number; elapsedMs: number; firstTokenMs: number | null }
export interface PageJob { id: string; prompt: string; title: string; createdAt: number; status: JobStatus; raw: string; html: string; error?: string; incomplete?: boolean; seconds?: number; stats?: InferenceMetrics; startedAt?: number; endedAt?: number; firstRenderMs?: number }
export interface EngineStatus { ready: boolean; concurrency: number; pending: number; model: string; error: string | null }
export type GenerationEvent =
  | { type: 'queued' | 'started' | 'cancelled'; id: string }
  | { type: 'metrics'; id: string; stats: InferenceMetrics }
  | { type: 'delta'; id: string; text: string }
  | { type: 'done'; id: string; html: string; incomplete: boolean; seconds: number }
  | { type: 'error'; id: string; message: string };
export interface ModelProgress {
  phase: 'checking' | 'downloading' | 'importing' | 'verifying' | 'ready' | 'paused' | 'error';
  downloadedBytes: number;
  totalBytes: number;
  speedBps: number;
  source: string;
  message: string;
}
