import type { GenerationEvent, PageJob } from '../types';
export function applyGenerationEvent(job: PageJob, event: GenerationEvent, now: number): PageJob {
  switch (event.type) {
    case 'queued': return { ...job, status: 'queued' };
    case 'started': return { ...job, status: 'running', startedAt: job.startedAt ?? now };
    case 'metrics': return { ...job, stats: event.stats };
    case 'delta': return { ...job, raw: job.raw + event.text };
    case 'done': return { ...job, status: 'done', html: event.html, seconds: event.seconds, incomplete: event.incomplete, endedAt: now };
    case 'cancelled': return { ...job, status: 'cancelled', endedAt: now };
    case 'error': return { ...job, status: 'error', error: event.message, endedAt: now };
  }
}
