import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { EngineStatus, GenerationEvent, ModelProgress } from './types';

export const desktop = isTauri();
export const engineStatus = () => invoke<EngineStatus>('engine_status');
export const startEngine = (concurrency: number) => invoke<EngineStatus>('start_engine', { concurrency });
export const cancelBatch = (ids: string[]) => invoke<void>('cancel_generation', { ids });

export function generatePages(
  requests: Array<{ id: string; prompt: string }>,
  onEvent: (event: GenerationEvent) => void,
) {
  return invoke<void>('generate_pages', { requests, onEvent: new Channel(onEvent) });
}

export function prepareModel(onEvent: (progress: ModelProgress) => void) {
  return invoke<void>('prepare_model', { onEvent: new Channel(onEvent) });
}

export const cancelModelPreparation = () => invoke<void>('cancel_model_preparation');
