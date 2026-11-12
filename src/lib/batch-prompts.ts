import { CONCURRENCY_OPTIONS, MAX_CONCURRENCY, MIN_CONCURRENCY, type Concurrency } from '../types.ts';

export const MAX_PROMPT_LENGTH = 2000;
// One input, one batch. Multiline requirements stay intact; only a standalone
// separator starts another prompt. A single requirement can still yield variants.
export function splitPromptInput(value: string): string[] {
  return value.split(/^\s*---\s*$/m).map(prompt => prompt.trim()).filter(Boolean);
}
export function buildPromptBatch(value: string, count: Concurrency): string[] {
  if (!CONCURRENCY_OPTIONS.includes(count)) {
    throw new Error(`请选择 ${MIN_CONCURRENCY}–${MAX_CONCURRENCY} 路的偶数并发档位`);
  }
  const prompts = splitPromptInput(value);
  if (!prompts.length) throw new Error('请填写网页需求');
  if (prompts.length > MAX_CONCURRENCY) throw new Error(`每批最多 ${MAX_CONCURRENCY} 条需求`);
  if (prompts.length > count) throw new Error(`有 ${prompts.length} 条需求，当前为 ${count} 路，请提高并发数或分批提交`);
  if (prompts.some(prompt => [...prompt].length > MAX_PROMPT_LENGTH)) {
    throw new Error(`每条需求最多 ${MAX_PROMPT_LENGTH} 字`);
  }
  return prompts.length === 1 ? Array.from({ length: count }, () => prompts[0]) : prompts;
}
