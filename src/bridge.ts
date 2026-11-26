import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { EngineStatus, GenerationEvent, ModelProgress } from './types';

export const desktop = isTauri();
export const engineStatus = () => invoke<EngineStatus>('engine_status');
export const startEngine = (concurrency: number) => invoke<EngineStatus>('start_engine', { concurrency });
export const cancelModelPreparation = () => invoke<void>('cancel_model_preparation');
export const cancelBatch = (ids: string[]) => invoke<void>('cancel_generation', { ids });

export interface AppUpdateProgress {
  phase: 'downloading' | 'verifying' | 'installing';
  downloadedBytes: number;
  totalBytes: number | null;
}

export const appUpdateStatus = () => invoke<{ supported: boolean; version: string }>('app_update_status');
export const checkAppUpdate = () => invoke<{ version: string } | null>('check_app_update');

export function installAppUpdate(version: string, onProgress: (event: AppUpdateProgress) => void) {
  return invoke<void>('install_app_update', { version, onEvent: new Channel(onProgress) });
}

export function prepareModel(onEvent: (progress: ModelProgress) => void) {
  return invoke<void>('prepare_model', { onEvent: new Channel(onEvent) });
}

export function generatePages(
  requests: Array<{ id: string; prompt: string }>,
  onEvent: (event: GenerationEvent) => void,
) {
  return invoke<void>('generate_pages', { requests, onEvent: new Channel(onEvent) });
}
