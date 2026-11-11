import type { Concurrency } from '../types';
// One input, one batch. Multiline requirements stay intact; only a standalone
// separator starts another prompt. A single requirement can still yield variants.
export function splitPromptInput(value: string): string[] {
  return value.split(/^\s*---\s*$/m).map(prompt => prompt.trim()).filter(Boolean);
}
export function buildPromptBatch(value: string, count: Concurrency): string[] {
  const prompts = splitPromptInput(value);
  if (!prompts.length) throw new Error('请填写网页需求');
  if (prompts.length > 4) throw new Error('本机一次批量支持最多 4 条需求');
  if (prompts.some(prompt => prompt.length > 2000)) throw new Error('每条需求最多 2000 字');
  return prompts.length === 1 ? Array.from({ length: count }, () => prompts[0]) : prompts;
}
